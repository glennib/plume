//! The series aggregates (`line_series`, `point_series`, `histogram_vertical`) and the methods
//! on `SERIES`.

use duckers_chart::{Accumulator, Key, Series, SeriesAggregate, SeriesBinding, Value};

use super::args::{XReader, chart_error, key_label, scalar, sort_key};
use crate::capi::{
    Aggregate, AggregateFunction, AggregateInput, Bind, Error, Extension, OutputVector, Result,
};
use crate::types::Types;

/// Argument positions of every series aggregate.
const X: usize = 0;
const Y: usize = 1;
const KEY: usize = 2;
const ORDER_BY: usize = 3;

/// The SQL name of each aggregate and the names of its two value parameters.
const AGGREGATES: [(SeriesAggregate, &str, &str, &str); 3] = [
    (SeriesAggregate::LineSeries, "line_series", "x", "y"),
    (SeriesAggregate::PointSeries, "point_series", "x", "y"),
    (
        SeriesAggregate::Histogram,
        "histogram_vertical",
        "bucket",
        "value",
    ),
];

pub fn register(ext: &Extension<'_>, types: &Types) -> Result<()> {
    let ctx = ext.context();
    for (_, name, x, y) in AGGREGATES {
        // `key` and `order_by` default to a typed NULL: an ANY parameter cannot default to an
        // untyped one. Bind tells "not given" by the constant NULL.
        ext.register_aggregate(
            AggregateFunction::<SeriesState>::new(name, &types.any)
                .param(x, &types.any)
                .param(y, &types.any)
                .named("key", &types.any, ctx.null(&types.varchar)?)
                .named("order_by", &types.any, ctx.null(&types.varchar)?),
        )?;
    }
    register_methods(ext, types)
}

/// Bind data of a series aggregate call.
pub struct SeriesBind {
    name: &'static str,
    binding: SeriesBinding,
    x: XReader,
    /// Called with `key :=`: the result is `SERIES[]`.
    keyed: bool,
    /// Called with `order_by :=`.
    ordered: bool,
}

/// The state of a series aggregate for one group.
pub struct SeriesState {
    acc: Accumulator,
}

/// Whether the named argument `index` was given: anything but the constant NULL default.
fn given(b: &Bind<'_>, index: usize) -> bool {
    match b.arg_value(index) {
        Ok(v) => !v.is_null(),
        Err(_) => true,
    }
}

impl Aggregate for SeriesState {
    type BindData = SeriesBind;

    fn bind(b: &mut Bind<'_>) -> Result<SeriesBind> {
        let (aggregate, name, x_name, y_name) = AGGREGATES
            .into_iter()
            .find(|(_, name, ..)| *name == b.function_name())
            .ok_or_else(|| Error::internal(format!("unknown aggregate {}", b.function_name())))?;
        let x_type = b.arg_type(X)?;
        let histogram = aggregate == SeriesAggregate::Histogram;
        let (x, sql_type) = XReader::for_type(x_type.id()).ok_or_else(|| {
            let expected = if histogram {
                "VARCHAR, an integer or DATE"
            } else {
                "a number, DATE, TIMESTAMP or VARCHAR"
            };
            Error::binder(format!(
                "{name}: {x_name} must be {expected}, got {}",
                x_type.to_text()
            ))
        })?;
        let binding = SeriesBinding::new(aggregate, sql_type).map_err(|e| {
            let e = chart_error(name, e);
            Error::binder(e.message())
        })?;
        let y_type = b.arg_type(Y)?;
        if !y_type.id().is_numeric() {
            return Err(Error::binder(format!(
                "{name}: {y_name} must be a number, got {}",
                y_type.to_text()
            )));
        }
        let keyed = given(b, KEY);
        let ordered = given(b, ORDER_BY);
        let returns = b
            .context()
            .type_from_text(if keyed { "SERIES[]" } else { "SERIES" })?;
        b.set_return_type(&returns)?;
        Ok(SeriesBind {
            name,
            binding,
            x,
            keyed,
            ordered,
        })
    }

    fn init(_: &SeriesBind) -> Self {
        SeriesState {
            acc: Accumulator::new(),
        }
    }

    fn update(&mut self, bind: &SeriesBind, input: &AggregateInput<'_>, row: usize) -> Result<()> {
        let (x, y) = (input.arg(X), input.arg(Y));
        if !x.is_valid(row) || !y.is_valid(row) {
            return Ok(());
        }
        let Some(x) = bind.x.read(x, row)? else {
            return Ok(());
        };
        let y = y.f64(row)?;
        let key = if bind.keyed {
            let k = input.arg(KEY);
            let sort = sort_key(k, row)?;
            // The label is the key's text, needed once per key.
            let label = if self.acc.contains_key(&sort) {
                None
            } else {
                key_label(k, row)?
            };
            Some(Key { sort, label })
        } else {
            None
        };
        let order_by = if bind.ordered {
            Some(sort_key(input.arg(ORDER_BY), row)?)
        } else {
            None
        };
        self.acc
            .push(&bind.binding, Some(x), Some(y), key, order_by)
            .map_err(|e| chart_error(bind.name, e))
    }

    fn combine(&mut self, _: &SeriesBind, other: &Self) -> Result<()> {
        self.acc.combine(other.acc.clone());
        Ok(())
    }

    fn finalize(
        &mut self,
        bind: &SeriesBind,
        out: &mut OutputVector<'_>,
        row: usize,
    ) -> Result<()> {
        let acc = std::mem::take(&mut self.acc);
        if bind.keyed {
            let series = acc
                .finish_keyed(&bind.binding)
                .map_err(|e| chart_error(bind.name, e))?;
            let encoded: Vec<Vec<u8>> = series.iter().map(Series::encode).collect();
            out.set_list_bytes(row, &encoded)
        } else {
            let series = acc
                .finish(&bind.binding)
                .map_err(|e| chart_error(bind.name, e))?;
            out.set_bytes(row, &series.encode())
        }
    }
}

/// `style`, `stroke_width`, `filled`, `label`, `point_size` and `size` on `SERIES`.
fn register_methods(ext: &Extension<'_>, t: &Types) -> Result<()> {
    ext.register_scalar(
        scalar("style", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.style(a.str(1)?, None))?.encode())
        })
        .param("series", &t.series)
        .param("color", &t.varchar),
    )?;
    ext.register_scalar(
        scalar("style", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.style(a.str(1)?, Some(a.i64(2)?)))?.encode())
        })
        .param("series", &t.series)
        .param("color", &t.varchar)
        .param("stroke_width", &t.bigint),
    )?;
    ext.register_scalar(
        scalar("stroke_width", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.stroke_width(a.i64(1)?))?.encode())
        })
        .param("series", &t.series)
        .param("px", &t.bigint),
    )?;
    ext.register_scalar(
        scalar("filled", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.filled())?.encode())
        })
        .param("series", &t.series),
    )?;
    ext.register_scalar(
        scalar("label", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(s.label(a.str(1)?).encode())
        })
        .param("series", &t.series)
        .param("text", &t.varchar),
    )?;
    ext.register_scalar(
        scalar("point_size", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.point_size(a.i64(1)?))?.encode())
        })
        .param("series", &t.series)
        .param("px", &t.bigint),
    )?;
    ext.register_scalar(
        scalar("size", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.size(a.i64(1)?))?.encode())
        })
        .param("series", &t.series)
        .param("px", &t.bigint),
    )
}
