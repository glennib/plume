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
use crate::spec::{AxisKind, AxisRange, Chart, Column, Series, SeriesKind};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeDelta};
use plotters::coord::ranged1d::{KeyPointHint, NoDefaultFormatting, Ranged, ValueFormatter};
use plotters::coord::types::{RangedCoordf64, RangedCoordi64};
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
    Date(Oriented<RangedDate<NaiveDate>>),
    /// `RangedDateTime`; values are microseconds since 1970-01-01 00:00:00. `f64` holds every
    /// microsecond exactly up to about the year 2255.
    Timestamp(TimeCoord),
    /// A segmented axis; values are band positions.
    Band(BandCoord),
}

impl AxisCoord {
    /// The position of value `i` of `column`: the value itself, or the centre of its band.
    /// `None` for a column of another kind and for a date or timestamp chrono cannot represent.
    pub(crate) fn at(&self, column: &Column, i: usize) -> Option<f64> {
        match (self, column) {
            (AxisCoord::Linear(_), Column::Numeric(v)) => Some(v[i]),
            (AxisCoord::Date(_), Column::Date(v)) => date(v[i]).map(|_| f64::from(v[i])),
            (AxisCoord::Timestamp(_), Column::Timestamp(v)) => timestamp(v[i]).map(|_| v[i] as f64),
            (AxisCoord::Band(b), _) => Some(b.index(column, i)? as f64 + 0.5),
            _ => None,
        }
    }

    /// The position of a number (a y value, a bar height or a baseline), on numeric axes.
    pub(crate) fn number(&self, value: f64) -> Option<f64> {
        match self {
            AxisCoord::Linear(_) => Some(value),
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
            AxisCoord::Date(c) => day_of(*value).map_or(limit.0, |d| c.map(&d, limit)),
            AxisCoord::Timestamp(c) => instant_of(*value).map_or(limit.0, |t| c.map(&t, limit)),
            AxisCoord::Band(c) => c.map(value, limit),
        }
    }

    fn key_points<Hint: KeyPointHint>(&self, hint: Hint) -> Vec<f64> {
        match self {
            AxisCoord::Linear(c) => c.key_points(hint),
            AxisCoord::Date(c) => c.key_points(hint).into_iter().map(days_of).collect(),
            AxisCoord::Timestamp(c) => c.key_points(hint).into_iter().map(micros_of).collect(),
            AxisCoord::Band(c) => c.key_points(hint),
        }
    }

    fn range(&self) -> Range<f64> {
        match self {
            AxisCoord::Linear(c) => c.range(),
            AxisCoord::Date(c) => {
                let r = c.range();
                days_of(r.start)..days_of(r.end)
            }
            AxisCoord::Timestamp(c) => {
                let r = c.range();
                micros_of(r.start)..micros_of(r.end)
            }
            AxisCoord::Band(c) => c.range(),
        }
    }
}

impl ValueFormatter<f64> for AxisCoord {
    fn format_ext(&self, value: &f64) -> String {
        match self {
            AxisCoord::Linear(c) => c.format_ext(value),
            AxisCoord::Date(c) => day_of(*value).map_or_else(String::new, |d| c.format_ext(&d)),
            AxisCoord::Timestamp(c) => {
                instant_of(*value).map_or_else(String::new, |t| c.format_ext(&t))
            }
            AxisCoord::Band(c) => c.format_ext(value),
        }
    }
}

/// A `RangedDateTime` axis whose labels are formatted to fit the span shown.
///
/// plotters formats `NaiveDateTime` labels with `{:?}` (`2024-01-01T08:00:00`), which is too
/// wide for the key points it picks on a default-sized chart, so the labels overlap and none
/// can be read. This is the one place where duckers replaces a plotters label default.
#[derive(Clone)]
pub(crate) struct TimeCoord {
    inner: Oriented<RangedDateTime<NaiveDateTime>>,
    format: &'static str,
}

impl TimeCoord {
    fn new(lo: NaiveDateTime, hi: NaiveDateTime, reversed: bool) -> TimeCoord {
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
        self.inner.key_points(hint)
    }

    fn range(&self) -> Range<NaiveDateTime> {
        self.inner.range()
    }
}

impl ValueFormatter<NaiveDateTime> for TimeCoord {
    fn format_ext(&self, value: &NaiveDateTime) -> String {
        value.format(self.format).to_string()
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
/// row of its first appearance and the sum of its values, as `Histogram::data` sums.
pub(crate) fn bucket_sums(series: &Series) -> Vec<(usize, f64)> {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum Bucket<'a> {
        Int(i64),
        Text(&'a str),
    }
    let key = |i: usize| match &series.x {
        Column::Integer(v) => Some(Bucket::Int(v[i])),
        Column::Date(v) => Some(Bucket::Int(v[i].into())),
        Column::Timestamp(v) => Some(Bucket::Int(v[i])),
        Column::Category(v) => Some(Bucket::Text(&v[i])),
        Column::Numeric(_) => None,
    };
    let mut index: BTreeMap<Bucket, usize> = BTreeMap::new();
    let mut out: Vec<(usize, f64)> = Vec::new();
    for (i, y) in series.y.iter().enumerate() {
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
            if s.x_kind() != first.x_kind() {
                return Err(Error::invalid(format!(
                    "cannot draw {} and {} x values on the same chart",
                    first.x_kind(),
                    s.x_kind()
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn x_coord(chart: &Chart, series: &[&Series]) -> Result<AxisCoord> {
    let range = chart.x_axis.range;
    let kind = series
        .first()
        .map(|s| s.x_kind())
        .or(range.map(|r| match r {
            AxisRange::Numeric(..) => AxisKind::Numeric,
            AxisRange::Date(..) => AxisKind::Date,
            AxisRange::Timestamp(..) => AxisKind::Timestamp,
        }))
        .unwrap_or(AxisKind::Numeric);
    let has_bars = series
        .iter()
        .any(|s| matches!(s.kind, SeriesKind::Histogram(_)));
    let columns = || series.iter().map(|s| &s.x);
    let mismatch = || Error::invalid(format!("x_range does not fit the chart's {kind} x axis"));

    Ok(match kind {
        AxisKind::Numeric => {
            let (lo, hi) = match range {
                Some(AxisRange::Numeric(lo, hi)) => (lo, hi),
                Some(_) => return Err(mismatch()),
                None => fit(
                    extent(
                        columns()
                            .flat_map(|c| match c {
                                Column::Numeric(v) => v.as_slice(),
                                _ => &[],
                            })
                            .copied(),
                    ),
                    0.0,
                    1.0,
                ),
            };
            AxisCoord::Linear((lo..hi).into())
        }
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
            let lo = timestamp(lo).ok_or_else(|| out_of_range("the x range"))?;
            let hi = timestamp(hi).ok_or_else(|| out_of_range("the x range"))?;
            AxisCoord::Timestamp(TimeCoord::new(lo, hi, reversed))
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
                    .ok_or_else(|| too_many_bands(hi - lo + 1))?;
                let labels = if kind == AxisKind::Integer {
                    BandLabels::Integer(lo)
                } else {
                    let first = i32::try_from(lo)
                        .ok()
                        .and_then(date)
                        .ok_or_else(|| out_of_range("the x range"))?;
                    BandLabels::Date(first)
                };
                AxisCoord::Band(BandCoord {
                    n,
                    reversed,
                    labels,
                })
            } else {
                let day = |d: i64| {
                    i32::try_from(d)
                        .ok()
                        .and_then(date)
                        .ok_or_else(|| out_of_range("the x range"))
                };
                AxisCoord::Date(Oriented {
                    inner: RangedDate::from(day(lo)?..day(hi)?),
                    reversed,
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
                return Err(too_many_bands(names.len() as i64));
            }
            AxisCoord::Band(BandCoord {
                n: names.len().max(1),
                reversed: false,
                labels: BandLabels::Category(Arc::new(names), Arc::new(index)),
            })
        }
    })
}

pub(crate) fn y_coord(chart: &Chart, series: &[&Series]) -> Result<AxisCoord> {
    let (lo, hi) = match chart.y_axis.range {
        Some(AxisRange::Numeric(lo, hi)) => (lo, hi),
        Some(_) => {
            return Err(Error::invalid(
                "y_range does not fit the chart's numeric y axis",
            ));
        }
        None => {
            let mut values: Vec<f64> = Vec::new();
            for s in series {
                match &s.kind {
                    SeriesKind::Histogram(options) => {
                        values.push(options.baseline);
                        values.extend(bucket_sums(s).into_iter().map(|(_, sum)| sum));
                    }
                    _ => values.extend(&s.y),
                }
            }
            fit(
                extent(values.into_iter().filter(|v| v.is_finite())),
                0.0,
                1.0,
            )
        }
    };
    Ok(AxisCoord::Linear((lo..hi).into()))
}

/// More bands than this is a range mistake (a chart is at most 8192 px wide).
const MAX_BANDS: usize = 100_000;

fn too_many_bands(n: i64) -> Error {
    Error::invalid(format!(
        "a segmented axis with {n} bands is too large (at most {MAX_BANDS}); \
         check x_range or the bucket values"
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
    /// Band `i` is the day `first + i`.
    Date(NaiveDate),
    /// Band `i` is `names[i]`.
    Category(Arc<Vec<String>>, Arc<HashMap<String, usize>>),
}

impl BandCoord {
    fn index(&self, column: &Column, i: usize) -> Option<i64> {
        match (&self.labels, column) {
            (BandLabels::Integer(lo), Column::Integer(v)) => v[i].checked_sub(*lo),
            (BandLabels::Date(first), Column::Date(v)) => Some((date(v[i])? - *first).num_days()),
            (BandLabels::Category(_, index), Column::Category(v)) => {
                index.get(&v[i]).map(|j| *j as i64)
            }
            _ => None,
        }
    }

    fn band(&self, column: &Column, i: usize) -> Option<(f64, f64)> {
        let index = self.index(column, i)?;
        (0..self.n as i64)
            .contains(&index)
            .then_some((index as f64, index as f64 + 1.0))
    }

    fn key_indices(&self, max: usize) -> Vec<usize> {
        if max == 0 {
            return Vec::new();
        }
        if self.n == 1 {
            return vec![0];
        }
        let last = self.n as i64 - 1;
        match &self.labels {
            BandLabels::Integer(lo) => RangedCoordi64::from(*lo..*lo + last)
                .key_points(max)
                .into_iter()
                .map(|v| (v - lo) as usize)
                .collect(),
            BandLabels::Date(first) => {
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
        self.key_indices(hint.max_num_points())
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
            BandLabels::Date(first) => format!("{:?}", *first + TimeDelta::days(i as i64)),
            BandLabels::Category(names, _) => names.get(i).cloned().unwrap_or_default(),
        }
    }
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
        Series::new(SeriesAggregate::LineSeries, Column::Numeric(x), y).unwrap()
    }

    fn hist(x: Column, y: Vec<f64>) -> Series {
        Series::new(SeriesAggregate::Histogram, x, y).unwrap()
    }

    fn ranges(chart: &Chart) -> (Range<f64>, Range<f64>) {
        let series: Vec<&Series> = chart.series().collect();
        let x = x_coord(chart, &series).unwrap();
        assert!(matches!(x, AxisCoord::Linear(_) | AxisCoord::Band(_)));
        (x.range(), y_coord(chart, &series).unwrap().range())
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
            vec![1.0, 2.0],
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
            vec![1.0, 2.0],
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
