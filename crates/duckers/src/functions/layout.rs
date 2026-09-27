//! Layout: `split_evenly` and `titled` over `CHART` values, the `pie` aggregate, and the
//! methods on a pie `CHART` (`start_angle`, `label_style`, `percentages`, `label_offset`,
//! `radius`).

use duckers_chart::spec::Pie;
use duckers_chart::{Font, PieAccumulator, Root, Value};

use super::args::{Args, key_label, scalar, sort_key};
use super::chart::text_style_methods;
use crate::capi::{
    Aggregate, AggregateFunction, AggregateInput, Bind, Error, Extension, LogicalType,
    OutputVector, Result,
};
use crate::types::Types;

pub fn register(ext: &Extension<'_>, t: &Types) -> Result<()> {
    ext.register_scalar(
        scalar("split_evenly", &t.chart, |a| {
            let list = a.arg(0);
            let elements = list.list_child()?;
            let mut cells = Vec::new();
            // A NULL element is a blank cell.
            for i in list.list_entry(a.row())? {
                cells.push(if elements.is_valid(i) {
                    Some(a.check(Root::decode(elements.bytes(i)?))?)
                } else {
                    None
                });
            }
            Ok(a.check(Root::split_evenly(cells, a.i64(1)?, a.i64(2)?))?
                .encode())
        })
        .param("charts", &t.chart_list)
        .param("rows", &t.bigint)
        .param("cols", &t.bigint),
    )?;
    ext.register_scalar(
        scalar("titled", &t.chart, |a| {
            let root: Root = a.value(0)?;
            Ok(a.check(root.titled(a.str(1)?, None))?.encode())
        })
        .param("chart", &t.chart)
        .param("text", &t.varchar),
    )?;
    ext.register_scalar(
        scalar("titled", &t.chart, |a| {
            let root: Root = a.value(0)?;
            Ok(a.check(root.titled(a.str(1)?, Some(a.i64(2)?)))?.encode())
        })
        .param("chart", &t.chart)
        .param("text", &t.varchar)
        .param("size", &t.bigint),
    )?;
    ext.register_scalar(
        scalar("titled", &t.chart, |a| {
            let root: Root = a.value(0)?;
            let font: Font = a.value(2)?;
            Ok(root.titled_font(a.str(1)?, font).encode())
        })
        .param("chart", &t.chart)
        .param("text", &t.varchar)
        .param("font", &t.font),
    )?;
    register_pie_methods(ext, t)?;
    ext.register_aggregate(
        AggregateFunction::<PieState>::new("pie", &t.chart)
            .param("size", &t.any)
            .param("label", &t.any)
            .named("order_by", &t.any, ext.context().null(&t.varchar)?),
    )
}

/// A method on a pie `CHART`: the root must be a pie.
fn pie_method(
    t: &Types,
    name: &'static str,
    param: (&'static str, &LogicalType),
    f: fn(Pie, &Args<'_, '_>) -> Result<Pie>,
) -> crate::capi::ScalarFunction {
    scalar(name, &t.chart, move |a| {
        let root: Root = a.value(0)?;
        let pie = a.check(root.into_pie(name))?;
        Ok(Root::Pie(f(pie, a)?).encode())
    })
    .param("chart", &t.chart)
    .param(param.0, param.1)
}

fn register_pie_methods(ext: &Extension<'_>, t: &Types) -> Result<()> {
    ext.register_scalar(pie_method(
        t,
        "start_angle",
        ("degrees", &t.double),
        |p, a| a.check(p.start_angle(a.f64(1)?)),
    ))?;
    ext.register_scalar(pie_method(t, "label_offset", ("px", &t.bigint), |p, a| {
        a.check(p.label_offset(a.i64(1)?))
    }))?;
    ext.register_scalar(pie_method(t, "radius", ("px", &t.bigint), |p, a| {
        a.check(p.radius(a.i64(1)?))
    }))?;
    // `label_style` is also a method on MESH; the first argument picks the overload.
    text_style_methods(ext, t, &t.chart, "chart", "label_style", |r: Root, f| {
        Ok(Root::Pie(r.into_pie("label_style")?.label_style(f)))
    })?;
    text_style_methods(ext, t, &t.chart, "chart", "percentages", |r: Root, f| {
        Ok(Root::Pie(r.into_pie("percentages")?.percentages(f)))
    })
}

/// Bind data of a `pie` call.
pub struct PieBind {
    /// Called with `order_by :=`.
    ordered: bool,
}

/// The state of the `pie` aggregate for one group.
pub struct PieState {
    acc: PieAccumulator,
}

const SIZE: usize = 0;
const LABEL: usize = 1;
const ORDER_BY: usize = 2;

impl Aggregate for PieState {
    type BindData = PieBind;

    fn bind(b: &mut Bind<'_>) -> Result<PieBind> {
        let ty = b.arg_type(SIZE)?;
        if !ty.id().is_numeric() {
            return Err(Error::binder(format!(
                "pie: size must be a number, got {}",
                ty.to_text()
            )));
        }
        let ordered = match b.arg_value(ORDER_BY) {
            Ok(v) => !v.is_null(),
            Err(_) => true,
        };
        Ok(PieBind { ordered })
    }

    fn init(_: &PieBind) -> Self {
        PieState {
            acc: PieAccumulator::new(),
        }
    }

    fn update(&mut self, bind: &PieBind, input: &AggregateInput<'_>, row: usize) -> Result<()> {
        let size = input.arg(SIZE);
        if !size.is_valid(row) {
            return Ok(());
        }
        let size = size.f64(row)?;
        if !(size.is_finite() && size > 0.0) {
            return Ok(());
        }
        let label = input.arg(LABEL);
        let order_by = if bind.ordered {
            Some(sort_key(input.arg(ORDER_BY), row)?)
        } else {
            None
        };
        self.acc.push(
            Some(size),
            sort_key(label, row)?,
            key_label(label, row)?,
            order_by,
        );
        Ok(())
    }

    fn combine(&mut self, _: &PieBind, other: &Self) -> Result<()> {
        self.acc.combine(other.acc.clone());
        Ok(())
    }

    fn finalize(&mut self, _: &PieBind, out: &mut OutputVector<'_>, row: usize) -> Result<()> {
        let acc = std::mem::take(&mut self.acc);
        out.set_bytes(row, &acc.finish().encode())
    }
}
