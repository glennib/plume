//! Replaying a chart's calls on a plotters `ChartContext`.

use super::axis::{self, AxisCoord, LabelValue, bucket_sums};
use crate::color::Color;
use crate::error::{Error, Result};
use crate::label_format::{LabelFormat, strftime};
use crate::spec::{
    Axis, AxisKind, Chart, DrawOp, Font, FontStyle, LabelPosition, LineStyle, Marker, MeshSetting,
    MeshStyle, Series, SeriesKind, SeriesLabelSetting, SeriesLabelStyle, TickPosition,
};
use plotters::chart::SeriesAnno;
use plotters::coord::Shift;
use plotters::coord::ranged1d::ValueFormatter;
use plotters::prelude::*;

/// The px width of a legend glyph, inside plotters' default 30 px glyph column.
const GLYPH_WIDTH: i32 = 20;
/// Half the height of a histogram's legend rectangle.
const GLYPH_HALF_HEIGHT: i32 = 5;

static DEFAULT_MESH: MeshStyle = MeshStyle {
    settings: Vec::new(),
};
static DEFAULT_LABELS: SeriesLabelStyle = SeriesLabelStyle {
    settings: Vec::new(),
};

/// A drawing call after duckers' defaults are filled in.
enum Op<'a> {
    Mesh(&'a MeshStyle),
    /// A series and its index among the chart's series, which picks its default colour.
    Series(&'a Series, usize),
    SeriesLabels(&'a SeriesLabelStyle),
}

/// The chart's calls, plus a default mesh first if the chain never drew one, and a default
/// legend last if a series has a label and the chain never drew one.
fn resolve(chart: &Chart) -> Vec<Op<'_>> {
    let mut index = 0;
    let mut ops: Vec<Op> = chart
        .ops
        .iter()
        .map(|op| match op {
            DrawOp::Mesh(m) => Op::Mesh(m),
            DrawOp::Series(s) => {
                index += 1;
                Op::Series(s, index - 1)
            }
            DrawOp::SeriesLabels(l) => Op::SeriesLabels(l),
        })
        .collect();
    if !ops.iter().any(|op| matches!(op, Op::Mesh(_))) {
        ops.insert(0, Op::Mesh(&DEFAULT_MESH));
    }
    let labelled = chart.series().any(|s| s.label.is_some());
    if labelled && !ops.iter().any(|op| matches!(op, Op::SeriesLabels(_))) {
        ops.push(Op::SeriesLabels(&DEFAULT_LABELS));
    }
    ops
}

fn plotters_error(e: impl std::fmt::Display) -> Error {
    Error::render(e)
}

pub(crate) fn font_style(style: FontStyle) -> plotters::style::FontStyle {
    match style {
        FontStyle::Normal => plotters::style::FontStyle::Normal,
        FontStyle::Oblique => plotters::style::FontStyle::Oblique,
        FontStyle::Italic => plotters::style::FontStyle::Italic,
        FontStyle::Bold => plotters::style::FontStyle::Bold,
    }
}

fn text_style(font: &Font) -> TextStyle<'_> {
    let desc = FontDesc::new(
        FontFamily::from(font.family.as_str()),
        f64::from(font.size),
        font_style(font.style),
    );
    match font.color {
        Some(color) => desc.color(&color.to_plotters()),
        None => desc.into(),
    }
}

fn label_position(p: LabelPosition) -> SeriesLabelPosition {
    match p {
        LabelPosition::UpperLeft => SeriesLabelPosition::UpperLeft,
        LabelPosition::MiddleLeft => SeriesLabelPosition::MiddleLeft,
        LabelPosition::LowerLeft => SeriesLabelPosition::LowerLeft,
        LabelPosition::UpperMiddle => SeriesLabelPosition::UpperMiddle,
        LabelPosition::MiddleMiddle => SeriesLabelPosition::MiddleMiddle,
        LabelPosition::LowerMiddle => SeriesLabelPosition::LowerMiddle,
        LabelPosition::UpperRight => SeriesLabelPosition::UpperRight,
        LabelPosition::MiddleRight => SeriesLabelPosition::MiddleRight,
        LabelPosition::LowerRight => SeriesLabelPosition::LowerRight,
        LabelPosition::Coordinate(x, y) => SeriesLabelPosition::Coordinate(x, y),
    }
}

/// Draws the chart on a root area: fill, then the builder, then every call in chain order.
pub(crate) fn draw<DB: DrawingBackend>(root: &DrawingArea<DB, Shift>, chart: &Chart) -> Result<()> {
    let series: Vec<&Series> = chart.series().collect();
    axis::check_kinds(&series)?;
    let x = axis::coord(chart, &series, Axis::X)?;
    let y = axis::coord(chart, &series, Axis::Y)?;
    root.fill(&chart.fill.to_plotters())
        .map_err(plotters_error)?;
    let ops = resolve(chart);
    draw_on(root, chart, &ops, x, y)
}

fn draw_on<DB: DrawingBackend>(
    root: &DrawingArea<DB, Shift>,
    chart: &Chart,
    ops: &[Op<'_>],
    x: AxisCoord,
    y: AxisCoord,
) -> Result<()> {
    let mut builder = ChartBuilder::on(root);
    builder
        .margin_top(chart.margin.top)
        .margin_bottom(chart.margin.bottom)
        .margin_left(chart.margin.left)
        .margin_right(chart.margin.right)
        .set_label_area_size(LabelAreaPosition::Top, chart.label_area.top)
        .set_label_area_size(LabelAreaPosition::Bottom, chart.label_area.bottom)
        .set_label_area_size(LabelAreaPosition::Left, chart.label_area.left)
        .set_label_area_size(LabelAreaPosition::Right, chart.label_area.right);
    if let Some(caption) = &chart.caption {
        builder.caption(&caption.text, text_style(&caption.font));
    }
    let mut ctx = builder
        .build_cartesian_2d(x.clone(), y.clone())
        .map_err(plotters_error)?;

    for op in ops {
        match op {
            Op::Mesh(style) => draw_mesh(&mut ctx, style, &x, &y)?,
            Op::Series(series, index) => draw_series(&mut ctx, series, *index, &x, &y)?,
            Op::SeriesLabels(style) => {
                let mut labels = ctx.configure_series_labels();
                for setting in &style.settings {
                    match setting {
                        SeriesLabelSetting::Position(p) => labels.position(label_position(*p)),
                        SeriesLabelSetting::BorderStyle {
                            color,
                            stroke_width,
                        } => labels.border_style(ShapeStyle {
                            color: color.to_plotters(),
                            filled: false,
                            stroke_width: *stroke_width,
                        }),
                        SeriesLabelSetting::BackgroundStyle(color) => {
                            labels.background_style(color.to_plotters())
                        }
                        SeriesLabelSetting::Margin(px) => labels.margin(*px),
                        SeriesLabelSetting::LegendAreaSize(px) => labels.legend_area_size(*px),
                        SeriesLabelSetting::LabelFont(f) => labels.label_font(text_style(f)),
                    };
                }
                labels.draw().map_err(plotters_error)?;
            }
        }
    }
    Ok(())
}

/// A label formatter closure, what `x_label_formatter`/`y_label_formatter` take.
type Formatter = Box<dyn Fn(&f64) -> String>;

/// The closure for a `format()` template or strftime pattern on `axis`, or the error for a
/// format that does not suit the axis kind.
fn formatter(what: &str, format: &str, axis: &AxisCoord, name: &str) -> Result<Formatter> {
    let format = LabelFormat::parse(what, format)?;
    let kind = axis.kind();
    let coord = axis.clone();
    Ok(match (format, kind) {
        (LabelFormat::Template(t), AxisKind::Numeric | AxisKind::Integer) => {
            Box::new(move |v| match coord.label_value(*v) {
                Some(LabelValue::Number(n)) => t.number(n, &coord.format_ext(v)),
                _ => String::new(),
            })
        }
        (LabelFormat::Template(t), AxisKind::Category) => {
            if !t.fits_text() {
                return Err(Error::invalid(format!(
                    "{what}: the template formats numbers, and the {name} axis is category: \
                     category labels take only fill, alignment, width and precision \
                     ('{{:>10}}', '{{:.3}}')"
                )));
            }
            Box::new(move |v| match coord.label_value(*v) {
                Some(LabelValue::Text(s)) => t.text(&s),
                _ => String::new(),
            })
        }
        (LabelFormat::Strftime(p), AxisKind::Date | AxisKind::Timestamp) => {
            Box::new(move |v| match coord.label_value(*v) {
                Some(LabelValue::Time(t)) => strftime(&p, t),
                _ => String::new(),
            })
        }
        (format, kind) => {
            let example = match format {
                LabelFormat::Template(_) => "a strftime pattern such as '%Y-%m-%d'",
                LabelFormat::Strftime(_) => "a format() template such as '{:.1f}'",
            };
            return Err(Error::invalid(format!(
                "{what}: a {} does not apply to the {kind} {name} axis; use {example}",
                format.kind_name()
            )));
        }
    })
}

fn line_style(style: &LineStyle) -> ShapeStyle {
    ShapeStyle {
        color: style.color.to_plotters(),
        filled: false,
        stroke_width: style.stroke_width,
    }
}

fn tick_position(p: TickPosition) -> LabelAreaPosition {
    match p {
        TickPosition::Top => LabelAreaPosition::Top,
        TickPosition::Bottom => LabelAreaPosition::Bottom,
        TickPosition::Left => LabelAreaPosition::Left,
        TickPosition::Right => LabelAreaPosition::Right,
    }
}

/// `configure_mesh()`, the setter calls in chain order, then `draw()`.
fn draw_mesh<'a, DB: DrawingBackend + 'a>(
    ctx: &mut ChartContext<'a, DB, Cartesian2d<AxisCoord, AxisCoord>>,
    style: &MeshStyle,
    x: &AxisCoord,
    y: &AxisCoord,
) -> Result<()> {
    // A later formatter call replaces an earlier one, as in plotters, so only the last counts.
    let last = |f: fn(&MeshSetting) -> Option<&String>| style.settings.iter().rev().find_map(f);
    let format_x = last(|s| match s {
        MeshSetting::XLabelFormatter(f) => Some(f),
        _ => None,
    })
    .map(|f| formatter("x_label_formatter", f, x, "x"))
    .transpose()?;
    let format_y = last(|s| match s {
        MeshSetting::YLabelFormatter(f) => Some(f),
        _ => None,
    })
    .map(|f| formatter("y_label_formatter", f, y, "y"))
    .transpose()?;

    let mut mesh = ctx.configure_mesh();
    for setting in &style.settings {
        match setting {
            MeshSetting::XDesc(text) => mesh.x_desc(text.as_str()),
            MeshSetting::YDesc(text) => mesh.y_desc(text.as_str()),
            MeshSetting::AxisDescStyle(f) => mesh.axis_desc_style(text_style(f)),
            MeshSetting::XLabels(n) => mesh.x_labels(*n as usize),
            MeshSetting::YLabels(n) => mesh.y_labels(*n as usize),
            MeshSetting::XLabelFormatter(_) | MeshSetting::YLabelFormatter(_) => &mut mesh,
            MeshSetting::LabelStyle(f) => mesh.label_style(text_style(f)),
            MeshSetting::XLabelStyle(f) => mesh.x_label_style(text_style(f)),
            MeshSetting::YLabelStyle(f) => mesh.y_label_style(text_style(f)),
            MeshSetting::XLabelOffset(px) => mesh.x_label_offset(*px),
            MeshSetting::YLabelOffset(px) => mesh.y_label_offset(*px),
            MeshSetting::XMaxLightLines(n) => mesh.x_max_light_lines(*n as usize),
            MeshSetting::YMaxLightLines(n) => mesh.y_max_light_lines(*n as usize),
            MeshSetting::MaxLightLines(n) => mesh.max_light_lines(*n as usize),
            MeshSetting::LightLineStyle(l) => mesh.light_line_style(line_style(l)),
            MeshSetting::BoldLineStyle(l) => mesh.bold_line_style(line_style(l)),
            MeshSetting::AxisStyle(l) => mesh.axis_style(line_style(l)),
            MeshSetting::DisableXMesh => mesh.disable_x_mesh(),
            MeshSetting::DisableYMesh => mesh.disable_y_mesh(),
            MeshSetting::DisableMesh => mesh.disable_mesh(),
            MeshSetting::DisableXAxis => mesh.disable_x_axis(),
            MeshSetting::DisableYAxis => mesh.disable_y_axis(),
            MeshSetting::DisableAxes => mesh.disable_axes(),
            MeshSetting::SetTickMarkSize(p, px) => mesh.set_tick_mark_size(tick_position(*p), *px),
            MeshSetting::SetAllTickMarkSize(px) => mesh.set_all_tick_mark_size(*px),
        };
    }
    if let Some(f) = &format_x {
        mesh.x_label_formatter(f.as_ref());
    }
    if let Some(f) = &format_y {
        mesh.y_label_formatter(f.as_ref());
    }
    mesh.draw().map_err(plotters_error)
}

fn draw_series<'a, DB: DrawingBackend + 'a>(
    ctx: &mut ChartContext<'a, DB, Cartesian2d<AxisCoord, AxisCoord>>,
    series: &Series,
    index: usize,
    x: &AxisCoord,
    y: &AxisCoord,
) -> Result<()> {
    let color = series
        .style
        .color
        .unwrap_or_else(|| Color::palette99(index))
        .to_plotters();
    let style = ShapeStyle {
        color,
        filled: series.style.filled,
        stroke_width: series.style.stroke_width,
    };
    let points = || -> Vec<(f64, f64)> {
        (0..series.len())
            .filter_map(|i| Some((x.at(&series.x, i)?, y.at(&series.y, i)?)))
            .collect()
    };

    let anno: &mut SeriesAnno<'a, DB> = match &series.kind {
        SeriesKind::Line(options) => {
            ctx.draw_series(LineSeries::new(points(), style).point_size(options.point_size))
        }
        SeriesKind::Point(options) => {
            let (pts, size) = (points(), options.size);
            match options.marker {
                Marker::Circle => {
                    ctx.draw_series(PointSeries::<_, _, Circle<_, _>, _>::new(pts, size, style))
                }
                Marker::Cross => {
                    ctx.draw_series(PointSeries::<_, _, Cross<_, _>, _>::new(pts, size, style))
                }
                Marker::Triangle => ctx.draw_series(
                    PointSeries::<_, _, TriangleMarker<_, _>, _>::new(pts, size, style),
                ),
                Marker::Pixel => {
                    ctx.draw_series(PointSeries::<_, _, Pixel<_>, _>::new(pts, size, style))
                }
            }
        }
        SeriesKind::Histogram(options) => {
            let base = y.number(options.baseline);
            let bars: Vec<Rectangle<(f64, f64)>> = bucket_sums(series)
                .into_iter()
                .filter_map(|(row, sum)| {
                    let (left, right) = x.band(&series.x, row)?;
                    let mut bar = Rectangle::new([(left, y.number(sum)?), (right, base?)], style);
                    bar.set_margin(0, 0, options.margin, options.margin);
                    Some(bar)
                })
                .collect();
            ctx.draw_series(bars)
        }
        SeriesKind::HistogramHorizontal(options) => {
            let base = x.number(options.baseline);
            let bars: Vec<Rectangle<(f64, f64)>> = bucket_sums(series)
                .into_iter()
                .filter_map(|(row, sum)| {
                    let (low, high) = y.band(&series.y, row)?;
                    let mut bar = Rectangle::new([(x.number(sum)?, low), (base?, high)], style);
                    bar.set_margin(options.margin, options.margin, 0, 0);
                    Some(bar)
                })
                .collect();
            ctx.draw_series(bars)
        }
    }
    .map_err(plotters_error)?;

    if let Some(label) = &series.label {
        anno.label(label.as_str());
        legend_glyph(anno, series, style);
    }
    Ok(())
}

/// The legend glyph plotters' examples draw for each series kind, in place of the
/// `SeriesAnno::legend` closure.
fn legend_glyph<'a, DB: DrawingBackend + 'a>(
    anno: &mut SeriesAnno<'a, DB>,
    series: &Series,
    style: ShapeStyle,
) {
    match &series.kind {
        SeriesKind::Line(options) => {
            let point_size = options.point_size;
            if point_size > 0 {
                anno.legend(move |(x, y)| {
                    EmptyElement::at((x, y))
                        + PathElement::new(vec![(0, 0), (GLYPH_WIDTH, 0)], style)
                        + Circle::new((GLYPH_WIDTH / 2, 0), point_size, style)
                });
            } else {
                anno.legend(move |(x, y)| {
                    PathElement::new(vec![(x, y), (x + GLYPH_WIDTH, y)], style)
                });
            }
        }
        SeriesKind::Point(options) => {
            let (size, marker) = (options.size, options.marker);
            anno.legend(move |(x, y)| {
                let at = (x + GLYPH_WIDTH / 2, y);
                match marker {
                    Marker::Circle => Circle::new(at, size, style).into_dyn(),
                    Marker::Cross => Cross::new(at, size, style).into_dyn(),
                    Marker::Triangle => TriangleMarker::new(at, size, style).into_dyn(),
                    Marker::Pixel => Pixel::new(at, style).into_dyn(),
                }
            });
        }
        SeriesKind::Histogram(_) | SeriesKind::HistogramHorizontal(_) => {
            anno.legend(move |(x, y)| {
                Rectangle::new(
                    [
                        (x, y - GLYPH_HALF_HEIGHT),
                        (x + GLYPH_WIDTH, y + GLYPH_HALF_HEIGHT),
                    ],
                    style,
                )
            });
        }
    }
}
