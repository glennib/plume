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
//! Rows whose x or any value is NULL, NaN or infinite are skipped. The result does not depend
//! on the order rows arrive in or on how DuckDB partitions them: every series is sorted by
//! `order_by` (default: x), then x, then the values.

use crate::color::Color;
use crate::error::{Error, Result};
use crate::spec::{
    AreaOptions, AxisKind, BoxplotOptions, CandleStickOptions, Column, DashedLineOptions,
    ErrorBarOptions, HistogramOptions, LineOptions, LineStyle, Marker, PointOptions, Series,
    SeriesKind, Style,
};
use plotters::data::Quartiles;
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// The most value arguments an aggregate takes (`candle_stick`'s open, high, low, close).
const MAX_VALUES: usize = 4;

/// The series aggregates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesAggregate {
    /// `line_series(x, y)`.
    LineSeries,
    /// `point_series(x, y)`.
    PointSeries,
    /// `histogram_vertical(bucket, value)`.
    Histogram,
    /// `histogram_horizontal(bucket, value)`.
    HistogramHorizontal,
    /// `area_series(x, y)`.
    AreaSeries,
    /// `dashed_line_series(x, y)`.
    DashedLineSeries,
    /// `error_bar_vertical(x, min, avg, max)`.
    ErrorBarVertical,
    /// `error_bar_horizontal(y, min, avg, max)`.
    ErrorBarHorizontal,
    /// `candle_stick(x, open, high, low, close)`.
    CandleStick,
    /// `boxplot_vertical(key, value)`.
    BoxplotVertical,
    /// `boxplot_horizontal(key, value)`.
    BoxplotHorizontal,
}

impl SeriesAggregate {
    /// Every aggregate, in registration order.
    pub const ALL: [SeriesAggregate; 11] = [
        SeriesAggregate::LineSeries,
        SeriesAggregate::PointSeries,
        SeriesAggregate::Histogram,
        SeriesAggregate::HistogramHorizontal,
        SeriesAggregate::AreaSeries,
        SeriesAggregate::DashedLineSeries,
        SeriesAggregate::ErrorBarVertical,
        SeriesAggregate::ErrorBarHorizontal,
        SeriesAggregate::CandleStick,
        SeriesAggregate::BoxplotVertical,
        SeriesAggregate::BoxplotHorizontal,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SeriesAggregate::LineSeries => "line_series",
            SeriesAggregate::PointSeries => "point_series",
            SeriesAggregate::Histogram => "histogram_vertical",
            SeriesAggregate::HistogramHorizontal => "histogram_horizontal",
            SeriesAggregate::AreaSeries => "area_series",
            SeriesAggregate::DashedLineSeries => "dashed_line_series",
            SeriesAggregate::ErrorBarVertical => "error_bar_vertical",
            SeriesAggregate::ErrorBarHorizontal => "error_bar_horizontal",
            SeriesAggregate::CandleStick => "candle_stick",
            SeriesAggregate::BoxplotVertical => "boxplot_vertical",
            SeriesAggregate::BoxplotHorizontal => "boxplot_horizontal",
        }
    }

    /// Whether the aggregate sums values per bucket.
    pub fn is_histogram(self) -> bool {
        matches!(
            self,
            SeriesAggregate::Histogram | SeriesAggregate::HistogramHorizontal
        )
    }

    /// Whether the aggregate draws one box per distinct key.
    pub fn is_boxplot(self) -> bool {
        matches!(
            self,
            SeriesAggregate::BoxplotVertical | SeriesAggregate::BoxplotHorizontal
        )
    }

    /// Whether the first argument goes on the y axis and the values on x.
    pub fn is_horizontal(self) -> bool {
        matches!(
            self,
            SeriesAggregate::HistogramHorizontal
                | SeriesAggregate::ErrorBarHorizontal
                | SeriesAggregate::BoxplotHorizontal
        )
    }

    /// The name of the first argument: `x`, `y` or `bucket`. A boxplot's key (plotters'
    /// `Boxplot::new_vertical(key, ...)`) is `bucket` too, since `key` names the `key :=`
    /// parameter of every aggregate.
    pub fn x_name(self) -> &'static str {
        match self {
            _ if self.is_histogram() || self.is_boxplot() => "bucket",
            SeriesAggregate::ErrorBarHorizontal => "y",
            _ => "x",
        }
    }

    /// The names of the value arguments that follow the first.
    pub fn value_names(self) -> &'static [&'static str] {
        match self {
            _ if self.is_histogram() || self.is_boxplot() => &["value"],
            SeriesAggregate::ErrorBarVertical | SeriesAggregate::ErrorBarHorizontal => {
                &["min", "avg", "max"]
            }
            SeriesAggregate::CandleStick => &["open", "high", "low", "close"],
            _ => &["y"],
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
        let bucketed = aggregate.is_histogram() || aggregate.is_boxplot();
        let column = match (bucketed, x) {
            (true, SqlType::Varchar) => AxisKind::Category,
            (true, SqlType::Integer) => AxisKind::Integer,
            (true, SqlType::Date) => AxisKind::Date,
            // Binned into bands by `.step(s)`, which drawing requires.
            (true, SqlType::Float) if aggregate.is_histogram() => AxisKind::Numeric,
            (true, SqlType::Float) => {
                return Err(Error::invalid(
                    "boxplot buckets cannot be DOUBLE or DECIMAL: plotters' Boxplot draws one box \
                     per distinct key on a segmented axis; cast them to an integer or VARCHAR, \
                     or bin them first (for example floor(x / 10)::INTEGER * 10)",
                ));
            }
            (true, SqlType::Timestamp) => {
                let what = if aggregate.is_histogram() {
                    "histogram buckets"
                } else {
                    "boxplot buckets"
                };
                return Err(Error::invalid(format!(
                    "{what} cannot be TIMESTAMP: cast them to DATE for one band per day, or bin \
                     them to integers"
                )));
            }
            (false, SqlType::Integer | SqlType::Float) => AxisKind::Numeric,
            (false, SqlType::Date) => AxisKind::Date,
            (false, SqlType::Timestamp) => AxisKind::Timestamp,
            (false, SqlType::Varchar) => AxisKind::Category,
        };
        Ok(SeriesBinding { aggregate, column })
    }

    pub fn aggregate(&self) -> SeriesAggregate {
        self.aggregate
    }

    /// The kind of the first argument's column: the x axis kind, or the bucket axis kind of a
    /// histogram (y for `histogram_horizontal`).
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
    /// The value arguments in order; those past the aggregate's count are 0.
    values: [f64; MAX_VALUES],
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
        self.push_values(binding, x, &[y], key, order_by)
    }

    /// Adds one row of an aggregate with any number of value arguments (`min, avg, max` of an
    /// error bar, `open, high, low, close` of a candlestick), in argument order. `push` is
    /// this with one value.
    ///
    /// A row with a missing, NaN or infinite value is skipped as a whole. Passing a number of
    /// values other than the aggregate's is an error.
    pub fn push_values(
        &mut self,
        binding: &SeriesBinding,
        x: Option<XValue>,
        values: &[Option<f64>],
        key: Option<Key>,
        order_by: Option<SortKey>,
    ) -> Result<()> {
        let expected = binding.aggregate.value_names().len();
        if values.len() != expected {
            return Err(Error::invalid(format!(
                "{}: {} values in a row, but it takes {expected}",
                binding.aggregate.name(),
                values.len()
            )));
        }
        let Some(x) = x else {
            return Ok(());
        };
        let mut row_values = [0.0; MAX_VALUES];
        for (slot, value) in row_values.iter_mut().zip(values) {
            match value {
                Some(v) if v.is_finite() => *slot = *v,
                _ => return Ok(()),
            }
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
            values: row_values,
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
            .then_with(|| {
                a.values
                    .iter()
                    .zip(&b.values)
                    .map(|(a, b)| cmp_f64(*a, *b))
                    .find(|o| o.is_ne())
                    .unwrap_or(Ordering::Equal)
            })
    });
    let aggregate = binding.aggregate;
    if aggregate.is_boxplot() {
        // One box per key, keys in order of first appearance.
        let mut index: BTreeMap<XValue, usize> = BTreeMap::new();
        let mut keys: Vec<(XValue, Vec<f64>)> = Vec::new();
        for row in rows {
            match index.get(&row.x) {
                Some(&i) => keys[i].1.push(row.values[0]),
                None => {
                    index.insert(row.x.clone(), keys.len());
                    keys.push((row.x, vec![row.values[0]]));
                }
            }
        }
        let (xs, quartiles): (Vec<XValue>, Vec<[f64; 5]>) = keys
            .into_iter()
            .map(|(key, values)| (key, quartile_values(&values)))
            .unzip();
        let column = key_column(binding.column, xs);
        let medians = Column::Numeric(quartiles.iter().map(|q| q[2]).collect());
        let (x, y) = if aggregate.is_horizontal() {
            (medians, column)
        } else {
            (column, medians)
        };
        let mut series = Series::new(aggregate, x, y)?;
        if let SeriesKind::BoxplotVertical(o) | SeriesKind::BoxplotHorizontal(o) = &mut series.kind
        {
            o.quartiles = quartiles;
        }
        return Ok(series);
    }
    let (xs, values): (Vec<XValue>, Vec<[f64; MAX_VALUES]>) = if aggregate.is_histogram() {
        // Sum per bucket, buckets in order of first appearance, sums in sorted row order.
        let mut index: BTreeMap<XValue, usize> = BTreeMap::new();
        let mut buckets: Vec<(XValue, [f64; MAX_VALUES])> = Vec::new();
        for row in rows {
            match index.get(&row.x) {
                Some(&i) => buckets[i].1[0] += row.values[0],
                None => {
                    index.insert(row.x.clone(), buckets.len());
                    buckets.push((row.x, row.values));
                }
            }
        }
        buckets.into_iter().unzip()
    } else {
        rows.into_iter().map(|r| (r.x, r.values)).unzip()
    };
    let column = key_column(binding.column, xs);
    let value = |i: usize| -> Vec<f64> { values.iter().map(|v| v[i]).collect() };
    // The value drawn on the value axis: `avg` of an error bar, `close` of a candlestick.
    let main = match aggregate {
        SeriesAggregate::ErrorBarVertical | SeriesAggregate::ErrorBarHorizontal => 1,
        SeriesAggregate::CandleStick => 3,
        _ => 0,
    };
    let values_column = Column::Numeric(value(main));
    let (x, y) = if aggregate.is_horizontal() {
        (values_column, column)
    } else {
        (column, values_column)
    };
    let mut series = Series::new(aggregate, x, y)?;
    match &mut series.kind {
        SeriesKind::ErrorBarVertical(o) | SeriesKind::ErrorBarHorizontal(o) => {
            o.min = value(0);
            o.max = value(2);
        }
        SeriesKind::CandleStick(o) => {
            o.open = value(0);
            o.high = value(1);
            o.low = value(2);
        }
        _ => {}
    }
    Ok(series)
}

/// `Quartiles::new(values).values()`, widened to `f64`. plotters computes the quartiles in
/// `f64` and hands them out as `f32`; the box is drawn from these numbers.
pub fn quartile_values(values: &[f64]) -> [f64; 5] {
    Quartiles::new(values).values().map(f64::from)
}

/// The first argument's values as the column of the binding's kind.
fn key_column(kind: AxisKind, xs: Vec<XValue>) -> Column {
    match kind {
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
    }
}

fn collect<T>(xs: Vec<XValue>, f: impl Fn(XValue) -> Option<T>) -> Vec<T> {
    // `push` normalised every value to the binding's variant.
    xs.into_iter()
        .map(|x| f(x).expect("x values match the binding"))
        .collect()
}

impl Series {
    /// A series of the given aggregate's kind with plotters' (and plume') defaults, from
    /// ready columns on the x and y axis: points in drawing order, or histogram buckets and
    /// values (repeated buckets are summed when drawn, as `Histogram::data` does). The first
    /// argument of the horizontal kinds (`histogram_horizontal`, `error_bar_horizontal`,
    /// `boxplot_horizontal`) is the `y` column.
    ///
    /// The value column of an error bar is its `avg`, of a candlestick its `close`, and of a
    /// boxplot its median; their other values start empty and every one is `0` when drawn
    /// until the aggregate fills them in.
    pub fn new(aggregate: SeriesAggregate, x: Column, y: Column) -> Result<Series> {
        if x.len() != y.len() {
            return Err(Error::invalid(format!(
                "{}: {} x values but {} y values",
                aggregate.name(),
                x.len(),
                y.len()
            )));
        }
        let bucket = |c: &Column| {
            matches!(
                c.axis_kind(),
                AxisKind::Integer | AxisKind::Date | AxisKind::Category
            ) || (aggregate.is_histogram() && c.axis_kind() == AxisKind::Numeric)
        };
        let numeric = |c: &Column| c.axis_kind() == AxisKind::Numeric;
        let point = |c: &Column| c.axis_kind() != AxisKind::Integer;
        let (key, values) = if aggregate.is_horizontal() {
            (&y, &x)
        } else {
            (&x, &y)
        };
        let key_ok = if aggregate.is_histogram() || aggregate.is_boxplot() {
            bucket(key)
        } else {
            point(key)
        };
        let (column, what) = match (key_ok, numeric(values)) {
            (true, true) => (None, ""),
            (false, _) => (Some(key), aggregate.x_name()),
            (_, false) => (Some(values), aggregate.value_names()[0]),
        };
        if let Some(column) = column {
            return Err(Error::invalid(format!(
                "{}: {} columns cannot be its {what}",
                aggregate.name(),
                column.axis_kind(),
            )));
        }
        let n = x.len();
        let style = Style {
            color: None,
            stroke_width: 1,
            filled: false,
        };
        let filled = Style {
            filled: true,
            ..style.clone()
        };
        let histogram = HistogramOptions {
            margin: 5,
            baseline: 0.0,
            step: None,
        };
        let error_bar = ErrorBarOptions {
            width: 10,
            min: vec![0.0; n],
            max: vec![0.0; n],
        };
        let boxplot = BoxplotOptions {
            width: 10,
            quartiles: vec![[0.0; 5]; n],
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
            SeriesAggregate::Histogram => (SeriesKind::Histogram(histogram), filled),
            SeriesAggregate::HistogramHorizontal => {
                (SeriesKind::HistogramHorizontal(histogram), filled)
            }
            // plotters fills an area's polygon whatever the style says.
            SeriesAggregate::AreaSeries => (
                SeriesKind::Area(AreaOptions {
                    baseline: 0.0,
                    border_style: LineStyle {
                        color: Color::TRANSPARENT,
                        stroke_width: 1,
                    },
                }),
                filled,
            ),
            SeriesAggregate::DashedLineSeries => (
                SeriesKind::DashedLine(DashedLineOptions {
                    size: 5,
                    spacing: 5,
                }),
                style,
            ),
            SeriesAggregate::ErrorBarVertical => (SeriesKind::ErrorBarVertical(error_bar), style),
            SeriesAggregate::ErrorBarHorizontal => {
                (SeriesKind::ErrorBarHorizontal(error_bar), style)
            }
            SeriesAggregate::CandleStick => (
                SeriesKind::CandleStick(Box::new(CandleStickOptions {
                    width: 10,
                    gain: Color::rgb(0, 255, 0),
                    loss: Color::rgb(255, 0, 0),
                    open: vec![0.0; n],
                    high: vec![0.0; n],
                    low: vec![0.0; n],
                })),
                style,
            ),
            SeriesAggregate::BoxplotVertical => (SeriesKind::BoxplotVertical(boxplot), style),
            SeriesAggregate::BoxplotHorizontal => (SeriesKind::BoxplotHorizontal(boxplot), style),
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
    use crate::spec::{AreaOptions, LineStyle};

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
        assert_eq!(s.y, Column::Numeric(vec![2.0, 5.0, 0.0, 1.0]));
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
        assert_eq!(l.y, Column::Numeric(vec![0.2, 0.1 + 0.3, 1e16]));
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
        assert_eq!(s.y, Column::Numeric(vec![9.0, 5.0, 1.0]));
    }

    #[test]
    fn horizontal_histograms_put_buckets_on_y() {
        let b = SeriesBinding::new(SeriesAggregate::HistogramHorizontal, SqlType::Varchar).unwrap();
        assert_eq!(b.x_kind(), AxisKind::Category);
        let mut acc = Accumulator::new();
        for (x, n) in [("b", 1.0), ("a", 2.0), ("b", 3.0)] {
            acc.push(&b, Some(XValue::Category(x.into())), Some(n), None, None)
                .unwrap();
        }
        let s = acc.finish(&b).unwrap();
        assert_eq!(s.x, Column::Numeric(vec![2.0, 4.0]));
        assert_eq!(s.y, Column::Category(vec!["a".into(), "b".into()]));
        assert!(matches!(s.kind, SeriesKind::HistogramHorizontal(_)));
        assert!(s.style.filled);
        let float = SeriesBinding::new(SeriesAggregate::HistogramHorizontal, SqlType::Float);
        assert_eq!(float.unwrap().x_kind(), AxisKind::Numeric);
        let err = Series::new(
            SeriesAggregate::HistogramHorizontal,
            Column::Date(vec![1]),
            Column::Numeric(vec![1.0]),
        )
        .unwrap_err();
        assert_eq!(
            err.message(),
            "histogram_horizontal: date columns cannot be its value"
        );
        let err = Series::new(
            SeriesAggregate::HistogramHorizontal,
            Column::Numeric(vec![1.0]),
            Column::Timestamp(vec![1]),
        )
        .unwrap_err();
        assert_eq!(
            err.message(),
            "histogram_horizontal: timestamp columns cannot be its bucket"
        );
    }

    #[test]
    fn histogram_bucket_types() {
        // Numeric buckets bind; drawing them needs .step(s).
        let float = SeriesBinding::new(SeriesAggregate::Histogram, SqlType::Float).unwrap();
        assert_eq!(float.x_kind(), AxisKind::Numeric);
        let mut acc = Accumulator::new();
        for x in [0.5, 1.5, 0.5] {
            acc.push(&float, num(x), Some(1.0), None, None).unwrap();
        }
        let s = acc.finish(&float).unwrap();
        assert_eq!(s.x, Column::Numeric(vec![0.5, 1.5]));
        assert_eq!(s.y, Column::Numeric(vec![2.0, 1.0]));
        let err = SeriesBinding::new(SeriesAggregate::Histogram, SqlType::Timestamp).unwrap_err();
        assert!(
            err.message()
                .starts_with("histogram buckets cannot be TIMESTAMP")
        );
        let int = SeriesBinding::new(SeriesAggregate::Histogram, SqlType::Integer).unwrap();
        assert_eq!(int.x_kind(), AxisKind::Integer);
    }

    fn push_all(b: &SeriesBinding, rows: &[(XValue, &[f64])]) -> Series {
        let mut acc = Accumulator::new();
        for (x, values) in rows {
            let values: Vec<Option<f64>> = values.iter().map(|v| Some(*v)).collect();
            acc.push_values(b, Some(x.clone()), &values, None, None)
                .unwrap();
        }
        acc.finish(b).unwrap()
    }

    #[test]
    fn area_and_dashed_line_are_points() {
        for (aggregate, name) in [
            (SeriesAggregate::AreaSeries, "area"),
            (SeriesAggregate::DashedLineSeries, "dashed line"),
        ] {
            let b = SeriesBinding::new(aggregate, SqlType::Float).unwrap();
            let s = push_all(
                &b,
                &[(XValue::Number(2.0), &[1.0]), (XValue::Number(1.0), &[3.0])],
            );
            assert_eq!(s.kind.name(), name);
            assert_eq!(xs(&s), [1.0, 2.0]);
            assert_eq!(s.y, Column::Numeric(vec![3.0, 1.0]));
        }
        let b = SeriesBinding::new(SeriesAggregate::AreaSeries, SqlType::Float).unwrap();
        let s = push_all(&b, &[]);
        assert!(s.style.filled, "an area's polygon is filled");
        assert!(matches!(
            s.kind,
            SeriesKind::Area(AreaOptions { baseline: 0.0, border_style: LineStyle { color, .. } })
                if color == Color::TRANSPARENT
        ));
    }

    #[test]
    fn error_bars_keep_min_avg_max() {
        let b = SeriesBinding::new(SeriesAggregate::ErrorBarVertical, SqlType::Integer).unwrap();
        let s = push_all(
            &b,
            &[
                (XValue::Integer(2), &[1.0, 2.0, 3.0]),
                (XValue::Integer(1), &[4.0, 5.0, 6.0]),
            ],
        );
        assert_eq!(xs(&s), [1.0, 2.0]);
        assert_eq!(s.y, Column::Numeric(vec![5.0, 2.0]));
        let SeriesKind::ErrorBarVertical(o) = &s.kind else {
            panic!("{s:?}")
        };
        assert_eq!(
            (o.width, o.min.clone(), o.max.clone()),
            (10, vec![4.0, 1.0], vec![6.0, 3.0])
        );

        // Horizontal ones have the key on y.
        let b = SeriesBinding::new(SeriesAggregate::ErrorBarHorizontal, SqlType::Varchar).unwrap();
        let s = push_all(&b, &[(XValue::Category("a".into()), &[1.0, 2.0, 3.0])]);
        assert_eq!(s.y, Column::Category(vec!["a".into()]));
        assert_eq!(s.x, Column::Numeric(vec![2.0]));
        assert_eq!(s.key_axis(), Some(crate::spec::Axis::Y));
        assert_eq!(s.extra_numbers(crate::spec::Axis::X), [1.0, 3.0]);
        assert!(s.extra_numbers(crate::spec::Axis::Y).is_empty());

        // A row with any value missing or not finite is skipped; the arity is checked.
        let mut acc = Accumulator::new();
        acc.push_values(
            &b,
            Some(XValue::Category("a".into())),
            &[Some(1.0), None, Some(2.0)],
            None,
            None,
        )
        .unwrap();
        acc.push_values(
            &b,
            Some(XValue::Category("a".into())),
            &[Some(1.0), Some(f64::NAN), Some(2.0)],
            None,
            None,
        )
        .unwrap();
        assert!(acc.is_empty());
        let err = acc
            .push(
                &b,
                Some(XValue::Category("a".into())),
                Some(1.0),
                None,
                None,
            )
            .unwrap_err();
        assert_eq!(
            err.message(),
            "error_bar_horizontal: 1 values in a row, but it takes 3"
        );
    }

    #[test]
    fn candle_sticks_keep_open_high_low_close() {
        let b = SeriesBinding::new(SeriesAggregate::CandleStick, SqlType::Date).unwrap();
        let s = push_all(&b, &[(XValue::Date(3), &[1.0, 4.0, 0.5, 2.0])]);
        assert_eq!(s.x, Column::Date(vec![3]));
        assert_eq!(s.y, Column::Numeric(vec![2.0]));
        let SeriesKind::CandleStick(o) = &s.kind else {
            panic!("{s:?}")
        };
        assert_eq!((o.open[0], o.high[0], o.low[0]), (1.0, 4.0, 0.5));
        assert_eq!(
            (o.gain, o.loss),
            (Color::rgb(0, 255, 0), Color::rgb(255, 0, 0))
        );
        assert_eq!(s.extra_numbers(crate::spec::Axis::Y), [1.0, 4.0, 0.5]);
    }

    #[test]
    fn boxplots_compute_quartiles_per_key() {
        let b = SeriesBinding::new(SeriesAggregate::BoxplotVertical, SqlType::Varchar).unwrap();
        let mut rows: Vec<(XValue, &[f64])> = Vec::new();
        for v in [&[10.0][..], &[20.0], &[30.0], &[40.0]] {
            rows.push((XValue::Category("b".into()), v));
        }
        rows.push((XValue::Category("a".into()), &[5.0]));
        let s = push_all(&b, &rows);
        assert_eq!(s.x, Column::Category(vec!["a".into(), "b".into()]));
        assert_eq!(s.y, Column::Numeric(vec![5.0, 25.0]));
        let SeriesKind::BoxplotVertical(o) = &s.kind else {
            panic!("{s:?}")
        };
        assert_eq!(o.quartiles[0], [5.0; 5]);
        // plotters' Quartiles: 17.5, 25, 32.5 and fences 1.5 IQR out.
        assert_eq!(o.quartiles[1], [-5.0, 17.5, 25.0, 32.5, 55.0]);

        let err = SeriesBinding::new(SeriesAggregate::BoxplotVertical, SqlType::Float).unwrap_err();
        assert!(
            err.message()
                .starts_with("boxplot buckets cannot be DOUBLE"),
            "{err}"
        );
        let err =
            SeriesBinding::new(SeriesAggregate::BoxplotHorizontal, SqlType::Timestamp).unwrap_err();
        assert!(
            err.message()
                .starts_with("boxplot buckets cannot be TIMESTAMP"),
            "{err}"
        );
        let h = SeriesBinding::new(SeriesAggregate::BoxplotHorizontal, SqlType::Integer).unwrap();
        let s = push_all(&h, &[(XValue::Integer(7), &[1.0])]);
        assert_eq!(
            (s.x.clone(), s.y.clone()),
            (Column::Numeric(vec![1.0]), Column::Integer(vec![7]))
        );
        assert_eq!(s.bucket_axis(), Some(crate::spec::Axis::Y));
    }

    /// The five numbers stored per box, fed back through `Quartiles::new`, give the same
    /// quartiles exactly (the 25th, 50th and 75th percentile of five sorted values are the
    /// second, third and fourth), but the fences only within one `f32` rounding step, since
    /// plotters recomputes them from the rounded quartiles. The renderer therefore draws the
    /// stored numbers rather than rebuilt ones.
    #[test]
    fn quartile_values_round_trip() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 10_000) as f64 / 7.0 - 500.0
        };
        for n in 1..60 {
            let values: Vec<f64> = (0..n).map(|_| next()).collect();
            let q = quartile_values(&values);
            let again = quartile_values(&q);
            assert_eq!(q[1..4], again[1..4], "quartiles of {values:?}");
            for i in [0, 4] {
                let ulp = f64::from(f32::EPSILON) * q[i].abs().max(1.0);
                assert!((q[i] - again[i]).abs() <= ulp, "fence {i} of {values:?}");
            }
        }
        // Whole numbers round-trip exactly.
        let q = quartile_values(&[3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0]);
        assert_eq!(quartile_values(&q), q);
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
