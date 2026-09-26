//! Series constructors: scalar functions that turn lists of x and y values into a
//! single-kind `CHART`.
//!
//! The lists come from DuckDB's `list()` aggregate, so the SQL decides what gets drawn and
//! in which order: `line_series(list(x ORDER BY x), list(y ORDER BY x))`. Points are drawn
//! in list order. An optional label argument, either one label for the whole series or a
//! list with one label per point, splits the points into series with legend entries.
//!
//! These are scalars and not aggregates taking `(x, y ORDER BY x)` because DuckDB's C API
//! mishandles `ORDER BY` in C API aggregates: the sorted-aggregate path hands `update` a
//! constant state vector that the C shim reads as a flat one.

use super::{
    capi::{BoxError, Column, ListColumn, chart_type_handle},
    spec::{Chart, Series, SeriesKind, XData},
};
use duckdb::{
    Connection,
    core::{DataChunkHandle, Inserter, LogicalTypeHandle, LogicalTypeId},
    vscalar::{ScalarFunctionSignature, VScalar},
    vtab::arrow::WritableVector,
};
use std::marker::PhantomData;

trait Kind: 'static {
    const KIND: SeriesKind;
    /// Element types the x list may have.
    const X_TYPES: &'static [LogicalTypeId];
    /// Whether the label overloads exist.
    const LABELS: bool;
}

struct Line;
struct Point;
struct Bar;

impl Kind for Line {
    const KIND: SeriesKind = SeriesKind::Line;
    const X_TYPES: &'static [LogicalTypeId] = &[LogicalTypeId::Double, LogicalTypeId::Timestamp];
    const LABELS: bool = true;
}

impl Kind for Point {
    const KIND: SeriesKind = SeriesKind::Point;
    const X_TYPES: &'static [LogicalTypeId] = &[LogicalTypeId::Double, LogicalTypeId::Timestamp];
    const LABELS: bool = true;
}

impl Kind for Bar {
    const KIND: SeriesKind = SeriesKind::Bar;
    const X_TYPES: &'static [LogicalTypeId] = &[LogicalTypeId::Varchar];
    // Several bar series would need side-by-side placement, which isn't implemented.
    const LABELS: bool = false;
}

enum Labels {
    None,
    /// One `VARCHAR` label for all points of the row.
    Constant(Column),
    /// A `VARCHAR[]` with one label per point.
    PerPoint(ListColumn),
}

impl XData {
    fn empty_like(id: LogicalTypeId) -> Self {
        match id {
            LogicalTypeId::Timestamp => XData::Time(Vec::new()),
            LogicalTypeId::Varchar => XData::Category(Vec::new()),
            _ => XData::F64(Vec::new()),
        }
    }

    /// Appends `column[index]`, whose type matches this variant.
    ///
    /// # Safety
    /// `index` must be in range and `column` must hold the type this variant was made for.
    unsafe fn push_from(&mut self, column: &Column, index: usize) {
        unsafe {
            match self {
                XData::F64(v) => v.push(column.get(index)),
                XData::Time(v) => v.push(column.get(index)),
                XData::Category(v) => v.push(column.string(index)),
            }
        }
    }
}

/// `<kind>_series(xs, ys [, label | labels])`.
struct SeriesFn<K>(PhantomData<K>);

impl<K: Kind> VScalar for SeriesFn<K> {
    type State = ();

    fn invoke(
        _: &(),
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), BoxError> {
        let chunk = input.get_ptr();
        let x_type = input.flat_vector(0).logical_type().child(0).id();
        let top = unsafe { Column::from_chunk(chunk) };
        let (xs, ys) = unsafe {
            (
                ListColumn::from_chunk(chunk, 0),
                ListColumn::from_chunk(chunk, 1),
            )
        };
        let labels = match input.num_columns() {
            2 => Labels::None,
            _ if input.flat_vector(2).logical_type().id() == LogicalTypeId::List => {
                Labels::PerPoint(unsafe { ListColumn::from_chunk(chunk, 2) })
            }
            _ => Labels::Constant(unsafe {
                Column::from_vector(duckdb::ffi::duckdb_data_chunk_get_vector(chunk, 2))
            }),
        };

        let mut out = output.flat_vector();
        for row in 0..input.len() {
            if !top.iter().all(|c| c.is_valid(row)) {
                out.set_null(row);
                continue;
            }
            let chart = unsafe { build::<K>(x_type, &xs, &ys, &labels, row)? };
            out.insert(row, chart.encode().as_slice());
        }
        Ok(())
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        let list = |id: LogicalTypeId| LogicalTypeHandle::list(&id.into());
        let mut out = Vec::new();
        for &x in K::X_TYPES {
            let base = || vec![list(x), list(LogicalTypeId::Double)];
            out.push(ScalarFunctionSignature::exact(base(), chart_type_handle()));
            if K::LABELS {
                let mut constant = base();
                constant.push(LogicalTypeId::Varchar.into());
                out.push(ScalarFunctionSignature::exact(
                    constant,
                    chart_type_handle(),
                ));
                let mut per_point = base();
                per_point.push(list(LogicalTypeId::Varchar));
                out.push(ScalarFunctionSignature::exact(
                    per_point,
                    chart_type_handle(),
                ));
            }
        }
        out
    }
}

/// The chart for input row `row`. A point with a NULL x or y can't be placed and is left
/// out; a NULL label makes a series without a legend entry.
///
/// # Safety
/// `row` must be in range and valid in every argument, and the columns must have the types
/// of a registered signature.
unsafe fn build<K: Kind>(
    x_type: LogicalTypeId,
    xs: &ListColumn,
    ys: &ListColumn,
    labels: &Labels,
    row: usize,
) -> Result<Chart, BoxError> {
    let (x_range, y_range) = unsafe { (xs.range(row), ys.range(row)) };
    if x_range.len() != y_range.len() {
        return Err(format!(
            "x and y lists must have the same length, got {} and {}",
            x_range.len(),
            y_range.len()
        )
        .into());
    }
    let label_range = match labels {
        Labels::PerPoint(l) => {
            let range = unsafe { l.range(row) };
            if range.len() != x_range.len() {
                return Err(format!(
                    "the label list must have one label per point, got {} labels for {} points",
                    range.len(),
                    x_range.len()
                )
                .into());
            }
            Some(range)
        }
        _ => None,
    };
    let constant_label = match labels {
        Labels::Constant(c) => Some(unsafe { c.string(row) }),
        _ => None,
    };

    let mut series: Vec<Series> = Vec::new();
    for i in 0..x_range.len() {
        let (xi, yi) = (x_range.start + i, y_range.start + i);
        if !xs.child.is_valid(xi) || !ys.child.is_valid(yi) {
            continue;
        }
        let label = match (&label_range, labels) {
            (Some(range), Labels::PerPoint(l)) => {
                let li = range.start + i;
                l.child.is_valid(li).then(|| unsafe { l.child.string(li) })
            }
            _ => constant_label.clone(),
        };
        let index = match series.iter().position(|s| s.label == label) {
            Some(index) => index,
            None => {
                series.push(Series {
                    kind: K::KIND,
                    label,
                    x: XData::empty_like(x_type),
                    y: Vec::new(),
                });
                series.len() - 1
            }
        };
        let s = &mut series[index];
        unsafe {
            s.x.push_from(&xs.child, xi);
            s.y.push(ys.child.get(yi));
        }
    }
    if series.is_empty() {
        // Keep the series (and its label) even without drawable points, so the chart is
        // still valid and renders empty axes.
        series.push(Series {
            kind: K::KIND,
            label: constant_label,
            x: XData::empty_like(x_type),
            y: Vec::new(),
        });
    }
    Ok(Chart {
        series,
        ..Default::default()
    })
}

pub fn register(con: &Connection) -> Result<(), BoxError> {
    con.register_scalar_function::<SeriesFn<Line>>("line_series")?;
    con.register_scalar_function::<SeriesFn<Point>>("point_series")?;
    con.register_scalar_function::<SeriesFn<Bar>>("bar_series")?;
    Ok(())
}
