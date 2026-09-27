//! Turning the rows of a SQL group into series: the state behind the series aggregates.
//!
//! The SQL layer drives an [`Accumulator`] the way DuckDB drives an aggregate:
//!
//! 1. bind: [`SeriesBinding::new`] checks the argument types and fails with the SQL error;
//! 2. update: [`Accumulator::push`] once per row, with values mapped to [`XValue`],
//!    `f64`, [`Key`] and [`SortKey`];
//! 3. combine: [`Accumulator::combine`] merges two partial states in any order;
//! 4. finalize: [`Accumulator::finish`] gives one `SERIES`, [`Accumulator::finish_keyed`] the
//!    `SERIES[]` of an aggregate called with `key := ...`.
//!
//! Rows whose x or y is NULL, NaN or infinite are skipped. The result does not depend on the
//! order rows arrive in or on how DuckDB partitions them: every series is sorted by
//! `order_by` (default: x), then x, then y.

use crate::error::{Error, Result};
use crate::spec::{
    AxisKind, Column, HistogramOptions, LineOptions, Marker, PointOptions, Series, SeriesKind,
    Style,
};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// The series aggregates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesAggregate {
    /// `line_series(x, y)`.
    LineSeries,
    /// `point_series(x, y)`.
    PointSeries,
    /// `histogram_vertical(bucket, value)`.
    Histogram,
}

impl SeriesAggregate {
    pub fn name(self) -> &'static str {
        match self {
            SeriesAggregate::LineSeries => "line_series",
            SeriesAggregate::PointSeries => "point_series",
            SeriesAggregate::Histogram => "histogram_vertical",
        }
    }

    fn x_name(self) -> &'static str {
        match self {
            SeriesAggregate::Histogram => "bucket",
            _ => "x",
        }
    }
}

/// The SQL type family of the x (or bucket) argument, as seen at bind time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlType {
    /// `TINYINT` .. `HUGEINT`, `UTINYINT` .. `UHUGEINT`.
    Integer,
    /// `FLOAT`, `DOUBLE`, `DECIMAL`.
    Float,
    Date,
    /// `TIMESTAMP`, `TIMESTAMPTZ`, `TIMESTAMP_S`, `TIMESTAMP_MS`, `TIMESTAMP_NS`.
    Timestamp,
    Varchar,
}

/// The bind-time result of a series aggregate: what the x column is and what the series is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeriesBinding {
    aggregate: SeriesAggregate,
    column: AxisKind,
}

impl SeriesBinding {
    /// Checks the x (or bucket) type of an aggregate. The error is the bind error to raise.
    pub fn new(aggregate: SeriesAggregate, x: SqlType) -> Result<SeriesBinding> {
        let column = match (aggregate, x) {
            (SeriesAggregate::Histogram, SqlType::Varchar) => AxisKind::Category,
            (SeriesAggregate::Histogram, SqlType::Integer) => AxisKind::Integer,
            (SeriesAggregate::Histogram, SqlType::Date) => AxisKind::Date,
            (SeriesAggregate::Histogram, SqlType::Float) => {
                return Err(Error::invalid(
                    "histogram buckets cannot be DOUBLE or DECIMAL: plotters' Histogram needs a \
                     discrete bucket axis; bin the values first (for example \
                     floor(x / 10)::INTEGER * 10), or use .step() once it exists",
                ));
            }
            (SeriesAggregate::Histogram, SqlType::Timestamp) => {
                return Err(Error::invalid(
                    "histogram buckets cannot be TIMESTAMP: cast them to DATE for one band per \
                     day, or bin them to integers",
                ));
            }
            (_, SqlType::Integer | SqlType::Float) => AxisKind::Numeric,
            (_, SqlType::Date) => AxisKind::Date,
            (_, SqlType::Timestamp) => AxisKind::Timestamp,
            (_, SqlType::Varchar) => AxisKind::Category,
        };
        Ok(SeriesBinding { aggregate, column })
    }

    pub fn aggregate(&self) -> SeriesAggregate {
        self.aggregate
    }

    /// The x axis kind the aggregate's series will have.
    pub fn x_kind(&self) -> AxisKind {
        self.column
    }

    /// The `XValue` variant `push` expects for this binding.
    pub fn expects(&self) -> &'static str {
        match self.column {
            AxisKind::Numeric => "XValue::Number (or XValue::Integer)",
            AxisKind::Integer => "XValue::Integer",
            AxisKind::Date => "XValue::Date",
            AxisKind::Timestamp => "XValue::Timestamp",
            AxisKind::Category => "XValue::Category",
        }
    }
}

/// One x (or bucket) value of a row.
#[derive(Clone, Debug)]
pub enum XValue {
    Number(f64),
    Integer(i64),
    /// Days since 1970-01-01.
    Date(i32),
    /// Microseconds since 1970-01-01 00:00:00.
    Timestamp(i64),
    Category(String),
}

impl XValue {
    fn rank(&self) -> u8 {
        match self {
            XValue::Number(_) => 0,
            XValue::Integer(_) => 1,
            XValue::Date(_) => 2,
            XValue::Timestamp(_) => 3,
            XValue::Category(_) => 4,
        }
    }
}

impl Ord for XValue {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (XValue::Number(a), XValue::Number(b)) => cmp_f64(*a, *b),
            (XValue::Integer(a), XValue::Integer(b)) => a.cmp(b),
            (XValue::Date(a), XValue::Date(b)) => a.cmp(b),
            (XValue::Timestamp(a), XValue::Timestamp(b)) => a.cmp(b),
            (XValue::Category(a), XValue::Category(b)) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

impl PartialOrd for XValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for XValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for XValue {}

/// A value of any SQL type reduced to something with DuckDB's sort order, for `key` and
/// `order_by`.
///
/// Suggested mapping: booleans to `Bool`; integers, `DATE`, `TIME`, `TIMESTAMP*` (as their
/// stored integer) to `Int`; `FLOAT`, `DOUBLE`, `DECIMAL` to `Float` (or `Int` of the unscaled
/// value); `VARCHAR` to `Text`; `BLOB` and `UUID` to `Bytes`; `INTERVAL`, `LIST`, `STRUCT` and
/// arrays to `List` of their parts; `NULL` of any type to `Null`.
///
/// Values of different variants order by variant (they do not occur in one SQL column).
/// `Null` sorts last, as DuckDB's default `NULLS LAST`. `NaN` sorts after every other float and
/// `-0.0` equals `0.0`, as in DuckDB.
#[derive(Clone, Debug)]
pub enum SortKey {
    Bool(bool),
    Int(i128),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    List(Vec<SortKey>),
    Null,
}

impl SortKey {
    fn rank(&self) -> u8 {
        match self {
            SortKey::Bool(_) => 0,
            SortKey::Int(_) => 1,
            SortKey::Float(_) => 2,
            SortKey::Text(_) => 3,
            SortKey::Bytes(_) => 4,
            SortKey::List(_) => 5,
            SortKey::Null => 6,
        }
    }
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (SortKey::Bool(a), SortKey::Bool(b)) => a.cmp(b),
            (SortKey::Int(a), SortKey::Int(b)) => a.cmp(b),
            (SortKey::Float(a), SortKey::Float(b)) => cmp_f64(*a, *b),
            (SortKey::Text(a), SortKey::Text(b)) => a.cmp(b),
            (SortKey::Bytes(a), SortKey::Bytes(b)) => a.cmp(b),
            (SortKey::List(a), SortKey::List(b)) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for SortKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SortKey {}

/// Floats in DuckDB order: NaN last, -0.0 equal to 0.0.
fn cmp_f64(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).expect("neither is NaN"),
    }
}

/// The `key := expr` value of a row: how it sorts, and the series label (`key::VARCHAR`, which
/// is `None` for a `NULL` key).
#[derive(Clone, Debug)]
pub struct Key {
    pub sort: SortKey,
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
struct Row {
    order: Option<SortKey>,
    x: XValue,
    y: f64,
}

#[derive(Clone, Debug, Default)]
struct Group {
    label: Option<String>,
    rows: Vec<Row>,
}

/// The state of one series aggregate for one SQL group.
#[derive(Clone, Debug, Default)]
pub struct Accumulator {
    groups: BTreeMap<SortKey, Group>,
}

impl Accumulator {
    pub fn new() -> Accumulator {
        Accumulator::default()
    }

    /// Adds one row. `key` is `Some` for every row of a keyed aggregate (with
    /// `SortKey::Null` for a `NULL` key) and `None` otherwise; `order_by` likewise.
    ///
    /// Rows with a missing, NaN or infinite x or y are skipped. An `x` of the wrong variant for
    /// the binding is an error, since it means the SQL layer mapped the column wrongly.
    pub fn push(
        &mut self,
        binding: &SeriesBinding,
        x: Option<XValue>,
        y: Option<f64>,
        key: Option<Key>,
        order_by: Option<SortKey>,
    ) -> Result<()> {
        let (Some(x), Some(y)) = (x, y) else {
            return Ok(());
        };
        if !y.is_finite() {
            return Ok(());
        }
        let x = match (binding.column, x) {
            (AxisKind::Numeric, XValue::Number(v)) if !v.is_finite() => return Ok(()),
            (AxisKind::Numeric, XValue::Number(v)) => XValue::Number(v),
            (AxisKind::Numeric, XValue::Integer(v)) => XValue::Number(v as f64),
            (AxisKind::Integer, x @ XValue::Integer(_))
            | (AxisKind::Date, x @ XValue::Date(_))
            | (AxisKind::Timestamp, x @ XValue::Timestamp(_))
            | (AxisKind::Category, x @ XValue::Category(_)) => x,
            (_, x) => {
                return Err(Error::invalid(format!(
                    "{}: {} value {x:?} does not match the bound type, which expects {}",
                    binding.aggregate.name(),
                    binding.aggregate.x_name(),
                    binding.expects()
                )));
            }
        };
        let (sort, label) = match key {
            Some(Key { sort, label }) => (sort, label),
            None => (SortKey::Null, None),
        };
        let group = self.groups.entry(sort).or_default();
        if group.label.is_none() {
            group.label = label;
        }
        group.rows.push(Row {
            order: order_by,
            x,
            y,
        });
        Ok(())
    }

    /// Merges another partial state into this one.
    pub fn combine(&mut self, other: Accumulator) {
        for (key, group) in other.groups {
            let mine = self.groups.entry(key).or_default();
            if mine.label.is_none() {
                mine.label = group.label;
            }
            mine.rows.extend(group.rows);
        }
    }

    /// Whether a row with this key has been pushed, so the SQL layer computes a key's label
    /// (`key::VARCHAR`) only for the first row of each key.
    pub fn contains_key(&self, key: &SortKey) -> bool {
        self.groups.contains_key(key)
    }

    /// The number of usable rows pushed so far.
    pub fn len(&self) -> usize {
        self.groups.values().map(|g| g.rows.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `SERIES` of an aggregate without `key`. Zero usable rows give an empty series.
    pub fn finish(self, binding: &SeriesBinding) -> Result<Series> {
        let rows = self.groups.into_values().flat_map(|g| g.rows).collect();
        build(binding, rows)
    }

    /// The `SERIES[]` of an aggregate with `key`: one series per distinct key, ordered by key
    /// and labelled with the key's text.
    pub fn finish_keyed(self, binding: &SeriesBinding) -> Result<Vec<Series>> {
        self.groups
            .into_values()
            .map(|group| {
                let series = build(binding, group.rows)?;
                Ok(match group.label {
                    Some(label) => series.label(label),
                    None => series,
                })
            })
            .collect()
    }
}

fn build(binding: &SeriesBinding, mut rows: Vec<Row>) -> Result<Series> {
    rows.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.x.cmp(&b.x))
            .then_with(|| cmp_f64(a.y, b.y))
    });
    let (xs, ys): (Vec<XValue>, Vec<f64>) = match binding.aggregate {
        SeriesAggregate::Histogram => {
            // Sum per bucket, buckets in order of first appearance, sums in sorted row order.
            let mut index: BTreeMap<XValue, usize> = BTreeMap::new();
            let mut buckets: Vec<(XValue, f64)> = Vec::new();
            for row in rows {
                match index.get(&row.x) {
                    Some(&i) => buckets[i].1 += row.y,
                    None => {
                        index.insert(row.x.clone(), buckets.len());
                        buckets.push((row.x, row.y));
                    }
                }
            }
            buckets.into_iter().unzip()
        }
        _ => rows.into_iter().map(|r| (r.x, r.y)).unzip(),
    };
    let column = match binding.column {
        AxisKind::Numeric => Column::Numeric(collect(xs, |x| match x {
            XValue::Number(v) => Some(v),
            _ => None,
        })),
        AxisKind::Integer => Column::Integer(collect(xs, |x| match x {
            XValue::Integer(v) => Some(v),
            _ => None,
        })),
        AxisKind::Date => Column::Date(collect(xs, |x| match x {
            XValue::Date(v) => Some(v),
            _ => None,
        })),
        AxisKind::Timestamp => Column::Timestamp(collect(xs, |x| match x {
            XValue::Timestamp(v) => Some(v),
            _ => None,
        })),
        AxisKind::Category => Column::Category(collect(xs, |x| match x {
            XValue::Category(v) => Some(v),
            _ => None,
        })),
    };
    Series::new(binding.aggregate, column, ys)
}

fn collect<T>(xs: Vec<XValue>, f: impl Fn(XValue) -> Option<T>) -> Vec<T> {
    // `push` normalised every value to the binding's variant.
    xs.into_iter()
        .map(|x| f(x).expect("x values match the binding"))
        .collect()
}

impl Series {
    /// A series of the given aggregate's kind with plotters' (and duckers') defaults, from
    /// ready columns: points in drawing order, or histogram buckets (repeated buckets are summed
    /// when drawn, as `Histogram::data` does).
    pub fn new(aggregate: SeriesAggregate, x: Column, y: Vec<f64>) -> Result<Series> {
        if x.len() != y.len() {
            return Err(Error::invalid(format!(
                "{}: {} x values but {} y values",
                aggregate.name(),
                x.len(),
                y.len()
            )));
        }
        let allowed = match aggregate {
            SeriesAggregate::Histogram => matches!(
                x.axis_kind(),
                AxisKind::Integer | AxisKind::Date | AxisKind::Category
            ),
            _ => x.axis_kind() != AxisKind::Integer,
        };
        if !allowed {
            return Err(Error::invalid(format!(
                "{}: {} columns cannot be its {}",
                aggregate.name(),
                x.axis_kind(),
                aggregate.x_name()
            )));
        }
        let style = Style {
            color: None,
            stroke_width: 1,
            filled: false,
        };
        let (kind, style) = match aggregate {
            SeriesAggregate::LineSeries => (SeriesKind::Line(LineOptions { point_size: 0 }), style),
            SeriesAggregate::PointSeries => (
                SeriesKind::Point(PointOptions {
                    size: 3,
                    marker: Marker::Circle,
                }),
                style,
            ),
            SeriesAggregate::Histogram => (
                SeriesKind::Histogram(HistogramOptions {
                    margin: 5,
                    baseline: 0.0,
                }),
                Style {
                    filled: true,
                    ..style
                },
            ),
        };
        Ok(Series {
            kind,
            x,
            y,
            style,
            label: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> SeriesBinding {
        SeriesBinding::new(SeriesAggregate::LineSeries, SqlType::Float).unwrap()
    }

    fn num(v: f64) -> Option<XValue> {
        Some(XValue::Number(v))
    }

    fn xs(s: &Series) -> Vec<f64> {
        match &s.x {
            Column::Numeric(v) => v.clone(),
            other => panic!("not numeric: {other:?}"),
        }
    }

    #[test]
    fn sorts_by_x_then_y() {
        let b = line();
        let mut acc = Accumulator::new();
        for (x, y) in [(3.0, 1.0), (1.0, 5.0), (2.0, 0.0), (1.0, 2.0)] {
            acc.push(&b, num(x), Some(y), None, None).unwrap();
        }
        let s = acc.finish(&b).unwrap();
        assert_eq!(xs(&s), [1.0, 1.0, 2.0, 3.0]);
        assert_eq!(s.y, [2.0, 5.0, 0.0, 1.0]);
    }

    #[test]
    fn order_by_overrides_x() {
        let b = line();
        let mut acc = Accumulator::new();
        for (x, t) in [(1.0, 3), (2.0, 1), (3.0, 2)] {
            acc.push(&b, num(x), Some(0.0), None, Some(SortKey::Int(t)))
                .unwrap();
        }
        assert_eq!(xs(&acc.finish(&b).unwrap()), [2.0, 3.0, 1.0]);
    }

    #[test]
    fn skips_nulls_nan_and_infinities() {
        let b = line();
        let mut acc = Accumulator::new();
        acc.push(&b, None, Some(1.0), None, None).unwrap();
        acc.push(&b, num(1.0), None, None, None).unwrap();
        acc.push(&b, num(f64::NAN), Some(1.0), None, None).unwrap();
        acc.push(&b, num(1.0), Some(f64::INFINITY), None, None)
            .unwrap();
        acc.push(&b, num(f64::NEG_INFINITY), Some(1.0), None, None)
            .unwrap();
        assert!(acc.is_empty());
        acc.push(&b, num(1.0), Some(1.0), None, None).unwrap();
        let s = acc.finish(&b).unwrap();
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn empty_aggregate_gives_empty_series() {
        let b = SeriesBinding::new(SeriesAggregate::PointSeries, SqlType::Date).unwrap();
        let s = Accumulator::new().finish(&b).unwrap();
        assert_eq!(s.x, Column::Date(vec![]));
        assert!(Accumulator::new().finish_keyed(&b).unwrap().is_empty());
    }

    #[test]
    fn keys_split_and_label_in_key_order() {
        let b = line();
        let mut acc = Accumulator::new();
        let key = |k: &str| {
            Some(Key {
                sort: SortKey::Text(k.into()),
                label: Some(k.into()),
            })
        };
        acc.push(&b, num(2.0), Some(1.0), key("oslo"), None)
            .unwrap();
        acc.push(&b, num(1.0), Some(1.0), key("bergen"), None)
            .unwrap();
        acc.push(&b, num(1.0), Some(2.0), key("oslo"), None)
            .unwrap();
        let null = Key {
            sort: SortKey::Null,
            label: None,
        };
        acc.push(&b, num(1.0), Some(2.0), Some(null), None).unwrap();
        let series = acc.finish_keyed(&b).unwrap();
        let labels: Vec<_> = series.iter().map(|s| s.label.as_deref()).collect();
        assert_eq!(labels, [Some("bergen"), Some("oslo"), None]);
        assert_eq!(xs(&series[1]), [1.0, 2.0]);
    }

    #[test]
    fn combine_is_order_independent() {
        let b = SeriesBinding::new(SeriesAggregate::Histogram, SqlType::Varchar).unwrap();
        let rows: Vec<(&str, f64)> = vec![("b", 0.1), ("a", 0.2), ("b", 0.3), ("c", 1e16)];
        let make = |rows: &[(&str, f64)]| {
            let mut acc = Accumulator::new();
            for (x, y) in rows {
                acc.push(
                    &b,
                    Some(XValue::Category((*x).into())),
                    Some(*y),
                    None,
                    None,
                )
                .unwrap();
            }
            acc
        };
        let mut left = make(&rows[..2]);
        left.combine(make(&rows[2..]));
        let mut right = make(&rows[2..]);
        right.combine(make(&rows[..2]));
        let (l, r) = (left.finish(&b).unwrap(), right.finish(&b).unwrap());
        assert_eq!(l, r);
        assert_eq!(
            l.x,
            Column::Category(vec!["a".into(), "b".into(), "c".into()])
        );
        assert_eq!(l.y, [0.2, 0.1 + 0.3, 1e16]);
    }

    #[test]
    fn histogram_order_by_orders_buckets() {
        let b = SeriesBinding::new(SeriesAggregate::Histogram, SqlType::Varchar).unwrap();
        let mut acc = Accumulator::new();
        for (x, n) in [("a", 5.0), ("b", 9.0), ("c", 1.0)] {
            acc.push(
                &b,
                Some(XValue::Category(x.into())),
                Some(n),
                None,
                Some(SortKey::Float(-n)),
            )
            .unwrap();
        }
        let s = acc.finish(&b).unwrap();
        assert_eq!(
            s.x,
            Column::Category(vec!["b".into(), "a".into(), "c".into()])
        );
        assert_eq!(s.y, [9.0, 5.0, 1.0]);
    }

    #[test]
    fn histogram_rejects_double_buckets() {
        let err = SeriesBinding::new(SeriesAggregate::Histogram, SqlType::Float).unwrap_err();
        assert!(err.message().contains("bin the values first"), "{err}");
        assert!(err.message().contains(".step()"), "{err}");
        assert!(SeriesBinding::new(SeriesAggregate::Histogram, SqlType::Timestamp).is_err());
        let int = SeriesBinding::new(SeriesAggregate::Histogram, SqlType::Integer).unwrap();
        assert_eq!(int.x_kind(), AxisKind::Integer);
    }

    #[test]
    fn integers_become_numbers_on_numeric_axes() {
        let b = SeriesBinding::new(SeriesAggregate::LineSeries, SqlType::Integer).unwrap();
        let mut acc = Accumulator::new();
        acc.push(&b, Some(XValue::Integer(4)), Some(1.0), None, None)
            .unwrap();
        assert_eq!(xs(&acc.finish(&b).unwrap()), [4.0]);
        let err = Accumulator::new()
            .push(&b, Some(XValue::Date(1)), Some(1.0), None, None)
            .unwrap_err();
        assert!(err.message().starts_with("line_series: x value"), "{err}");
    }

    #[test]
    fn sort_key_order() {
        use SortKey::*;
        let mut keys = [
            Null,
            Float(f64::NAN),
            Float(1.0),
            Float(-0.0),
            Float(f64::NEG_INFINITY),
        ];
        keys.sort();
        assert!(matches!(keys[0], Float(v) if v == f64::NEG_INFINITY));
        assert!(matches!(keys[3], Float(v) if v.is_nan()));
        assert!(matches!(keys[4], Null));
        assert_eq!(Float(-0.0), Float(0.0));
        assert!(Int(-5) < Int(3));
        assert!(Text("a".into()) < Text("b".into()));
        assert!(List(vec![Int(1), Int(2)]) < List(vec![Int(1), Int(3)]));
        assert!(Bool(true) < Null);
    }
}
