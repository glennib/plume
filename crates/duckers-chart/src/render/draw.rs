//! Replaying a chart's calls on a plotters `ChartContext`.

use super::axis::{self, DataAxis, XCoord, YCoord, bucket_sums};
use crate::color::Color;
use crate::error::{Error, Result};
use crate::spec::{
    Chart, DrawOp, Font, FontStyle, LabelPosition, Marker, MeshSetting, MeshStyle, Series,
    SeriesKind, SeriesLabelSetting, SeriesLabelStyle,
};
use plotters::chart::SeriesAnno;
use plotters::coord::Shift;
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
    let x = axis::x_coord(chart, &series)?;
    let y = axis::y_coord(chart, &series)?;
    root.fill(&chart.fill.to_plotters())
        .map_err(plotters_error)?;
    let ops = resolve(chart);
    match x {
        XCoord::Linear(x) => with_y(root, chart, &ops, x, y),
        XCoord::Date(x) => with_y(root, chart, &ops, x, y),
        XCoord::Timestamp(x) => with_y(root, chart, &ops, x, y),
        XCoord::Band(x) => with_y(root, chart, &ops, x, y),
    }
}

fn with_y<DB: DrawingBackend, X: DataAxis>(
    root: &DrawingArea<DB, Shift>,
    chart: &Chart,
    ops: &[Op<'_>],
    x: X,
    y: YCoord,
) -> Result<()>
where
    X::ValueType: Clone + 'static,
{
    match y {
        YCoord::Linear(y) => draw_on(root, chart, ops, x, y),
    }
}

fn draw_on<DB: DrawingBackend, X: DataAxis, Y: DataAxis>(
    root: &DrawingArea<DB, Shift>,
    chart: &Chart,
    ops: &[Op<'_>],
    x: X,
    y: Y,
) -> Result<()>
where
    X::ValueType: Clone + 'static,
    Y::ValueType: Clone + 'static,
{
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
            Op::Mesh(style) => {
                let mut mesh = ctx.configure_mesh();
                for setting in &style.settings {
                    match setting {
                        MeshSetting::XDesc(text) => mesh.x_desc(text.as_str()),
                        MeshSetting::YDesc(text) => mesh.y_desc(text.as_str()),
                    };
                }
                mesh.draw().map_err(plotters_error)?;
            }
            Op::Series(series, index) => draw_series(&mut ctx, series, *index, &x, &y)?,
            Op::SeriesLabels(style) => {
                let mut labels = ctx.configure_series_labels();
                for setting in &style.settings {
                    match setting {
                        SeriesLabelSetting::Position(p) => labels.position(label_position(*p)),
                    };
                }
                labels.draw().map_err(plotters_error)?;
            }
        }
    }
    Ok(())
}

fn draw_series<'a, DB, X, Y>(
    ctx: &mut ChartContext<'a, DB, Cartesian2d<X, Y>>,
    series: &Series,
    index: usize,
    x: &X,
    y: &Y,
) -> Result<()>
where
    DB: DrawingBackend + 'a,
    X: DataAxis,
    Y: DataAxis,
    X::ValueType: Clone + 'static,
    Y::ValueType: Clone + 'static,
{
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
    let points = || -> Vec<(X::ValueType, Y::ValueType)> {
        (0..series.len())
            .filter_map(|i| Some((x.at(&series.x, i)?, y.number(series.y[i])?)))
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
            let bars: Vec<Rectangle<(X::ValueType, Y::ValueType)>> = bucket_sums(series)
                .into_iter()
                .filter_map(|(row, sum)| {
                    let (left, right) = x.band(&series.x, row)?;
                    let mut bar =
                        Rectangle::new([(left, y.number(sum)?), (right, base.clone()?)], style);
                    bar.set_margin(0, 0, options.margin, options.margin);
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
        SeriesKind::Histogram(_) => {
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
