//! The coordinate type of the chart axes and how data values map onto them.
//!
//! plotters fixes the coordinate type of a chart at compile time
//! (`ChartContext<DB, Cartesian2d<X, Y>>`), while a chart value only knows its axis kinds at
//! run time. Every axis is therefore one [`AxisCoord`]: a run-time choice between plotters'
//! coordinate types, with `f64` values on every axis. Numbers are themselves, dates are days
//! since 1970-01-01, timestamps are microseconds since 1970-01-01 00:00:00, and bands are
//! positions in band units (band `i` spans `i..i + 1`). Each variant converts the `f64` to its
//! inner coordinate's value type and delegates mapping, key points and label formatting to it,
//! so the drawing code works on one `Cartesian2d<AxisCoord, AxisCoord>` whatever the axis
//! kinds, and a new axis kind is a new variant rather than a new type combination.

use crate::error::{Error, Result};
use crate::methods::check_scale_fits;
use crate::spec::{Axis, AxisKind, AxisRange, Chart, Column, Scale, Series, SeriesKind, bin};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeDelta, Utc};
use plotters::coord::combinators::{IntoLogRange, LogCoord};
use plotters::coord::ranged1d::{KeyPointHint, NoDefaultFormatting, Ranged, ValueFormatter};
use plotters::coord::types::{
    IntoMonthly, IntoYearly, Monthly, RangedCoordf64, RangedCoordi64, Yearly,
};
use plotters::data::float::FloatPrettyPrinter;
use plotters::prelude::{RangedDate, RangedDateTime};
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::Arc;

/// One axis of a chart, with `f64` values whatever its kind.
#[derive(Clone)]
pub(crate) enum AxisCoord {
    /// `RangedCoordf64`.
    Linear(RangedCoordf64),
    /// `RangedDate`; values are days since 1970-01-01.
    Date(DateCoord),
    /// `RangedDateTime`; values are microseconds since 1970-01-01 00:00:00. `f64` holds every
    /// microsecond exactly up to about the year 2255.
    Timestamp(TimeCoord),
    /// A segmented axis; values are band positions.
    Band(BandCoord),
    /// `LogCoord<f64>`.
    Log(LogAxis),
}

impl AxisCoord {
    /// The position of value `i` of `column`: the value itself, or the centre of its band.
    /// `None` for a column of another kind, for a date or timestamp chrono cannot represent,
    /// and for a value a log axis cannot place (zero or negative).
    pub(crate) fn at(&self, column: &Column, i: usize) -> Option<f64> {
        match (self, column) {
            (AxisCoord::Linear(_), Column::Numeric(v)) => Some(v[i]),
            (AxisCoord::Log(_), Column::Numeric(v)) => (v[i] > 0.0).then_some(v[i]),
            (AxisCoord::Date(_), Column::Date(v)) => date(v[i]).map(|_| f64::from(v[i])),
            (AxisCoord::Timestamp(_), Column::Timestamp(v)) => timestamp(v[i]).map(|_| v[i] as f64),
            (AxisCoord::Band(b), _) => Some(b.index(column, i)? as f64 + 0.5),
            _ => None,
        }
    }

    /// The position of a number (a bar end or a baseline), on numeric axes. On a log axis a
    /// number that is not positive is placed at the low end of the axis, so bars from a
    /// baseline of 0 start at the axis edge.
    pub(crate) fn number(&self, value: f64) -> Option<f64> {
        match self {
            AxisCoord::Linear(_) => Some(value),
            AxisCoord::Log(_) if value > 0.0 => Some(value),
            AxisCoord::Log(log) => Some(log.lo.min(log.hi)),
            _ => None,
        }
    }

    /// The position of a value that belongs on a numeric axis (an error bar's extremes, a
    /// candle's prices, a boxplot's quartiles), as `at` places a column value: `None` on other
    /// axes and for a value a log axis cannot place.
    pub(crate) fn value(&self, value: f64) -> Option<f64> {
        match self {
            AxisCoord::Linear(_) => Some(value),
            AxisCoord::Log(_) => (value > 0.0).then_some(value),
            _ => None,
        }
    }

    /// The low and high edge of the band of value `i` of `column`, where
    /// `SegmentValue::Exact(v)` and `SegmentValue::Exact(v + 1)` would map. `None` on axes
    /// without bands and for values outside the axis, which plotters' segmented coordinates
    /// drop.
    pub(crate) fn band(&self, column: &Column, i: usize) -> Option<(f64, f64)> {
        match self {
            AxisCoord::Band(b) => b.band(column, i),
            _ => None,
        }
    }
}

/// What a label formatter formats: the number, category name or date-time at a key point.
pub(crate) enum LabelValue {
    Number(f64),
    Text(String),
    /// A date is its midnight.
    Time(NaiveDateTime),
}

impl AxisCoord {
    /// The kind of the axis, for label formatters and their errors.
    pub(crate) fn kind(&self) -> AxisKind {
        match self {
            AxisCoord::Linear(_) | AxisCoord::Log(_) => AxisKind::Numeric,
            AxisCoord::Date(_) => AxisKind::Date,
            AxisCoord::Timestamp(_) => AxisKind::Timestamp,
            AxisCoord::Band(b) => match b.labels {
                BandLabels::Integer(_) => AxisKind::Integer,
                BandLabels::Date(..) => AxisKind::Date,
                BandLabels::Category(..) => AxisKind::Category,
                BandLabels::Step { .. } => AxisKind::Numeric,
            },
        }
    }

    /// The value a label at `value` stands for; `None` outside a band axis.
    pub(crate) fn label_value(&self, value: f64) -> Option<LabelValue> {
        match self {
            AxisCoord::Linear(_) | AxisCoord::Log(_) => Some(LabelValue::Number(value)),
            AxisCoord::Date(_) => day_of(value).map(|d| LabelValue::Time(d.into())),
            AxisCoord::Timestamp(_) => instant_of(value).map(LabelValue::Time),
            AxisCoord::Band(b) => b.label_value(value),
        }
    }
}

/// A date axis value (days since the epoch) as the date, `None` beyond chrono's range.
fn day_of(value: f64) -> Option<NaiveDate> {
    date(value.round() as i32)
}

fn days_of(d: NaiveDate) -> f64 {
    (d - NaiveDate::default()).num_days() as f64
}

/// A timestamp axis value (microseconds since the epoch) as the timestamp.
fn instant_of(value: f64) -> Option<NaiveDateTime> {
    timestamp(value.round() as i64)
}

fn micros_of(t: NaiveDateTime) -> f64 {
    t.and_utc().timestamp_micros() as f64
}

impl Ranged for AxisCoord {
    type FormatOption = NoDefaultFormatting;
    type ValueType = f64;

    fn map(&self, value: &f64, limit: (i32, i32)) -> i32 {
        match self {
            AxisCoord::Linear(c) => c.map(value, limit),
            // Positions come from `at`, which only yields representable dates and times.
            AxisCoord::Date(c) => day_of(*value).map_or(limit.0, |d| c.inner.map(&d, limit)),
            AxisCoord::Timestamp(c) => instant_of(*value).map_or(limit.0, |t| c.map(&t, limit)),
            AxisCoord::Band(c) => c.map(value, limit),
            AxisCoord::Log(c) => c.coord.map(value, limit),
        }
    }

    fn key_points<Hint: KeyPointHint>(&self, hint: Hint) -> Vec<f64> {
        match self {
            AxisCoord::Linear(c) => c.key_points(hint),
            AxisCoord::Date(c) => c.key_points(hint).into_iter().map(days_of).collect(),
            AxisCoord::Timestamp(c) => c.key_points(hint).into_iter().map(micros_of).collect(),
            AxisCoord::Band(c) => c.key_points(hint),
            AxisCoord::Log(c) => c.coord.key_points(hint),
        }
    }

    fn range(&self) -> Range<f64> {
        match self {
            AxisCoord::Linear(c) => c.range(),
            AxisCoord::Date(c) => {
                let r = c.inner.range();
                days_of(r.start)..days_of(r.end)
            }
            AxisCoord::Timestamp(c) => {
                let r = c.range();
                micros_of(r.start)..micros_of(r.end)
            }
            AxisCoord::Band(c) => c.range(),
            AxisCoord::Log(c) => c.lo..c.hi,
        }
    }
}

impl ValueFormatter<f64> for AxisCoord {
    fn format_ext(&self, value: &f64) -> String {
        match self {
            AxisCoord::Linear(c) => c.format_ext(value),
            AxisCoord::Date(c) => day_of(*value).map_or_else(String::new, |d| c.format(&d)),
            AxisCoord::Timestamp(c) => {
                instant_of(*value).map_or_else(String::new, |t| c.format_ext(&t))
            }
            AxisCoord::Band(c) => c.format_ext(value),
            // `LogCoord` formats with `{:?}` (`1000000.0`); plotters' float printer with
            // scientific notation keeps a decade's label short (`1e6`, `100`, `0.001`).
            AxisCoord::Log(_) => FloatPrettyPrinter {
                allow_scientific: true,
                min_decimal: 0,
                max_decimal: 5,
            }
            .print(*value),
        }
    }
}

/// The key points of a time axis: the coordinate's own, or those of plotters' `Monthly` or
/// `Yearly` over the same range (`x_monthly()`, `x_yearly()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimeKeys {
    Default,
    Monthly,
    Yearly,
}

impl TimeKeys {
    fn of(scale: Scale) -> TimeKeys {
        match scale {
            Scale::Monthly => TimeKeys::Monthly,
            Scale::Yearly => TimeKeys::Yearly,
            Scale::Linear | Scale::Log { .. } => TimeKeys::Default,
        }
    }

    /// The monthly or yearly key points of the dates `lo..hi` (`lo <= hi`); `None` for the
    /// default keys, which are the coordinate's own. plotters' `TimeValue` bound is not
    /// public, so dates and timestamps have a function each.
    fn date_key_points<H: KeyPointHint>(
        self,
        lo: NaiveDate,
        hi: NaiveDate,
        hint: H,
    ) -> Option<Vec<NaiveDate>> {
        match self {
            TimeKeys::Default => None,
            TimeKeys::Monthly => Some((lo..hi).monthly().key_points(hint)),
            TimeKeys::Yearly => Some((lo..hi).yearly().key_points(hint)),
        }
    }

    /// The monthly or yearly key points of the timestamps `lo..hi` (`lo <= hi`). plotters
    /// has no ranged coordinate for a `NaiveDateTime` range behind `Monthly`, so they are
    /// computed on the same instants in UTC, which is how duckers reads timestamps.
    fn time_key_points<H: KeyPointHint>(
        self,
        lo: NaiveDateTime,
        hi: NaiveDateTime,
        hint: H,
    ) -> Option<Vec<NaiveDateTime>> {
        let range = lo.and_utc()..hi.and_utc();
        let points = match self {
            TimeKeys::Default => return None,
            TimeKeys::Monthly => range.monthly().key_points(hint),
            TimeKeys::Yearly => range.yearly().key_points(hint),
        };
        Some(points.into_iter().map(|t| t.naive_utc()).collect())
    }

    /// The label of a date key point: plotters' `Monthly`/`Yearly` text (`2024-3`), or `None`
    /// for the default keys.
    fn format_date(self, value: &NaiveDate) -> Option<String> {
        match self {
            TimeKeys::Default => None,
            TimeKeys::Monthly => Some(<Monthly<NaiveDate> as ValueFormatter<_>>::format(value)),
            TimeKeys::Yearly => Some(<Yearly<NaiveDate> as ValueFormatter<_>>::format(value)),
        }
    }

    /// The label of a timestamp key point.
    fn format_time(self, value: &NaiveDateTime) -> Option<String> {
        let value = value.and_utc();
        match self {
            TimeKeys::Default => None,
            TimeKeys::Monthly => Some(<Monthly<DateTime<Utc>> as ValueFormatter<_>>::format(
                &value,
            )),
            TimeKeys::Yearly => Some(<Yearly<DateTime<Utc>> as ValueFormatter<_>>::format(&value)),
        }
    }
}

/// A `RangedDate` axis, with monthly or yearly key points when asked for.
#[derive(Clone)]
pub(crate) struct DateCoord {
    inner: Oriented<RangedDate<NaiveDate>>,
    keys: TimeKeys,
}

impl DateCoord {
    fn key_points<H: KeyPointHint>(&self, hint: H) -> Vec<NaiveDate> {
        let r = self.inner.inner.range();
        if self.keys == TimeKeys::Default {
            return self.inner.key_points(hint);
        }
        self.keys
            .date_key_points(r.start, r.end, hint)
            .unwrap_or_default()
    }

    fn format(&self, value: &NaiveDate) -> String {
        self.keys
            .format_date(value)
            .unwrap_or_else(|| self.inner.format_ext(value))
    }
}

/// A `LogCoord<f64>` with the bounds and base it was built from, so it can be cloned
/// (`LogCoord` is not `Clone`).
pub(crate) struct LogAxis {
    coord: LogCoord<f64>,
    lo: f64,
    hi: f64,
    base: f64,
}

impl LogAxis {
    /// `(lo..hi).log_scale().base(base)`; `lo > hi` reverses the axis. Both bounds must be
    /// positive.
    fn new(lo: f64, hi: f64, base: f64) -> LogAxis {
        LogAxis {
            coord: (lo..hi).log_scale().base(base).into(),
            lo,
            hi,
            base,
        }
    }
}

impl Clone for LogAxis {
    fn clone(&self) -> Self {
        LogAxis::new(self.lo, self.hi, self.base)
    }
}

/// A `RangedDateTime` axis whose labels are formatted to fit the span shown.
///
/// plotters formats `NaiveDateTime` labels with `{:?}` (`2024-01-01T08:00:00`), which is too
/// wide for the key points it picks on a default-sized chart, so the labels overlap and none
/// can be read. This is the one place where duckers replaces a plotters label default.
///
/// With monthly or yearly key points, the labels are plotters' `Monthly`/`Yearly` ones.
#[derive(Clone)]
pub(crate) struct TimeCoord {
    inner: Oriented<RangedDateTime<NaiveDateTime>>,
    format: &'static str,
    keys: TimeKeys,
}

impl TimeCoord {
    fn new(lo: NaiveDateTime, hi: NaiveDateTime, reversed: bool, keys: TimeKeys) -> TimeCoord {
        let span = hi - lo;
        let format = if span >= TimeDelta::days(10) {
            "%Y-%m-%d"
        } else if span >= TimeDelta::days(1) {
            "%m-%d %H:%M"
        } else if span >= TimeDelta::minutes(10) {
            "%H:%M"
        } else if span >= TimeDelta::seconds(10) {
            "%H:%M:%S"
        } else {
            "%H:%M:%S%.3f"
        };
        TimeCoord {
            inner: Oriented {
                inner: RangedDateTime::from(lo..hi),
                reversed,
            },
            format,
            keys,
        }
    }
}

impl Ranged for TimeCoord {
    type FormatOption = NoDefaultFormatting;
    type ValueType = NaiveDateTime;

    fn map(&self, value: &NaiveDateTime, limit: (i32, i32)) -> i32 {
        self.inner.map(value, limit)
    }

    fn key_points<Hint: KeyPointHint>(&self, hint: Hint) -> Vec<NaiveDateTime> {
        let r = self.inner.inner.range();
        if self.keys == TimeKeys::Default {
            return self.inner.key_points(hint);
        }
        self.keys
            .time_key_points(r.start, r.end, hint)
            .unwrap_or_default()
    }

    fn range(&self) -> Range<NaiveDateTime> {
        self.inner.range()
    }
}

impl ValueFormatter<NaiveDateTime> for TimeCoord {
    fn format_ext(&self, value: &NaiveDateTime) -> String {
        self.keys
            .format_time(value)
            .unwrap_or_else(|| value.format(self.format).to_string())
    }
}

pub(crate) fn date(days: i32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(1970, 1, 1)?.checked_add_signed(TimeDelta::try_days(days.into())?)
}

pub(crate) fn timestamp(micros: i64) -> Option<NaiveDateTime> {
    DateTime::from_timestamp_micros(micros).map(|t| t.naive_utc())
}

fn out_of_range(what: &str) -> Error {
    Error::invalid(format!("{what} is outside the range chrono can represent"))
}

/// Min and max of the values, or `None` when there are none.
fn extent<T: PartialOrd + Copy>(values: impl IntoIterator<Item = T>) -> Option<(T, T)> {
    values.into_iter().fold(None, |acc, v| match acc {
        None => Some((v, v)),
        Some((lo, hi)) => Some((if v < lo { v } else { lo }, if v > hi { v } else { hi })),
    })
}

/// A default range: the data extent, `0..1` without data, and one unit either side of a
/// single value, since plotters cannot map a range whose start equals its end.
fn fit<T>(extent: Option<(T, T)>, zero: T, one: T) -> (T, T)
where
    T: PartialOrd + Copy + std::ops::Add<Output = T> + std::ops::Sub<Output = T>,
{
    match extent {
        None => (zero, zero + one),
        Some((lo, hi)) if lo == hi => (lo - one, hi + one),
        Some(range) => range,
    }
}

/// The histogram bars of a series: for each distinct bucket, in order of first appearance, the
/// row of its first appearance and the sum of its values, as `Histogram::data` sums. The
/// buckets are the column on the series' bucket axis, the values the column on the other.
pub(crate) fn bucket_sums(series: &Series) -> Vec<(usize, f64)> {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum Bucket<'a> {
        Int(i64),
        Text(&'a str),
    }
    let Some(axis) = series.bucket_axis() else {
        return Vec::new();
    };
    let (buckets, values) = (series.column(axis), series.column(axis.other()));
    let Column::Numeric(values) = values else {
        return Vec::new();
    };
    let step = series.kind.histogram().and_then(|o| o.step);
    let key = |i: usize| match buckets {
        Column::Integer(v) => Some(Bucket::Int(v[i])),
        Column::Date(v) => Some(Bucket::Int(v[i].into())),
        Column::Timestamp(v) => Some(Bucket::Int(v[i])),
        Column::Category(v) => Some(Bucket::Text(&v[i])),
        Column::Numeric(v) => Some(Bucket::Int(bin(v[i], step?)?)),
    };
    let mut index: BTreeMap<Bucket, usize> = BTreeMap::new();
    let mut out: Vec<(usize, f64)> = Vec::new();
    for (i, y) in values.iter().enumerate() {
        let Some(k) = key(i) else { continue };
        match index.get(&k) {
            Some(&j) => out[j].1 += y,
            None => {
                index.insert(k, out.len());
                out.push((i, *y));
            }
        }
    }
    out
}

/// Checks that all series agree on the axis kinds, which the methods already enforce for
/// values they built.
pub(crate) fn check_kinds(series: &[&Series]) -> Result<()> {
    if let Some(first) = series.first() {
        for s in &series[1..] {
            for axis in [Axis::X, Axis::Y] {
                if s.kind_on(axis) != first.kind_on(axis) {
                    return Err(Error::invalid(format!(
                        "cannot draw {} and {} {} values on the same chart",
                        first.kind_on(axis),
                        s.kind_on(axis),
                        axis.name()
                    )));
                }
            }
        }
    }
    Ok(())
}

/// The numbers that set the default extent of a numeric `axis`: the values of every series on
/// it, for a histogram whose buckets are on the other axis its baseline and bar ends, and the
/// other numbers a series has on the axis (see [`Series::extra_numbers`]).
fn numbers_on(series: &[&Series], axis: Axis) -> Vec<f64> {
    let mut values: Vec<f64> = Vec::new();
    for s in series {
        match s.kind.histogram() {
            Some(options) if s.bucket_axis() == Some(axis.other()) => {
                values.push(options.baseline);
                values.extend(bucket_sums(s).into_iter().map(|(_, sum)| sum));
            }
            _ => {
                if let Column::Numeric(v) = s.column(axis) {
                    values.extend(v);
                }
                values.extend(s.extra_numbers(axis));
            }
        }
    }
    values.retain(|v| v.is_finite());
    values
}

/// Resolves `axis` of the chart: its kind from the series (or the range bounds), its range
/// from `x_range`/`y_range` or the data, and bands where histograms put buckets on it.
pub(crate) fn coord(chart: &Chart, series: &[&Series], axis: Axis) -> Result<AxisCoord> {
    let name = axis.name();
    let range = chart.axis(axis).range;
    let kind = series
        .first()
        .map(|s| s.kind_on(axis))
        .or(range.map(AxisRange::kind))
        .unwrap_or(AxisKind::Numeric);
    let has_bars = series.iter().any(|s| s.bucket_axis() == Some(axis));
    let columns = || series.iter().map(|s| s.column(axis));
    let mismatch = || {
        Error::invalid(format!(
            "{name}_range does not fit the chart's {kind} {name} axis"
        ))
    };
    let beyond = || out_of_range(&format!("the {name} range"));
    let scale = chart.axis(axis).scale;
    check_scale_fits(&format!("{name}_log_scale"), axis, scale, kind)?;

    Ok(match kind {
        AxisKind::Numeric if has_bars => {
            if let Scale::Log { .. } = scale {
                return Err(Error::invalid(format!(
                    "{name}_log_scale does not apply to the stepped bucket {name} axis: plotters' \
                     log_scale (LogCoord) has no segmented form"
                )));
            }
            let step = series
                .iter()
                .filter(|s| s.bucket_axis() == Some(axis))
                .find_map(|s| s.kind.histogram()?.step)
                .ok_or_else(|| {
                    Error::invalid(format!(
                        "histogram buckets on the {name} axis are numeric and need .step(s)"
                    ))
                })?;
            let binned = |v: f64| bin(v, step).ok_or_else(|| out_of_range_bins(v, step, name));
            let (first, last, reversed) = match range {
                Some(AxisRange::Numeric(lo, hi)) => {
                    let (a, b) = (lo.min(hi), lo.max(hi));
                    // The bands that cover a..b: b itself starts a band only past a multiple.
                    let first = binned(a)?;
                    let end = binned(b)?;
                    let last = if bin(b, step) == Some(end)
                        && (end as f64 * step - b).abs() <= 1e-9 * b.abs().max(step)
                    {
                        end - 1
                    } else {
                        end
                    };
                    (first, last.max(first), lo > hi)
                }
                Some(_) => return Err(mismatch()),
                None => {
                    let bins: Vec<i64> = columns()
                        .flat_map(|c| match c {
                            Column::Numeric(v) => v.iter().filter_map(|v| bin(*v, step)).collect(),
                            _ => Vec::new(),
                        })
                        .collect();
                    let (first, last) = extent(bins).unwrap_or((0, 0));
                    (first, last, false)
                }
            };
            let n = usize::try_from(last - first + 1)
                .ok()
                .filter(|n| *n <= MAX_BANDS)
                .ok_or_else(|| too_many_bands(last - first + 1, name))?;
            AxisCoord::Band(BandCoord {
                n,
                reversed,
                labels: BandLabels::Step { first, step },
            })
        }
        AxisKind::Numeric => match scale {
            Scale::Linear | Scale::Monthly | Scale::Yearly => {
                let (lo, hi) = match range {
                    Some(AxisRange::Numeric(lo, hi)) => (lo, hi),
                    Some(_) => return Err(mismatch()),
                    None => fit(extent(numbers_on(series, axis)), 0.0, 1.0),
                };
                AxisCoord::Linear((lo..hi).into())
            }
            Scale::Log { base } => {
                let (lo, hi) = match range {
                    Some(AxisRange::Numeric(lo, hi)) => (lo, hi),
                    Some(_) => return Err(mismatch()),
                    // A log axis widens a single value by a factor of the base either side,
                    // and shows one decade without data.
                    None => match extent(numbers_on(series, axis)) {
                        None => (1.0, base),
                        Some((lo, hi)) if lo == hi && lo > 0.0 => (lo / base, hi * base),
                        Some(range) => range,
                    },
                };
                if lo <= 0.0 || hi <= 0.0 {
                    let (from, fix) = if range.is_some() {
                        (format!("{name}_range"), format!("{name}_range(lo, hi)"))
                    } else if series.iter().any(|s| {
                        s.kind.histogram().is_some() && s.bucket_axis() == Some(axis.other())
                    }) {
                        (
                            "the data (a histogram includes its baseline, 0 by default)".into(),
                            format!("a positive baseline(v) on the histogram, or {name}_range"),
                        )
                    } else if axis == Axis::Y
                        && series.iter().any(|s| matches!(s.kind, SeriesKind::Area(_)))
                    {
                        (
                            "the data (an area series includes its baseline, 0 by default)".into(),
                            format!("a positive baseline(v) on the area series, or {name}_range"),
                        )
                    } else {
                        ("the data".into(), format!("{name}_range(lo, hi)"))
                    };
                    return Err(Error::invalid(format!(
                        "{name}_log_scale needs positive {name} bounds, since plotters' LogCoord \
                         maps ln(v); {from} gives {lo} to {hi}: set {fix} with positive bounds"
                    )));
                }
                AxisCoord::Log(LogAxis::new(lo, hi, base))
            }
        },
        AxisKind::Timestamp => {
            let (lo, hi) = match range {
                Some(AxisRange::Timestamp(lo, hi)) => (lo, hi),
                Some(_) => return Err(mismatch()),
                None => fit(
                    extent(
                        columns()
                            .flat_map(|c| match c {
                                Column::Timestamp(v) => v.as_slice(),
                                _ => &[],
                            })
                            .copied(),
                    ),
                    0,
                    1_000_000,
                ),
            };
            let reversed = lo > hi;
            let (lo, hi) = (lo.min(hi), lo.max(hi));
            let lo = timestamp(lo).ok_or_else(beyond)?;
            let hi = timestamp(hi).ok_or_else(beyond)?;
            AxisCoord::Timestamp(TimeCoord::new(lo, hi, reversed, TimeKeys::of(scale)))
        }
        AxisKind::Date | AxisKind::Integer => {
            let (lo, hi): (i64, i64) = match (kind, range) {
                (AxisKind::Date, Some(AxisRange::Date(lo, hi))) => (lo.into(), hi.into()),
                (AxisKind::Integer, Some(AxisRange::Numeric(lo, hi))) => (lo as i64, hi as i64),
                (_, Some(_)) => return Err(mismatch()),
                (_, None) => fit(
                    extent(columns().flat_map(|c| -> Vec<i64> {
                        match c {
                            Column::Date(v) => v.iter().map(|d| i64::from(*d)).collect(),
                            Column::Integer(v) => v.clone(),
                            _ => Vec::new(),
                        }
                    })),
                    0,
                    1,
                ),
            };
            let reversed = lo > hi;
            let (lo, hi) = (lo.min(hi), lo.max(hi));
            if kind == AxisKind::Integer || has_bars {
                let n = usize::try_from(hi - lo + 1)
                    .ok()
                    .filter(|n| *n <= MAX_BANDS)
                    .ok_or_else(|| too_many_bands(hi - lo + 1, name))?;
                let labels = if kind == AxisKind::Integer {
                    BandLabels::Integer(lo)
                } else {
                    let first = i32::try_from(lo).ok().and_then(date).ok_or_else(beyond)?;
                    BandLabels::Date(first, TimeKeys::of(scale))
                };
                AxisCoord::Band(BandCoord {
                    n,
                    reversed,
                    labels,
                })
            } else {
                let day = |d: i64| i32::try_from(d).ok().and_then(date).ok_or_else(beyond);
                AxisCoord::Date(DateCoord {
                    inner: Oriented {
                        inner: RangedDate::from(day(lo)?..day(hi)?),
                        reversed,
                    },
                    keys: TimeKeys::of(scale),
                })
            }
        }
        AxisKind::Category => {
            if range.is_some() {
                return Err(mismatch());
            }
            let mut names: Vec<String> = Vec::new();
            let mut index: HashMap<String, usize> = HashMap::new();
            for column in columns() {
                if let Column::Category(values) = column {
                    for v in values {
                        if !index.contains_key(v) {
                            index.insert(v.clone(), names.len());
                            names.push(v.clone());
                        }
                    }
                }
            }
            if names.len() > MAX_BANDS {
                return Err(too_many_bands(names.len() as i64, name));
            }
            AxisCoord::Band(BandCoord {
                n: names.len().max(1),
                reversed: false,
                labels: BandLabels::Category(Arc::new(names), Arc::new(index)),
            })
        }
    })
}

/// More bands than this is a range mistake (a chart is at most 8192 px wide).
const MAX_BANDS: usize = 100_000;

fn out_of_range_bins(value: f64, step: f64, axis: &str) -> Error {
    Error::invalid(format!(
        "the {axis} value {value} is too far from 0 for bands of step {step}"
    ))
}

fn too_many_bands(n: i64, axis: &str) -> Error {
    Error::invalid(format!(
        "a segmented axis with {n} bands is too large (at most {MAX_BANDS}); \
         check {axis}_range or the bucket values"
    ))
}

/// A coordinate that maps in the direction of its range, for coordinate types that do not
/// support `start > end` themselves.
#[derive(Clone)]
pub(crate) struct Oriented<R> {
    pub(crate) inner: R,
    pub(crate) reversed: bool,
}

impl<R: Ranged> Ranged for Oriented<R> {
    type FormatOption = NoDefaultFormatting;
    type ValueType = R::ValueType;

    fn map(&self, value: &Self::ValueType, limit: (i32, i32)) -> i32 {
        if self.reversed {
            self.inner.map(value, (limit.1, limit.0))
        } else {
            self.inner.map(value, limit)
        }
    }

    fn key_points<Hint: KeyPointHint>(&self, hint: Hint) -> Vec<Self::ValueType> {
        self.inner.key_points(hint)
    }

    fn range(&self) -> Range<Self::ValueType> {
        let range = self.inner.range();
        if self.reversed {
            range.end..range.start
        } else {
            range
        }
    }
}

impl<R> ValueFormatter<R::ValueType> for Oriented<R>
where
    R: Ranged + ValueFormatter<R::ValueType>,
{
    fn format_ext(&self, value: &R::ValueType) -> String {
        self.inner.format_ext(value)
    }
}

/// A segmented axis: `n` equal bands, band `i` spanning `i..i + 1`, labelled at its centre.
///
/// It behaves as plotters' `SegmentedCoord` over an integer range, a `RangedDate` or a
/// `RangedSlice` (key points and labels at band centres, the same key-point choice as the
/// underlying coordinate), but is not tied to a borrowed slice, maps a single band correctly,
/// and supports a reversed range.
#[derive(Clone)]
pub(crate) struct BandCoord {
    n: usize,
    reversed: bool,
    labels: BandLabels,
}

#[derive(Clone)]
enum BandLabels {
    /// Band `i` is the integer `lo + i`.
    Integer(i64),
    /// Band `i` is the day `first + i`; the key points are every day's or, with monthly or
    /// yearly keys, the days that start a month or year.
    Date(NaiveDate, TimeKeys),
    /// Band `i` is `names[i]`.
    Category(Arc<Vec<String>>, Arc<HashMap<String, usize>>),
    /// Band `i` is the bin `first + i` of numeric buckets binned by `step`: the values from
    /// `(first + i) * step` up to the next bin, labelled with its start.
    Step { first: i64, step: f64 },
}

impl BandCoord {
    fn index(&self, column: &Column, i: usize) -> Option<i64> {
        match (&self.labels, column) {
            (BandLabels::Integer(lo), Column::Integer(v)) => v[i].checked_sub(*lo),
            (BandLabels::Date(first, _), Column::Date(v)) => {
                Some((date(v[i])? - *first).num_days())
            }
            (BandLabels::Step { first, step }, Column::Numeric(v)) => {
                bin(v[i], *step)?.checked_sub(*first)
            }
            (BandLabels::Category(_, index), Column::Category(v)) => {
                index.get(&v[i]).map(|j| *j as i64)
            }
            _ => None,
        }
    }

    /// The bucket of the band at `value`.
    fn label_value(&self, value: f64) -> Option<LabelValue> {
        let i = value.floor();
        if i < 0.0 || i >= self.n as f64 {
            return None;
        }
        let i = i as usize;
        Some(match &self.labels {
            BandLabels::Integer(lo) => LabelValue::Number((lo + i as i64) as f64),
            BandLabels::Date(first, _) => {
                LabelValue::Time((*first + TimeDelta::days(i as i64)).into())
            }
            BandLabels::Category(names, _) => LabelValue::Text(names.get(i)?.clone()),
            BandLabels::Step { first, step } => LabelValue::Number(step_start(*first, i, *step)),
        })
    }

    fn band(&self, column: &Column, i: usize) -> Option<(f64, f64)> {
        let index = self.index(column, i)?;
        (0..self.n as i64)
            .contains(&index)
            .then_some((index as f64, index as f64 + 1.0))
    }

    fn key_indices<H: KeyPointHint>(&self, hint: H) -> Vec<usize> {
        let max = hint.max_num_points();
        if max == 0 {
            return Vec::new();
        }
        let last = self.n as i64 - 1;
        if let BandLabels::Date(first, keys) = &self.labels
            && *keys != TimeKeys::Default
        {
            // Monthly and yearly keys pick the bands of the days that start a month or year.
            let end = *first + TimeDelta::days(last);
            return keys
                .date_key_points(*first, end, hint)
                .unwrap_or_default()
                .into_iter()
                .map(|d| (d - *first).num_days() as usize)
                .collect();
        }
        if self.n == 1 {
            return vec![0];
        }
        match &self.labels {
            BandLabels::Integer(lo) => RangedCoordi64::from(*lo..*lo + last)
                .key_points(max)
                .into_iter()
                .map(|v| (v - lo) as usize)
                .collect(),
            BandLabels::Step { first, .. } => RangedCoordi64::from(*first..*first + last)
                .key_points(max)
                .into_iter()
                .map(|v| (v - first) as usize)
                .collect(),
            BandLabels::Date(first, _) => {
                let end = *first + TimeDelta::days(last);
                RangedDate::from(*first..end)
                    .key_points(max)
                    .into_iter()
                    .map(|d| (d - *first).num_days() as usize)
                    .collect()
            }
            BandLabels::Category(..) => {
                // RangedSlice's choice.
                let step = (last as f64 / max as f64 + 1.0) as usize;
                (0..self.n).step_by(step).collect()
            }
        }
    }
}

impl Ranged for BandCoord {
    type FormatOption = NoDefaultFormatting;
    type ValueType = f64;

    fn map(&self, value: &f64, limit: (i32, i32)) -> i32 {
        let (start, end) = if self.reversed {
            (limit.1, limit.0)
        } else {
            limit
        };
        let length = end - start;
        let fraction = value / self.n as f64;
        // The rounding of plotters' numeric coordinates.
        if length > 0 {
            start + (f64::from(length) * fraction + 1e-3).floor() as i32
        } else {
            start + (f64::from(length) * fraction - 1e-3).ceil() as i32
        }
    }

    fn key_points<Hint: KeyPointHint>(&self, hint: Hint) -> Vec<f64> {
        self.key_indices(hint)
            .into_iter()
            .filter(|i| *i < self.n)
            .map(|i| i as f64 + 0.5)
            .collect()
    }

    fn range(&self) -> Range<f64> {
        if self.reversed {
            self.n as f64..0.0
        } else {
            0.0..self.n as f64
        }
    }
}

impl ValueFormatter<f64> for BandCoord {
    fn format_ext(&self, value: &f64) -> String {
        let i = value.floor();
        if i < 0.0 || i >= self.n as f64 {
            return String::new();
        }
        let i = i as usize;
        match &self.labels {
            // `{:?}`, the formatting of plotters' integer and date coordinates.
            BandLabels::Integer(lo) => format!("{:?}", lo + i as i64),
            BandLabels::Date(first, keys) => {
                let day = *first + TimeDelta::days(i as i64);
                keys.format_date(&day).unwrap_or_else(|| format!("{day:?}"))
            }
            BandLabels::Category(names, _) => names.get(i).cloned().unwrap_or_default(),
            // The formatting of plotters' `RangedCoordf64`, which `Linspace` delegates to.
            BandLabels::Step { first, step } => {
                <RangedCoordf64 as ValueFormatter<f64>>::format(&step_start(*first, i, *step))
            }
        }
    }
}

/// The value at the start of band `i` of a stepped axis starting at bin `first`.
fn step_start(first: i64, i: usize, step: f64) -> f64 {
    (first + i as i64) as f64 * step
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accumulate::SeriesAggregate;
    use crate::methods::RangeValue;

    fn chart(series: Vec<Series>) -> Chart {
        Chart::new().draw_series_list(series).unwrap()
    }

    fn line(x: Vec<f64>, y: Vec<f64>) -> Series {
        Series::new(
            SeriesAggregate::LineSeries,
            Column::Numeric(x),
            Column::Numeric(y),
        )
        .unwrap()
    }

    fn hist(x: Column, y: Vec<f64>) -> Series {
        Series::new(SeriesAggregate::Histogram, x, Column::Numeric(y)).unwrap()
    }

    fn x_coord(chart: &Chart, series: &[&Series]) -> Result<AxisCoord> {
        coord(chart, series, Axis::X)
    }

    fn ranges(chart: &Chart) -> (Range<f64>, Range<f64>) {
        let series: Vec<&Series> = chart.series().collect();
        let x = x_coord(chart, &series).unwrap();
        assert!(matches!(x, AxisCoord::Linear(_) | AxisCoord::Band(_)));
        (x.range(), coord(chart, &series, Axis::Y).unwrap().range())
    }

    fn band(coord: AxisCoord) -> BandCoord {
        match coord {
            AxisCoord::Band(b) => b,
            _ => panic!("not a band axis"),
        }
    }

    #[test]
    fn data_extent_over_all_series_without_padding() {
        let c = chart(vec![
            line(vec![1.0, 3.0], vec![5.0, -2.0]),
            line(vec![-4.0, 2.0], vec![0.5, 7.0]),
        ]);
        assert_eq!(ranges(&c), (-4.0..3.0, -2.0..7.0));
    }

    #[test]
    fn empty_chart_is_unit_range() {
        assert_eq!(ranges(&Chart::new()), (0.0..1.0, 0.0..1.0));
        let empty = chart(vec![line(vec![], vec![])]);
        assert_eq!(ranges(&empty), (0.0..1.0, 0.0..1.0));
    }

    #[test]
    fn single_value_is_widened() {
        let c = chart(vec![line(vec![2.0], vec![5.0])]);
        assert_eq!(ranges(&c), (1.0..3.0, 4.0..6.0));
    }

    #[test]
    fn histograms_include_the_baseline_and_sum_buckets() {
        let c = chart(vec![hist(
            Column::Category(vec!["a".into(), "b".into(), "a".into()]),
            vec![2.0, 3.0, 4.0],
        )]);
        assert_eq!(ranges(&c), (0.0..2.0, 0.0..6.0));
        let neg = chart(vec![hist(Column::Integer(vec![3, 5]), vec![-2.0, -1.0])]);
        // Integer bands from 3 to 5.
        assert_eq!(ranges(&neg), (0.0..3.0, -2.0..0.0));
    }

    #[test]
    fn explicit_and_reversed_ranges() {
        let c = chart(vec![line(vec![1.0, 3.0], vec![5.0, -2.0])])
            .x_range(RangeValue::Number(10.0), RangeValue::Number(0.0))
            .unwrap()
            .y_range(RangeValue::Number(-10.0), RangeValue::Number(30.0))
            .unwrap();
        assert_eq!(ranges(&c), (10.0..0.0, -10.0..30.0));
    }

    #[test]
    fn category_order_is_first_appearance() {
        let c = chart(vec![
            hist(
                Column::Category(vec!["b".into(), "a".into()]),
                vec![1.0, 1.0],
            ),
            hist(
                Column::Category(vec!["c".into(), "b".into()]),
                vec![1.0, 1.0],
            ),
        ]);
        let series: Vec<&Series> = c.series().collect();
        let b = band(x_coord(&c, &series).unwrap());
        let labels: Vec<String> = b.key_points(11).iter().map(|v| b.format_ext(v)).collect();
        assert_eq!(labels, ["b", "a", "c"]);
    }

    #[test]
    fn band_mapping() {
        let b = BandCoord {
            n: 1,
            reversed: false,
            labels: BandLabels::Integer(7),
        };
        assert_eq!(b.map(&0.5, (100, 200)), 150);
        assert_eq!(b.key_points(11), [0.5]);
        assert_eq!(b.format_ext(&0.5), "7");
        let r = BandCoord {
            reversed: true,
            n: 4,
            ..b
        };
        assert_eq!(r.map(&0.0, (0, 400)), 400);
        assert_eq!(r.map(&4.0, (0, 400)), 0);
    }

    #[test]
    fn date_bands_and_axes() {
        let dates = hist(Column::Date(vec![19723, 19725]), vec![1.0, 1.0]);
        let c = chart(vec![dates]);
        let series: Vec<&Series> = c.series().collect();
        let b = band(x_coord(&c, &series).unwrap());
        assert_eq!(b.n, 3);
        assert_eq!(b.format_ext(&0.5), "2024-01-01");
        assert_eq!(
            b.band(&series[0].x, 1),
            Some((2.0, 3.0)),
            "2024-01-03 is the third band"
        );

        let line = Series::new(
            SeriesAggregate::LineSeries,
            Column::Date(vec![19723, 19725]),
            Column::Numeric(vec![1.0, 2.0]),
        )
        .unwrap();
        let c = chart(vec![line]);
        let series: Vec<&Series> = c.series().collect();
        let x = x_coord(&c, &series).unwrap();
        assert!(matches!(x, AxisCoord::Date(_)));
        assert_eq!(x.range(), 19723.0..19725.0);
        assert_eq!(x.at(&series[0].x, 1), Some(19725.0));
        assert_eq!(x.format_ext(&19724.0), "2024-01-02");
    }

    #[test]
    fn timestamps_are_microseconds() {
        let start = 1_704_067_200_000_000i64;
        let line = Series::new(
            SeriesAggregate::LineSeries,
            Column::Timestamp(vec![start, start + 3_600_000_000]),
            Column::Numeric(vec![1.0, 2.0]),
        )
        .unwrap();
        let c = chart(vec![line]);
        let series: Vec<&Series> = c.series().collect();
        let x = x_coord(&c, &series).unwrap();
        assert!(matches!(x, AxisCoord::Timestamp(_)));
        assert_eq!(x.map(&(start as f64), (0, 100)), 0);
        assert_eq!(x.map(&((start + 3_600_000_000) as f64), (0, 100)), 100);
        assert_eq!(x.format_ext(&((start + 1_800_000_000) as f64)), "00:30");
        for key in x.key_points(5) {
            assert!(key >= start as f64 && key <= (start + 3_600_000_000) as f64);
        }
    }

    #[test]
    fn horizontal_histograms_band_the_y_axis() {
        let h = Series::new(
            SeriesAggregate::HistogramHorizontal,
            Column::Numeric(vec![2.0, 3.0, 4.0]),
            Column::Category(vec!["a".into(), "b".into(), "a".into()]),
        )
        .unwrap();
        let c = chart(vec![h]);
        let series: Vec<&Series> = c.series().collect();
        assert_eq!(bucket_sums(series[0]), [(0, 6.0), (1, 3.0)]);
        let x = x_coord(&c, &series).unwrap();
        assert!(matches!(x, AxisCoord::Linear(_)));
        assert_eq!(x.range(), 0.0..6.0, "the baseline and the summed bars");
        let y = band(coord(&c, &series, Axis::Y).unwrap());
        assert_eq!(y.range(), 0.0..2.0);
        assert_eq!(y.format_ext(&1.5), "b");
    }

    #[test]
    fn log_axes() {
        let c = chart(vec![line(vec![1.0, 1000.0], vec![0.5, 2.0])])
            .x_log_scale(None)
            .unwrap();
        let series: Vec<&Series> = c.series().collect();
        let x = x_coord(&c, &series).unwrap();
        assert!(matches!(x, AxisCoord::Log(_)));
        assert_eq!(x.range(), 1.0..1000.0);
        // Linear in ln(v): 10 is a third of the way.
        assert_eq!(x.map(&10.0, (0, 300)), 100);
        let labels: Vec<String> = x.key_points(4).iter().map(|v| x.format_ext(v)).collect();
        assert_eq!(labels, ["1", "10", "100", "1000"]);
        assert_eq!(x.format_ext(&1e6), "1e6");
        assert_eq!(x.format_ext(&0.001), "0.001");
        assert_eq!(x.at(&Column::Numeric(vec![0.0, -1.0, 5.0]), 0), None);
        assert_eq!(x.at(&Column::Numeric(vec![0.0, -1.0, 5.0]), 2), Some(5.0));
        // Bars from a baseline below the axis start at its low end.
        assert_eq!(x.number(0.0), Some(1.0));
        let clone = x.clone();
        assert_eq!(clone.map(&100.0, (0, 300)), 200);

        // One value is widened by the base either side; no data is one decade.
        let one = chart(vec![line(vec![4.0], vec![1.0])])
            .x_log_scale(Some(2.0))
            .unwrap();
        let series: Vec<&Series> = one.series().collect();
        assert_eq!(x_coord(&one, &series).unwrap().range(), 2.0..8.0);
        let empty = Chart::new().y_log_scale(None).unwrap();
        assert_eq!(coord(&empty, &[], Axis::Y).unwrap().range(), 1.0..10.0);
    }

    #[test]
    fn log_axes_need_positive_bounds() {
        let err = |c: &Chart| {
            let series: Vec<&Series> = c.series().collect();
            coord(c, &series, Axis::Y)
                .err()
                .unwrap()
                .message()
                .to_string()
        };
        let data = chart(vec![line(vec![1.0, 2.0], vec![0.0, 5.0])])
            .y_log_scale(None)
            .unwrap();
        assert_eq!(
            err(&data),
            "y_log_scale needs positive y bounds, since plotters' LogCoord maps ln(v); the data \
             gives 0 to 5: set y_range(lo, hi) with positive bounds"
        );
        let bars = chart(vec![hist(Column::Integer(vec![1, 2]), vec![10.0, 100.0])])
            .y_log_scale(None)
            .unwrap();
        assert_eq!(
            err(&bars),
            "y_log_scale needs positive y bounds, since plotters' LogCoord maps ln(v); the data \
             (a histogram includes its baseline, 0 by default) gives 0 to 100: set a positive \
             baseline(v) on the histogram, or y_range with positive bounds"
        );
        let explicit = data
            .clone()
            .y_range(RangeValue::Number(-1.0), RangeValue::Number(10.0))
            .unwrap();
        assert!(
            err(&explicit).contains("y_range gives -1 to 10"),
            "{}",
            err(&explicit)
        );
        let fixed = data
            .y_range(RangeValue::Number(0.1), RangeValue::Number(10.0))
            .unwrap();
        let series: Vec<&Series> = fixed.series().collect();
        assert!(coord(&fixed, &series, Axis::Y).is_ok());
        let baseline = chart(vec![
            hist(Column::Integer(vec![1, 2]), vec![10.0, 100.0])
                .baseline(1.0)
                .unwrap(),
        ])
        .y_log_scale(None)
        .unwrap();
        let series: Vec<&Series> = baseline.series().collect();
        assert_eq!(
            coord(&baseline, &series, Axis::Y).unwrap().range(),
            1.0..100.0
        );
    }

    #[test]
    fn bins() {
        assert_eq!(bin(0.3, 0.1), Some(3));
        assert_eq!(bin(0.29, 0.1), Some(2));
        assert_eq!(bin(-0.5, 1.0), Some(-1));
        assert_eq!(bin(-1.0, 1.0), Some(-1));
        assert_eq!(bin(12.0, 5.0), Some(2));
        assert_eq!(bin(1e300, 1e-10), None);
    }

    fn stepped(x: Vec<f64>, step: f64) -> Series {
        let n = x.len();
        hist(Column::Numeric(x), vec![1.0; n]).step(step).unwrap()
    }

    #[test]
    fn stepped_histograms_band_by_bins() {
        let c = chart(vec![stepped(vec![12.0, 3.5, 14.9, 29.0], 5.0)]);
        let series: Vec<&Series> = c.series().collect();
        let x = x_coord(&c, &series).unwrap();
        assert_eq!(x.kind(), AxisKind::Numeric);
        let b = band(x.clone());
        // Bins 0 (3.5) to 5 (29).
        assert_eq!(b.n, 6);
        let labels: Vec<String> = b.key_points(11).iter().map(|v| b.format_ext(v)).collect();
        assert_eq!(labels, ["0.0", "5.0", "10.0", "15.0", "20.0", "25.0"]);
        assert!(matches!(x.label_value(2.5), Some(LabelValue::Number(v)) if v == 10.0));
        // 12 and 14.9 share bin 2, summed; the band of a row is its bin's.
        assert_eq!(bucket_sums(series[0]), [(0, 2.0), (1, 1.0), (3, 1.0)]);
        assert_eq!(x.band(&series[0].x, 1), Some((0.0, 1.0)));
        assert_eq!(x.band(&series[0].x, 0), Some((2.0, 3.0)));
        // A line on the same axis goes to the centres of its values' bins.
        assert_eq!(x.at(&Column::Numeric(vec![26.0]), 0), Some(5.5));
        let y = coord(&c, &series, Axis::Y).unwrap();
        assert_eq!(y.range(), 0.0..2.0);

        // An explicit range covers lo..hi: a hi on a bin start ends the bands before it.
        let ranged = c
            .clone()
            .x_range(RangeValue::Number(-10.0), RangeValue::Number(40.0))
            .unwrap();
        let series: Vec<&Series> = ranged.series().collect();
        let b = band(x_coord(&ranged, &series).unwrap());
        assert_eq!(b.n, 10);
        assert_eq!(b.format_ext(&0.5), "-10.0");
        let reversed = c
            .x_range(RangeValue::Number(41.0), RangeValue::Number(0.0))
            .unwrap();
        let series: Vec<&Series> = reversed.series().collect();
        let b = band(x_coord(&reversed, &series).unwrap());
        assert_eq!((b.n, b.reversed), (9, true));

        // Fractional steps label their bin starts without float noise.
        let c = chart(vec![stepped(vec![0.3, 0.05], 0.1)]);
        let series: Vec<&Series> = c.series().collect();
        let b = band(x_coord(&c, &series).unwrap());
        let labels: Vec<String> = b.key_points(11).iter().map(|v| b.format_ext(v)).collect();
        assert_eq!(labels, ["0.0", "0.1", "0.2", "0.3"]);
    }

    #[test]
    fn monthly_and_yearly_key_points() {
        let days = |from: i32, n: i32| {
            Series::new(
                SeriesAggregate::LineSeries,
                Column::Date((from..from + n).collect()),
                Column::Numeric(vec![1.0; n as usize]),
            )
            .unwrap()
        };
        // 2024-01-01 to 2024-12-31.
        let c = chart(vec![days(19723, 366)]).x_monthly().unwrap();
        let series: Vec<&Series> = c.series().collect();
        let x = x_coord(&c, &series).unwrap();
        let points = x.key_points(plotters::coord::ranged1d::BoldPoints(12));
        let labels: Vec<String> = points.iter().map(|v| x.format_ext(v)).collect();
        assert_eq!(labels[..3], ["2024-1", "2024-2", "2024-3"]);
        assert_eq!(labels.len(), 12);
        // Key points are month starts: 2024-02-01 is day 19754.
        assert_eq!(points[1], 19754.0);
        // Labels at key points format with strftime through label_value, as ever.
        assert!(matches!(x.label_value(19754.0), Some(LabelValue::Time(_))));

        let c = chart(vec![days(16436, 3653)]).x_yearly().unwrap();
        let series: Vec<&Series> = c.series().collect();
        let x = x_coord(&c, &series).unwrap();
        let labels: Vec<String> = x
            .key_points(plotters::coord::ranged1d::BoldPoints(12))
            .iter()
            .map(|v| x.format_ext(v))
            .collect();
        assert_eq!(
            labels,
            (2015..=2024).map(|y| format!("{y}-1")).collect::<Vec<_>>()
        );

        // Timestamps: 2024-01-01 00:00 to 2024-04-01 00:00, monthly.
        let start = 1_704_067_200_000_000i64;
        let ts = Series::new(
            SeriesAggregate::LineSeries,
            Column::Timestamp(vec![start, start + 91 * 86_400_000_000]),
            Column::Numeric(vec![1.0, 2.0]),
        )
        .unwrap();
        let c = chart(vec![ts]).x_monthly().unwrap();
        let series: Vec<&Series> = c.series().collect();
        let x = x_coord(&c, &series).unwrap();
        let points = x.key_points(plotters::coord::ranged1d::BoldPoints(10));
        let labels: Vec<String> = points.iter().map(|v| x.format_ext(v)).collect();
        assert_eq!(labels, ["2024-1", "2024-2", "2024-3", "2024-4"]);
        assert_eq!(points[1], (start + 31 * 86_400_000_000) as f64);

        // Date bands put the key points on the bands of month starts.
        let bars = hist(Column::Date((19723..19823).collect()), vec![1.0; 100]);
        let c = chart(vec![bars]).x_monthly().unwrap();
        let series: Vec<&Series> = c.series().collect();
        let x = x_coord(&c, &series).unwrap();
        let points = x.key_points(plotters::coord::ranged1d::BoldPoints(10));
        assert_eq!(points, [0.5, 31.5, 60.5, 91.5]);
        assert_eq!(x.format_ext(&31.5), "2024-2");
    }

    #[test]
    fn extents_include_extra_numbers() {
        let area = Series::new(
            SeriesAggregate::AreaSeries,
            Column::Numeric(vec![1.0, 2.0]),
            Column::Numeric(vec![5.0, 7.0]),
        )
        .unwrap();
        assert_eq!(ranges(&chart(vec![area.clone()])), (1.0..2.0, 0.0..7.0));
        let raised = area.baseline(6.0).unwrap();
        assert_eq!(ranges(&chart(vec![raised])).1, 5.0..7.0);

        let err = |c: &Chart| {
            let series: Vec<&Series> = c.series().collect();
            coord(c, &series, Axis::Y)
                .err()
                .unwrap()
                .message()
                .to_string()
        };
        let area = Series::new(
            SeriesAggregate::AreaSeries,
            Column::Numeric(vec![1.0]),
            Column::Numeric(vec![5.0]),
        )
        .unwrap();
        let log = chart(vec![area]).y_log_scale(None).unwrap();
        assert!(
            err(&log).contains("an area series includes its baseline"),
            "{}",
            err(&log)
        );
    }

    #[test]
    fn integer_band_key_points_use_integer_steps() {
        let b = BandCoord {
            n: 101,
            reversed: false,
            labels: BandLabels::Integer(0),
        };
        let labels: Vec<String> = b.key_points(11).iter().map(|v| b.format_ext(v)).collect();
        assert_eq!(
            labels,
            [
                "0", "10", "20", "30", "40", "50", "60", "70", "80", "90", "100"
            ]
        );
    }
}
