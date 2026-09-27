//! `chart()` and the methods on `CHART`, `MESH` and `SERIES_LABELS`, `font()` and `color()` for
//! `FONT`, `mix`, and the output functions `to_svg` and `to_png`.

use duckers_chart::spec::MeshSetting;
use duckers_chart::{
    Chart, Font, Mesh, Series, SeriesLabels, Value, image_size, mix, to_png, to_svg,
};

use super::args::{Args, check_range_bound, scalar};
use crate::capi::{Extension, LogicalType, Result, ScalarFunction};
use crate::types::Types;

pub fn register(ext: &Extension<'_>, t: &Types) -> Result<()> {
    register_fonts(ext, t)?;
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

/// `font(family, size [, style])` (`FontDesc::new`) and `color(font, color)`.
fn register_fonts(ext: &Extension<'_>, t: &Types) -> Result<()> {
    ext.register_scalar(
        scalar("font", &t.font, |a| {
            Ok(a.check(Font::new(a.str(0)?, a.i64(1)?, None))?.encode())
        })
        .param("family", &t.varchar)
        .param("size", &t.bigint),
    )?;
    ext.register_scalar(
        scalar("font", &t.font, |a| {
            Ok(a.check(Font::new(a.str(0)?, a.i64(1)?, Some(a.str(2)?)))?
                .encode())
        })
        .param("family", &t.varchar)
        .param("size", &t.bigint)
        .param("style", &t.varchar),
    )?;
    ext.register_scalar(
        scalar("color", &t.font, |a| {
            let font: Font = a.value(0)?;
            Ok(a.check(font.color(a.str(1)?))?.encode())
        })
        .param("font", &t.font)
        .param("color", &t.varchar),
    )
}

/// A `CHART` method with one px argument.
type ChartPx = fn(Chart, i64) -> duckers_chart::Result<Chart>;

/// A method on `CHART` that takes one `BIGINT` px argument and returns the chart.
fn chart_px(t: &Types, name: &'static str, f: ChartPx) -> ScalarFunction {
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
    ext.register_scalar(
        scalar("caption", &t.chart, |a| {
            let chart: Chart = a.value(0)?;
            let font: Font = a.value(2)?;
            Ok(chart.caption_font(a.str(1)?, font).encode())
        })
        .param("chart", &t.chart)
        .param("text", &t.varchar)
        .param("font", &t.font),
    )?;
    let sizes: [(&'static str, ChartPx); 11] = [
        ("margin", Chart::margin),
        ("margin_top", Chart::margin_top),
        ("margin_bottom", Chart::margin_bottom),
        ("margin_left", Chart::margin_left),
        ("margin_right", Chart::margin_right),
        ("x_label_area_size", Chart::x_label_area_size),
        ("y_label_area_size", Chart::y_label_area_size),
        ("top_x_label_area_size", Chart::top_x_label_area_size),
        ("right_y_label_area_size", Chart::right_y_label_area_size),
        ("set_all_label_area_size", Chart::set_all_label_area_size),
        (
            "set_left_and_bottom_label_area_size",
            Chart::set_left_and_bottom_label_area_size,
        ),
    ];
    for (name, f) in sizes {
        ext.register_scalar(chart_px(t, name, f))?;
    }
    ext.register_scalar(range(t, "x_range", true))?;
    ext.register_scalar(range(t, "y_range", false))?;
    for (name, f) in [
        (
            "x_log_scale",
            Chart::x_log_scale as fn(Chart, Option<f64>) -> duckers_chart::Result<Chart>,
        ),
        ("y_log_scale", Chart::y_log_scale),
    ] {
        ext.register_scalar(
            scalar(name, &t.chart, move |a| {
                let chart: Chart = a.value(0)?;
                Ok(a.check(f(chart, None))?.encode())
            })
            .param("chart", &t.chart),
        )?;
        ext.register_scalar(
            scalar(name, &t.chart, move |a| {
                let chart: Chart = a.value(0)?;
                Ok(a.check(f(chart, Some(a.f64(1)?)))?.encode())
            })
            .param("chart", &t.chart)
            .param("base", &t.double),
        )?;
    }
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

/// A method on a duckers value that returns a value of the same type: `receiver` is the SQL
/// name of the first parameter, `params` the rest.
fn method<T: Value + 'static>(
    ty: &LogicalType,
    receiver: &str,
    name: &'static str,
    params: &[(&str, &LogicalType)],
    f: fn(T, &Args<'_, '_>) -> Result<T>,
) -> ScalarFunction {
    let mut function = scalar(name, ty, move |a| {
        let value: T = a.value(0)?;
        Ok(f(value, a)?.encode())
    })
    .param(receiver, ty);
    for (param, ty) in params {
        function = function.param(param, ty);
    }
    function
}

/// A text-style method (`IntoTextStyle`), registered twice: with a `BIGINT` size, which is
/// sans-serif of that size, and with a `FONT`.
fn text_style_methods<T: Value + 'static>(
    ext: &Extension<'_>,
    t: &Types,
    ty: &LogicalType,
    receiver: &str,
    name: &'static str,
    f: fn(T, Font) -> T,
) -> Result<()> {
    let sized = scalar(name, ty, move |a| {
        let value: T = a.value(0)?;
        let font = a.check(Font::sans_serif(a.i64(1)?))?;
        Ok(f(value, font).encode())
    })
    .param(receiver, ty)
    .param("size", &t.bigint);
    ext.register_scalar(sized)?;
    let font = scalar(name, ty, move |a| {
        let value: T = a.value(0)?;
        let font: Font = a.value(1)?;
        Ok(f(value, font).encode())
    })
    .param(receiver, ty)
    .param("font", &t.font);
    ext.register_scalar(font)
}

fn register_mesh(ext: &Extension<'_>, t: &Types) -> Result<()> {
    type MeshMethod = fn(Mesh, &Args<'_, '_>) -> Result<Mesh>;
    let mesh = |name: &'static str, params: &[(&str, &LogicalType)], f: MeshMethod| {
        method(&t.mesh, "mesh", name, params, f)
    };
    let text = [("text", &t.varchar)];
    ext.register_scalar(mesh("x_desc", &text, |m, a| Ok(m.x_desc(a.str(1)?))))?;
    ext.register_scalar(mesh("y_desc", &text, |m, a| Ok(m.y_desc(a.str(1)?))))?;
    let format = [("format", &t.varchar)];
    ext.register_scalar(mesh("x_label_formatter", &format, |m, a| {
        a.check(m.x_label_formatter(a.str(1)?))
    }))?;
    ext.register_scalar(mesh("y_label_formatter", &format, |m, a| {
        a.check(m.y_label_formatter(a.str(1)?))
    }))?;

    type TextStyleMethod = fn(Mesh, Font) -> Mesh;
    let text_styles: [(&'static str, TextStyleMethod); 4] = [
        ("axis_desc_style", Mesh::axis_desc_style),
        ("label_style", Mesh::label_style),
        ("x_label_style", Mesh::x_label_style),
        ("y_label_style", Mesh::y_label_style),
    ];
    for (name, f) in text_styles {
        text_style_methods(ext, t, &t.mesh, "mesh", name, f)?;
    }

    let n = [("n", &t.bigint)];
    ext.register_scalar(mesh("x_labels", &n, |m, a| a.check(m.x_labels(a.i64(1)?))))?;
    ext.register_scalar(mesh("y_labels", &n, |m, a| a.check(m.y_labels(a.i64(1)?))))?;
    ext.register_scalar(mesh("x_max_light_lines", &n, |m, a| {
        a.check(m.x_max_light_lines(a.i64(1)?))
    }))?;
    ext.register_scalar(mesh("y_max_light_lines", &n, |m, a| {
        a.check(m.y_max_light_lines(a.i64(1)?))
    }))?;
    ext.register_scalar(mesh("max_light_lines", &n, |m, a| {
        a.check(m.max_light_lines(a.i64(1)?))
    }))?;

    let px = [("px", &t.bigint)];
    ext.register_scalar(mesh("x_label_offset", &px, |m, a| {
        a.check(m.x_label_offset(a.i64(1)?))
    }))?;
    ext.register_scalar(mesh("y_label_offset", &px, |m, a| {
        a.check(m.y_label_offset(a.i64(1)?))
    }))?;
    ext.register_scalar(mesh("set_all_tick_mark_size", &px, |m, a| {
        a.check(m.set_all_tick_mark_size(a.i64(1)?))
    }))?;
    ext.register_scalar(mesh(
        "set_tick_mark_size",
        &[("position", &t.varchar), ("px", &t.bigint)],
        |m, a| a.check(m.set_tick_mark_size(a.str(1)?, a.i64(2)?)),
    ))?;

    type LineMethod = fn(Mesh, &str, Option<i64>) -> duckers_chart::Result<Mesh>;
    let lines: [(&'static str, LineMethod); 3] = [
        ("light_line_style", Mesh::light_line_style),
        ("bold_line_style", Mesh::bold_line_style),
        ("axis_style", Mesh::axis_style),
    ];
    for (name, f) in lines {
        let color = scalar(name, &t.mesh, move |a| {
            let m: Mesh = a.value(0)?;
            Ok(a.check(f(m, a.str(1)?, None))?.encode())
        })
        .param("mesh", &t.mesh)
        .param("color", &t.varchar);
        ext.register_scalar(color)?;
        let width = scalar(name, &t.mesh, move |a| {
            let m: Mesh = a.value(0)?;
            Ok(a.check(f(m, a.str(1)?, Some(a.i64(2)?)))?.encode())
        })
        .param("mesh", &t.mesh)
        .param("color", &t.varchar)
        .param("stroke_width", &t.bigint);
        ext.register_scalar(width)?;
    }

    let disables = [
        ("disable_x_mesh", MeshSetting::DisableXMesh),
        ("disable_y_mesh", MeshSetting::DisableYMesh),
        ("disable_mesh", MeshSetting::DisableMesh),
        ("disable_x_axis", MeshSetting::DisableXAxis),
        ("disable_y_axis", MeshSetting::DisableYAxis),
        ("disable_axes", MeshSetting::DisableAxes),
    ];
    for (name, setting) in disables {
        ext.register_scalar(
            scalar(name, &t.mesh, move |a| {
                let m: Mesh = a.value(0)?;
                Ok(m.set(setting.clone()).encode())
            })
            .param("mesh", &t.mesh),
        )?;
    }

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
