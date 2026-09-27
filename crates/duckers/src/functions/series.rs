//! The series aggregates (`line_series`, `point_series`, `histogram_vertical`,
//! `histogram_horizontal`, `area_series`, `dashed_line_series`, `error_bar_vertical`,
//! `error_bar_horizontal`, `candle_stick`, `boxplot_vertical`, `boxplot_horizontal`) and the
//! methods on `SERIES`.

use duckers_chart::{Accumulator, Key, Series, SeriesAggregate, SeriesBinding, Value};

use super::args::{XReader, chart_error, key_label, scalar, sort_key};
use crate::capi::{
    Aggregate, AggregateFunction, AggregateInput, Bind, Error, Extension, OutputVector, Result,
};
use crate::types::Types;

/// The position of the first argument of every series aggregate; its value arguments follow,
/// then `key` and `order_by`.
const X: usize = 0;

pub fn register(ext: &Extension<'_>, types: &Types) -> Result<()> {
    let ctx = ext.context();
    for aggregate in SeriesAggregate::ALL {
        let mut function = AggregateFunction::<SeriesState>::new(aggregate.name(), &types.any)
            .param(aggregate.x_name(), &types.any);
        for value in aggregate.value_names() {
            function = function.param(value, &types.any);
        }
        // `key` and `order_by` default to a typed NULL: an ANY parameter cannot default to an
        // untyped one. Bind tells "not given" by the constant NULL.
        ext.register_aggregate(
            function
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
    /// The number of value arguments after `x`.
    values: usize,
    /// Called with `key :=`: the result is `SERIES[]`.
    keyed: bool,
    /// Called with `order_by :=`.
    ordered: bool,
}

impl SeriesBind {
    fn key_index(&self) -> usize {
        X + self.values + 1
    }

    fn order_by_index(&self) -> usize {
        X + self.values + 2
    }
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
        let aggregate = SeriesAggregate::ALL
            .into_iter()
            .find(|a| a.name() == b.function_name())
            .ok_or_else(|| Error::internal(format!("unknown aggregate {}", b.function_name())))?;
        let (name, x_name) = (aggregate.name(), aggregate.x_name());
        let x_type = b.arg_type(X)?;
        let (x, sql_type) = XReader::for_type(x_type.id()).ok_or_else(|| {
            let expected = if aggregate.is_histogram() {
                "VARCHAR, a number or DATE"
            } else if aggregate.is_boxplot() {
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
        let value_names = aggregate.value_names();
        for (i, value) in value_names.iter().enumerate() {
            let ty = b.arg_type(X + 1 + i)?;
            if !ty.id().is_numeric() {
                return Err(Error::binder(format!(
                    "{name}: {value} must be a number, got {}",
                    ty.to_text()
                )));
            }
        }
        let values = value_names.len();
        let keyed = given(b, X + values + 1);
        let ordered = given(b, X + values + 2);
        let returns = b
            .context()
            .type_from_text(if keyed { "SERIES[]" } else { "SERIES" })?;
        b.set_return_type(&returns)?;
        Ok(SeriesBind {
            name,
            binding,
            x,
            values,
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
        let x = input.arg(X);
        if !x.is_valid(row) {
            return Ok(());
        }
        let mut values = [None; 4];
        for (i, slot) in values.iter_mut().take(bind.values).enumerate() {
            let v = input.arg(X + 1 + i);
            if !v.is_valid(row) {
                return Ok(());
            }
            *slot = Some(v.f64(row)?);
        }
        let Some(x) = bind.x.read(x, row)? else {
            return Ok(());
        };
        let key = if bind.keyed {
            let k = input.arg(bind.key_index());
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
            Some(sort_key(input.arg(bind.order_by_index()), row)?)
        } else {
            None
        };
        self.acc
            .push_values(
                &bind.binding,
                Some(x),
                &values[..bind.values],
                key,
                order_by,
            )
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

/// The methods on `SERIES`: `style`, `stroke_width`, `filled`, `label` and the kind-specific ones.
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
    )?;
    ext.register_scalar(
        scalar("marker", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.marker(a.str(1)?))?.encode())
        })
        .param("series", &t.series)
        .param("name", &t.varchar),
    )?;
    // `margin` is also a method on CHART and SERIES_LABELS; DuckDB picks the overload by the
    // type of the first argument.
    ext.register_scalar(
        scalar("margin", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.margin(a.i64(1)?))?.encode())
        })
        .param("series", &t.series)
        .param("px", &t.bigint),
    )?;
    ext.register_scalar(
        scalar("baseline", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.baseline(a.f64(1)?))?.encode())
        })
        .param("series", &t.series)
        .param("value", &t.double),
    )?;
    // `border_style` is also a method on SERIES_LABELS; the first argument picks the overload.
    ext.register_scalar(
        scalar("border_style", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.border_style(a.str(1)?, None))?.encode())
        })
        .param("series", &t.series)
        .param("color", &t.varchar),
    )?;
    ext.register_scalar(
        scalar("border_style", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.border_style(a.str(1)?, Some(a.i64(2)?)))?
                .encode())
        })
        .param("series", &t.series)
        .param("color", &t.varchar)
        .param("stroke_width", &t.bigint),
    )?;
    type PxMethod = fn(Series, i64) -> duckers_chart::Result<Series>;
    let px_methods: [(&'static str, PxMethod); 2] =
        [("spacing", Series::spacing), ("width", Series::width)];
    for (name, f) in px_methods {
        ext.register_scalar(
            scalar(name, &t.series, move |a| {
                let s: Series = a.value(0)?;
                Ok(a.check(f(s, a.i64(1)?))?.encode())
            })
            .param("series", &t.series)
            .param("px", &t.bigint),
        )?;
    }
    type ColorMethod = fn(Series, &str) -> duckers_chart::Result<Series>;
    let color_methods: [(&'static str, ColorMethod); 2] = [
        ("gain_style", Series::gain_style),
        ("loss_style", Series::loss_style),
    ];
    for (name, f) in color_methods {
        ext.register_scalar(
            scalar(name, &t.series, move |a| {
                let s: Series = a.value(0)?;
                Ok(a.check(f(s, a.str(1)?))?.encode())
            })
            .param("series", &t.series)
            .param("color", &t.varchar),
        )?;
    }
    ext.register_scalar(
        scalar("step", &t.series, |a| {
            let s: Series = a.value(0)?;
            Ok(a.check(s.step(a.f64(1)?))?.encode())
        })
        .param("series", &t.series)
        .param("s", &t.double),
    )
}
