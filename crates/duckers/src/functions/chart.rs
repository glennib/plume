//! `chart()` and the methods on `CHART`, `MESH` and `SERIES_LABELS`, `mix`, and the output
//! functions `to_svg` and `to_png`.

use duckers_chart::{Chart, Mesh, Series, SeriesLabels, Value, image_size, mix, to_png, to_svg};

use super::args::{check_range_bound, scalar};
use crate::capi::{Extension, LogicalType, Result, ScalarFunction};
use crate::types::Types;

pub fn register(ext: &Extension<'_>, t: &Types) -> Result<()> {
    register_builder(ext, t)?;
    register_mesh(ext, t)?;
    register_series_labels(ext, t)?;
    register_output(ext, t)?;
    ext.register_scalar(
        scalar("mix", &t.varchar, |a| a.check(mix(a.str(0)?, a.f64(1)?)))
            .param("color", &t.varchar)
            .param("alpha", &t.double),
    )
}

/// A method on `CHART` that takes one `BIGINT` px argument and returns the chart.
fn chart_px(
    t: &Types,
    name: &'static str,
    f: fn(Chart, i64) -> duckers_chart::Result<Chart>,
) -> ScalarFunction {
    scalar(name, &t.chart, move |a| {
        let chart: Chart = a.value(0)?;
        Ok(a.check(f(chart, a.i64(1)?))?.encode())
    })
    .param("chart", &t.chart)
    .param("px", &t.bigint)
}

/// `x_range(lo, hi)` or `y_range(lo, hi)`. The bounds are `ANY`, so every numeric, date and
/// timestamp type binds; bind checks them and the method checks them against the axis.
fn range(t: &Types, name: &'static str, x: bool) -> ScalarFunction {
    scalar(name, &t.chart, move |a| {
        let chart: Chart = a.value(0)?;
        let (lo, hi) = (a.range_bound(1)?, a.range_bound(2)?);
        let chart = if x {
            chart.x_range(lo, hi)
        } else {
            chart.y_range(lo, hi)
        };
        Ok(a.check(chart)?.encode())
    })
    .param("chart", &t.chart)
    .param("lo", &t.any)
    .param("hi", &t.any)
    .bind(|b| {
        check_range_bound(b, 1, "lo")?;
        check_range_bound(b, 2, "hi")?;
        Ok(None)
    })
}

fn register_builder(ext: &Extension<'_>, t: &Types) -> Result<()> {
    ext.register_scalar(scalar("chart", &t.chart, |_| Ok(Chart::new().encode())))?;
    ext.register_scalar(
        scalar("caption", &t.chart, |a| {
            let chart: Chart = a.value(0)?;
            Ok(a.check(chart.caption(a.str(1)?, None))?.encode())
        })
        .param("chart", &t.chart)
        .param("text", &t.varchar),
    )?;
    ext.register_scalar(
        scalar("caption", &t.chart, |a| {
            let chart: Chart = a.value(0)?;
            Ok(a.check(chart.caption(a.str(1)?, Some(a.i64(2)?)))?.encode())
        })
        .param("chart", &t.chart)
        .param("text", &t.varchar)
        .param("size", &t.bigint),
    )?;
    ext.register_scalar(chart_px(t, "margin", Chart::margin))?;
    ext.register_scalar(chart_px(t, "x_label_area_size", Chart::x_label_area_size))?;
    ext.register_scalar(chart_px(t, "y_label_area_size", Chart::y_label_area_size))?;
    ext.register_scalar(range(t, "x_range", true))?;
    ext.register_scalar(range(t, "y_range", false))?;
    // `root.fill(&color)`. DuckDB has a window function `fill`, and a scalar cannot share its
    // name, so the plotters receiver qualifies the name.
    ext.register_scalar(
        scalar("root_fill", &t.chart, |a| {
            let chart: Chart = a.value(0)?;
            Ok(a.check(chart.fill(a.str(1)?))?.encode())
        })
        .param("chart", &t.chart)
        .param("color", &t.varchar),
    )?;
    ext.register_scalar(
        scalar("draw_series", &t.chart, |a| {
            let chart: Chart = a.value(0)?;
            let series: Series = a.value(1)?;
            Ok(a.check(chart.draw_series(series))?.encode())
        })
        .param("chart", &t.chart)
        .param("series", &t.series),
    )?;
    ext.register_scalar(
        scalar("draw_series", &t.chart, |a| {
            let chart: Chart = a.value(0)?;
            let list = a.arg(1);
            let elements = list.list_child()?;
            let mut series = Vec::new();
            // NULL elements draw nothing.
            for i in list.list_entry(a.row())? {
                if elements.is_valid(i) {
                    series.push(a.check(Series::decode(elements.bytes(i)?))?);
                }
            }
            Ok(a.check(chart.draw_series_list(series))?.encode())
        })
        .param("chart", &t.chart)
        .param("series", &t.series_list),
    )?;
    ext.register_scalar(
        scalar("configure_mesh", &t.mesh, |a| {
            let chart: Chart = a.value(0)?;
            Ok(chart.configure_mesh().encode())
        })
        .param("chart", &t.chart),
    )?;
    ext.register_scalar(
        scalar("configure_series_labels", &t.series_labels, |a| {
            let chart: Chart = a.value(0)?;
            Ok(chart.configure_series_labels().encode())
        })
        .param("chart", &t.chart),
    )
}

/// A method on `MESH` that takes one `VARCHAR` and returns the mesh.
fn mesh_text(t: &Types, name: &'static str, f: fn(Mesh, &str) -> Mesh) -> ScalarFunction {
    scalar(name, &t.mesh, move |a| {
        let mesh: Mesh = a.value(0)?;
        Ok(f(mesh, a.str(1)?).encode())
    })
    .param("mesh", &t.mesh)
    .param("text", &t.varchar)
}

fn register_mesh(ext: &Extension<'_>, t: &Types) -> Result<()> {
    ext.register_scalar(mesh_text(t, "x_desc", |m, s| m.x_desc(s)))?;
    ext.register_scalar(mesh_text(t, "y_desc", |m, s| m.y_desc(s)))?;
    ext.register_scalar(
        scalar("draw", &t.chart, |a| {
            let mesh: Mesh = a.value(0)?;
            Ok(mesh.draw().encode())
        })
        .param("mesh", &t.mesh),
    )
}

fn register_series_labels(ext: &Extension<'_>, t: &Types) -> Result<()> {
    let labels = |name: &'static str,
                  params: &[(&str, &LogicalType)],
                  f: fn(SeriesLabels, &super::args::Args<'_, '_>) -> Result<SeriesLabels>|
     -> ScalarFunction {
        let mut function = scalar(name, &t.series_labels, move |a| {
            let labels: SeriesLabels = a.value(0)?;
            Ok(f(labels, a)?.encode())
        })
        .param("series_labels", &t.series_labels);
        for (param, ty) in params {
            function = function.param(param, ty);
        }
        function
    };
    ext.register_scalar(labels("position", &[("name", &t.varchar)], |l, a| {
        a.check(l.position(a.str(1)?))
    }))?;
    ext.register_scalar(labels(
        "position",
        &[("x", &t.bigint), ("y", &t.bigint)],
        |l, a| a.check(l.position_at(a.i64(1)?, a.i64(2)?)),
    ))?;
    ext.register_scalar(labels("border_style", &[("color", &t.varchar)], |l, a| {
        a.check(l.border_style(a.str(1)?, None))
    }))?;
    ext.register_scalar(labels(
        "border_style",
        &[("color", &t.varchar), ("stroke_width", &t.bigint)],
        |l, a| a.check(l.border_style(a.str(1)?, Some(a.i64(2)?))),
    ))?;
    ext.register_scalar(labels(
        "background_style",
        &[("color", &t.varchar)],
        |l, a| a.check(l.background_style(a.str(1)?)),
    ))?;
    ext.register_scalar(
        scalar("draw", &t.chart, |a| {
            let labels: SeriesLabels = a.value(0)?;
            Ok(labels.draw().encode())
        })
        .param("series_labels", &t.series_labels),
    )
}

fn register_output(ext: &Extension<'_>, t: &Types) -> Result<()> {
    ext.register_scalar(
        scalar("to_svg", &t.varchar, |a| {
            let chart: Chart = a.value(0)?;
            let (w, h) = a.check(image_size(None, None))?;
            a.check(to_svg(&chart, w, h))
        })
        .param("chart", &t.chart),
    )?;
    ext.register_scalar(
        scalar("to_svg", &t.varchar, |a| {
            let chart: Chart = a.value(0)?;
            let (w, h) = a.check(image_size(Some(a.i64(1)?), Some(a.i64(2)?)))?;
            a.check(to_svg(&chart, w, h))
        })
        .param("chart", &t.chart)
        .param("width", &t.bigint)
        .param("height", &t.bigint),
    )?;
    ext.register_scalar(
        scalar("to_png", &t.blob, |a| {
            let chart: Chart = a.value(0)?;
            let (w, h) = a.check(image_size(None, None))?;
            a.check(to_png(&chart, w, h))
        })
        .param("chart", &t.chart),
    )?;
    ext.register_scalar(
        scalar("to_png", &t.blob, |a| {
            let chart: Chart = a.value(0)?;
            let (w, h) = a.check(image_size(Some(a.i64(1)?), Some(a.i64(2)?)))?;
            a.check(to_png(&chart, w, h))
        })
        .param("chart", &t.chart)
        .param("width", &t.bigint)
        .param("height", &t.bigint),
    )
}
