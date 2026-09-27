//! The coordinate types of the chart axes and how data values map onto them.
//!
//! plotters fixes the coordinate type of a chart at compile time
//! (`ChartContext<DB, Cartesian2d<X, Y>>`), while a chart value only knows its axis kinds at
//! run time. Each axis is therefore resolved to one variant of [`XCoord`] or [`YCoord`], and the
//! drawing code is generic over [`DataAxis`], which every coordinate type implements. The
//! render entry point matches on both enums and calls the generic code once per combination.
//! A new axis kind (log scales, secondary axes) is a new variant and a `DataAxis` impl.

use crate::error::{Error, Result};
use crate::spec::{AxisKind, AxisRange, Chart, Column, Series, SeriesKind};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeDelta};
use plotters::coord::ranged1d::{KeyPointHint, NoDefaultFormatting, Ranged, ValueFormatter};
use plotters::coord::types::{RangedCoordf64, RangedCoordi64};
use plotters::prelude::{RangedDate, RangedDateTime};
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::Arc;

/// A plotters coordinate that can place the values of a [`Column`] or a number.
pub(crate) trait DataAxis:
    Ranged + ValueFormatter<<Self as Ranged>::ValueType> + Clone + 'static
{
    /// The position of value `i` of `column`: the value itself, or the centre of its band.
    fn at(&self, column: &Column, i: usize) -> Option<Self::ValueType>;

    /// The position of a number (a y value, a bar height or a baseline).
    fn number(&self, _value: f64) -> Option<Self::ValueType> {
        None
    }

    /// The left and right edge of the band of value `i` of `column`, as
    /// `SegmentValue::Exact(v)` and `SegmentValue::Exact(v + 1)`. `None` on axes without bands
    /// and for values outside the axis, which plotters' segmented coordinates drop.
    fn band(&self, _column: &Column, _i: usize) -> Option<(Self::ValueType, Self::ValueType)> {
        None
    }
}

impl DataAxis for RangedCoordf64 {
    fn at(&self, column: &Column, i: usize) -> Option<f64> {
        match column {
            Column::Numeric(v) => Some(v[i]),
            _ => None,
        }
    }

    fn number(&self, value: f64) -> Option<f64> {
        Some(value)
    }
}

impl DataAxis for Oriented<RangedDate<NaiveDate>> {
    fn at(&self, column: &Column, i: usize) -> Option<NaiveDate> {
        match column {
            Column::Date(v) => date(v[i]),
            _ => None,
        }
    }
}

impl DataAxis for TimeCoord {
    fn at(&self, column: &Column, i: usize) -> Option<NaiveDateTime> {
        match column {
            Column::Timestamp(v) => timestamp(v[i]),
            _ => None,
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

impl DataAxis for BandCoord {
    fn at(&self, column: &Column, i: usize) -> Option<f64> {
        Some(self.index(column, i)? as f64 + 0.5)
    }

    fn band(&self, column: &Column, i: usize) -> Option<(f64, f64)> {
        let index = self.index(column, i)?;
        (0..self.n as i64)
            .contains(&index)
            .then_some((index as f64, index as f64 + 1.0))
    }
}

/// The resolved x axis.
pub(crate) enum XCoord {
    Linear(RangedCoordf64),
    Date(Oriented<RangedDate<NaiveDate>>),
    Timestamp(TimeCoord),
    Band(BandCoord),
}

/// The resolved y axis.
pub(crate) enum YCoord {
    Linear(RangedCoordf64),
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

pub(crate) fn x_coord(chart: &Chart, series: &[&Series]) -> Result<XCoord> {
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
            XCoord::Linear((lo..hi).into())
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
            XCoord::Timestamp(TimeCoord::new(lo, hi, reversed))
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
                XCoord::Band(BandCoord {
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
                XCoord::Date(Oriented {
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
            XCoord::Band(BandCoord {
                n: names.len().max(1),
                reversed: false,
                labels: BandLabels::Category(Arc::new(names), Arc::new(index)),
            })
        }
    })
}

pub(crate) fn y_coord(chart: &Chart, series: &[&Series]) -> Result<YCoord> {
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
    Ok(YCoord::Linear((lo..hi).into()))
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
        let x = match x_coord(chart, &series).unwrap() {
            XCoord::Linear(x) => x.range(),
            XCoord::Band(b) => b.range(),
            _ => panic!("not a numeric or band axis"),
        };
        let YCoord::Linear(y) = y_coord(chart, &series).unwrap();
        (x, y.range())
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
        let XCoord::Band(b) = x_coord(&c, &series).unwrap() else {
            panic!("not a band axis")
        };
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
        let XCoord::Band(b) = x_coord(&c, &series).unwrap() else {
            panic!("not a band axis")
        };
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
        assert!(matches!(x_coord(&c, &series).unwrap(), XCoord::Date(_)));
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
