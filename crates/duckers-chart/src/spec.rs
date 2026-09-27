//! The plain-data specs carried inside the SQL values.
//!
//! Every type here is serialized with postcard (see [`crate::envelope`]). postcard is not
//! self-describing, so two rules keep old values readable:
//!
//! - enums only ever gain variants at the end (postcard writes the variant index), which is how
//!   new series kinds, mesh settings, legend settings and scales are added;
//! - structs never gain, lose or reorder fields without a [`crate::envelope::FORMAT_VERSION`]
//!   bump.
//!
//! The builder-style settings of `MeshStyle` and `SeriesLabelStyle` are stored as the list of
//! calls made, in chain order, so later plotters setters extend the enum instead of the struct.

use crate::color::Color;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The column that places a series on one axis: x or y values, or histogram buckets.
/// Its variant decides the kind of the axis.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Column {
    /// Any numeric SQL type, drawn on a continuous `f64` axis.
    Numeric(Vec<f64>),
    /// Integer histogram buckets, drawn on a segmented integer axis.
    Integer(Vec<i64>),
    /// Days since 1970-01-01, as DuckDB stores `DATE`.
    Date(Vec<i32>),
    /// Microseconds since 1970-01-01 00:00:00, as DuckDB stores `TIMESTAMP`.
    Timestamp(Vec<i64>),
    /// Category names, drawn on a segmented category axis.
    Category(Vec<String>),
}

impl Column {
    pub fn len(&self) -> usize {
        match self {
            Column::Numeric(v) => v.len(),
            Column::Integer(v) => v.len(),
            Column::Date(v) => v.len(),
            Column::Timestamp(v) => v.len(),
            Column::Category(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn axis_kind(&self) -> AxisKind {
        match self {
            Column::Numeric(_) => AxisKind::Numeric,
            Column::Integer(_) => AxisKind::Integer,
            Column::Date(_) => AxisKind::Date,
            Column::Timestamp(_) => AxisKind::Timestamp,
            Column::Category(_) => AxisKind::Category,
        }
    }
}

/// The kind of an axis, decided by the column types of the series drawn on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisKind {
    /// `RangedCoordf64`.
    Numeric,
    /// Integer buckets on a segmented axis (`(lo..hi).into_segmented()`).
    Integer,
    /// `RangedDate`, segmented when a histogram is drawn on it.
    Date,
    /// `RangedDateTime`.
    Timestamp,
    /// Categories on a segmented axis.
    Category,
}

impl fmt::Display for AxisKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AxisKind::Numeric => "numeric",
            AxisKind::Integer => "integer bucket",
            AxisKind::Date => "date",
            AxisKind::Timestamp => "timestamp",
            AxisKind::Category => "category",
        })
    }
}

/// One of the two axes of a chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

impl Axis {
    /// `"x"` or `"y"`, as in `x_range` and error messages.
    pub fn name(self) -> &'static str {
        match self {
            Axis::X => "x",
            Axis::Y => "y",
        }
    }

    /// The other axis.
    pub fn other(self) -> Axis {
        match self {
            Axis::X => Axis::Y,
            Axis::Y => Axis::X,
        }
    }
}

/// A `SERIES` value: one plotters series with its style and legend label.
///
/// `x` and `y` are the series' columns on the two axes. A vertical histogram has its buckets
/// in `x` and the bar heights in `y`; a horizontal one has the bar lengths in `x` and its
/// buckets in `y`. Both columns have one value per point (or per bucket).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Series {
    pub kind: SeriesKind,
    /// x values, or the buckets of a vertical histogram.
    pub x: Column,
    /// y values, or the buckets of a horizontal histogram.
    pub y: Column,
    pub style: Style,
    /// `SeriesAnno::label`; a series without one is not in the legend.
    pub label: Option<String>,
}

/// The series type and the options only that type has.
///
/// New kinds are appended as new variants carrying their own options and any value columns
/// beyond the series' `x` and `y`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SeriesKind {
    /// `LineSeries`.
    Line(LineOptions),
    /// `PointSeries`.
    Point(PointOptions),
    /// `Histogram::vertical`: buckets on x, bars up from the baseline.
    Histogram(HistogramOptions),
    /// `Histogram::horizontal`: buckets on y, bars rightwards from the baseline.
    HistogramHorizontal(HistogramOptions),
    /// `AreaSeries`: `x` and `y` are the points of the polygon's upper edge.
    Area(AreaOptions),
    /// `DashedLineSeries`.
    DashedLine(DashedLineOptions),
    /// `ErrorBar::new_vertical` per point: `x` is the key, `y` the average.
    ErrorBarVertical(ErrorBarOptions),
    /// `ErrorBar::new_horizontal` per point: `y` is the key, `x` the average.
    ErrorBarHorizontal(ErrorBarOptions),
    /// `CandleStick::new` per point: `x` is the key, `y` the close. Boxed, being the largest
    /// options.
    CandleStick(Box<CandleStickOptions>),
    /// `Boxplot::new_vertical` per key: `x` is the key, `y` the median.
    BoxplotVertical(BoxplotOptions),
    /// `Boxplot::new_horizontal` per key: `y` is the key, `x` the median.
    BoxplotHorizontal(BoxplotOptions),
}

impl SeriesKind {
    /// The name used in summaries and error messages.
    pub fn name(&self) -> &'static str {
        match self {
            SeriesKind::Line(_) => "line",
            SeriesKind::Point(_) => "point",
            SeriesKind::Histogram(_) => "histogram",
            SeriesKind::HistogramHorizontal(_) => "horizontal histogram",
            SeriesKind::Area(_) => "area",
            SeriesKind::DashedLine(_) => "dashed line",
            SeriesKind::ErrorBarVertical(_) => "error bar",
            SeriesKind::ErrorBarHorizontal(_) => "horizontal error bar",
            SeriesKind::CandleStick(_) => "candlestick",
            SeriesKind::BoxplotVertical(_) => "boxplot",
            SeriesKind::BoxplotHorizontal(_) => "horizontal boxplot",
        }
    }

    /// The SQL aggregate that makes this kind.
    pub fn aggregate_name(&self) -> &'static str {
        match self {
            SeriesKind::Line(_) => "line_series",
            SeriesKind::Point(_) => "point_series",
            SeriesKind::Histogram(_) => "histogram_vertical",
            SeriesKind::HistogramHorizontal(_) => "histogram_horizontal",
            SeriesKind::Area(_) => "area_series",
            SeriesKind::DashedLine(_) => "dashed_line_series",
            SeriesKind::ErrorBarVertical(_) => "error_bar_vertical",
            SeriesKind::ErrorBarHorizontal(_) => "error_bar_horizontal",
            SeriesKind::CandleStick(_) => "candle_stick",
            SeriesKind::BoxplotVertical(_) => "boxplot_vertical",
            SeriesKind::BoxplotHorizontal(_) => "boxplot_horizontal",
        }
    }

    /// The histogram options of either histogram kind.
    pub fn histogram(&self) -> Option<&HistogramOptions> {
        match self {
            SeriesKind::Histogram(o) | SeriesKind::HistogramHorizontal(o) => Some(o),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LineOptions {
    /// `LineSeries::point_size`, 0 for no markers.
    pub point_size: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PointOptions {
    /// The `size` argument of `PointSeries::new`.
    pub size: u32,
    /// The element type of `PointSeries::new`.
    pub marker: Marker,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Circle,
    Cross,
    Triangle,
    Pixel,
}

impl Marker {
    pub fn name(self) -> &'static str {
        match self {
            Marker::Circle => "circle",
            Marker::Cross => "cross",
            Marker::Triangle => "triangle",
            Marker::Pixel => "pixel",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HistogramOptions {
    /// `Histogram::margin`, px on each side of a bar.
    pub margin: u32,
    /// `Histogram::baseline`.
    pub baseline: f64,
    /// `(lo..hi).step(s).use_round().into_segmented()`: numeric buckets binned into bands of
    /// width `s`, bin `k` holding the values from `k * s` up to `(k + 1) * s`. `None` for
    /// category, integer and date buckets, which are one band per value.
    pub step: Option<f64>,
}

/// The bin of `value` for bands of width `step`: `floor(value / step)`, except that a quotient
/// within a relative 1e-9 of an integer is that integer, so that `0.3` is in bin 3 of step
/// `0.1` although `0.3 / 0.1` is `2.9999999999999996`. `None` beyond the `i64` range.
pub(crate) fn bin(value: f64, step: f64) -> Option<i64> {
    let q = value / step;
    let r = q.round();
    let k = if (q - r).abs() <= 1e-9 * r.abs().max(1.0) {
        r
    } else {
        q.floor()
    };
    (k.is_finite() && k.abs() < 9e15).then_some(k as i64)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaOptions {
    /// The `baseline` argument of `AreaSeries::new`, in the units of the y axis.
    pub baseline: f64,
    /// `AreaSeries::border_style`, transparent by default.
    pub border_style: LineStyle,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DashedLineOptions {
    /// The `size` argument of `DashedLineSeries::new`: px of each dash.
    pub size: u32,
    /// The `spacing` argument: px between dashes.
    pub spacing: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ErrorBarOptions {
    /// The `width` argument of `ErrorBar::new_*`: px of the end marks, and the dot's diameter.
    pub width: u32,
    /// The `min` of each point, in the units of the value axis.
    pub min: Vec<f64>,
    /// The `max` of each point.
    pub max: Vec<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CandleStickOptions {
    /// The `width` argument of `CandleStick::new`: px of the body.
    pub width: u32,
    /// The colour of `gain_style` (close above open); the series' stroke width and fill apply.
    pub gain: Color,
    /// The colour of `loss_style` (close at or below open).
    pub loss: Color,
    pub open: Vec<f64>,
    pub high: Vec<f64>,
    pub low: Vec<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BoxplotOptions {
    /// `Boxplot::width`: px of the box.
    pub width: u32,
    /// `Quartiles::values()` of each key: lower fence, lower quartile, median, upper quartile,
    /// upper fence (the `f32` values plotters computes, widened exactly to `f64`).
    pub quartiles: Vec<[f64; 5]>,
}

/// A `ShapeStyle`, with the colour left open until the chart picks it from `Palette99`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Style {
    /// `None` means `Palette99::pick(i)` for series `i` of the chart.
    pub color: Option<Color>,
    pub stroke_width: u32,
    pub filled: bool,
}

/// A `FONT` value: a `FontDesc` plus the colour of a `TextStyle`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Font {
    pub family: String,
    pub size: u32,
    pub style: FontStyle,
    /// `None` is plotters' default text colour, black.
    pub color: Option<Color>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontStyle {
    Normal,
    Oblique,
    Italic,
    Bold,
}

impl FontStyle {
    pub fn name(self) -> &'static str {
        match self {
            FontStyle::Normal => "normal",
            FontStyle::Oblique => "oblique",
            FontStyle::Italic => "italic",
            FontStyle::Bold => "bold",
        }
    }
}

/// A `CHART` value: what is drawn on the root `DrawingArea`. A chart made by `chart()` is a
/// cartesian chart; `split_evenly`, `titled` and `pie` make the other roots.
///
/// The cartesian variant is the largest; roots are decoded one per call (and one per grid
/// cell), so boxing it would buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub enum Root {
    /// `ChartBuilder::on(&root)` and the calls on its `ChartContext`.
    Cartesian(Chart),
    /// `root.split_evenly((rows, cols))`, one root per cell.
    Grid(Grid),
    /// `root.titled(text, style)`, with a root drawn below the title.
    Titled(Titled),
    /// A `Pie` element drawn on the root.
    Pie(Pie),
}

/// `DrawingArea::split_evenly((rows, cols))`: the cells in row-major order, `None` for a blank
/// one. There are at most `rows * cols` cells; missing ones at the end are blank.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Grid {
    pub rows: u32,
    pub cols: u32,
    pub cells: Vec<Option<Root>>,
    /// `root.fill(...)` of the whole area, which blank cells show.
    pub fill: Color,
}

/// `DrawingArea::titled(text, style)`: the title across the top, `inner` in the area below.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Titled {
    pub text: String,
    pub font: Font,
    /// `root.fill(...)` of the whole area, under the title.
    pub fill: Color,
    pub inner: Box<Root>,
}

/// A `Pie`: one slice per row of the `pie` aggregate, in slice order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pie {
    /// The `sizes` argument of `Pie::new`, all positive and finite.
    pub sizes: Vec<f64>,
    /// The `labels` argument of `Pie::new`, one per size.
    pub labels: Vec<String>,
    /// `root.fill(...)`.
    pub fill: Color,
    /// The setter calls on the `Pie` and its `radius`, in chain order.
    pub settings: Vec<PieSetting>,
}

/// One `Pie` setter call, or the `radius` argument of `Pie::new`. Further settings are appended
/// as new variants.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PieSetting {
    /// `Pie::start_angle`, degrees clockwise from the positive x axis.
    StartAngle(f64),
    /// `Pie::label_style`.
    LabelStyle(Font),
    /// `Pie::percentages`: draw each slice's percentage in this style.
    Percentages(Font),
    /// `Pie::label_offset`, px from the rim; may be negative.
    LabelOffset(i32),
    /// The `radius` argument of `Pie::new`, px.
    Radius(u32),
}

impl PieSetting {
    /// The font the setting names, if it takes one.
    pub fn font(&self) -> Option<&Font> {
        match self {
            PieSetting::LabelStyle(f) | PieSetting::Percentages(f) => Some(f),
            _ => None,
        }
    }
}

/// The derived `Debug` of a cartesian root is its chart's, so `__duckers_debug` shows the
/// builder settings of a `chart()` value directly.
impl fmt::Debug for Root {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Root::Cartesian(chart) => chart.fmt(f),
            Root::Grid(grid) => grid.fmt(f),
            Root::Titled(titled) => titled.fmt(f),
            Root::Pie(pie) => pie.fmt(f),
        }
    }
}

impl From<Chart> for Root {
    fn from(chart: Chart) -> Root {
        Root::Cartesian(chart)
    }
}

/// A cartesian chart: the `ChartBuilder` settings, the root fill, and the drawing calls made on
/// the `ChartContext`, in chain order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chart {
    pub caption: Option<Caption>,
    /// `ChartBuilder::margin*`.
    pub margin: Sides,
    /// `ChartBuilder::*_label_area_size`.
    pub label_area: LabelAreas,
    pub x_axis: AxisSpec,
    pub y_axis: AxisSpec,
    /// `ChartContext::set_secondary_coord`: `None` for a chart with one coordinate system.
    pub secondary: Option<SecondaryCoord>,
    /// `root.fill(...)`.
    pub fill: Color,
    /// Draw order is chain order.
    pub ops: Vec<DrawOp>,
}

/// The label area sizes of `ChartBuilder`, px. The top and right areas are `None` until the
/// chain sets them: their default depends on whether the chart has secondary axes, which a
/// later call may add.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabelAreas {
    /// `top_x_label_area_size`: 0 by default, 40 when the secondary x axis differs from the
    /// primary one.
    pub top: Option<u32>,
    /// `x_label_area_size`.
    pub bottom: u32,
    /// `y_label_area_size`.
    pub left: u32,
    /// `right_y_label_area_size`: 0 by default, 40 on a chart with secondary axes.
    pub right: Option<u32>,
}

/// The secondary coordinate system of `set_secondary_coord(x, y)`: what the range arguments
/// get beyond what the secondary series imply. Its scales are linear.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SecondaryCoord {
    pub x_axis: AxisSpec,
    pub y_axis: AxisSpec,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Caption {
    pub text: String,
    pub font: Font,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sides {
    pub top: u32,
    pub bottom: u32,
    pub left: u32,
    pub right: u32,
}

/// What `build_cartesian_2d` gets for one axis beyond what the data implies.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AxisSpec {
    /// `None` is the data extent.
    pub range: Option<AxisRange>,
    pub scale: Scale,
}

/// An explicit axis range, in the units of the axis kind. `lo > hi` reverses the axis.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum AxisRange {
    Numeric(f64, f64),
    /// Days since 1970-01-01.
    Date(i32, i32),
    /// Microseconds since 1970-01-01 00:00:00.
    Timestamp(i64, i64),
}

impl AxisRange {
    /// The axis kind the bounds' type implies, for an axis no series has set yet.
    pub fn kind(self) -> AxisKind {
        match self {
            AxisRange::Numeric(..) => AxisKind::Numeric,
            AxisRange::Date(..) => AxisKind::Date,
            AxisRange::Timestamp(..) => AxisKind::Timestamp,
        }
    }
}

/// The range combinator of an axis.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    Linear,
    /// `(lo..hi).log_scale().base(base)`, plotters' `LogCoord`. The base only changes the key
    /// points; the mapping is linear in `ln(v)`.
    Log {
        base: f64,
    },
    /// `(lo..hi).monthly()` (`IntoMonthly`): key points at month starts, labelled as plotters'
    /// `Monthly` labels them. Date and timestamp axes only.
    Monthly,
    /// `(lo..hi).yearly()` (`IntoYearly`): key points at year starts.
    Yearly,
}

impl Scale {
    /// The SQL method that sets the scale on `axis` (`x_log_scale`, `y_monthly`); `x_range`
    /// for the linear default.
    pub fn method(self, axis: Axis) -> String {
        let name = match self {
            Scale::Linear => "range",
            Scale::Log { .. } => "log_scale",
            Scale::Monthly => "monthly",
            Scale::Yearly => "yearly",
        };
        format!("{}_{name}", axis.name())
    }
}

/// One drawing call on the `ChartContext`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum DrawOp {
    /// `configure_mesh()...draw()`.
    Mesh(MeshStyle),
    /// `draw_series(...)`.
    Series(Series),
    /// `configure_series_labels()...draw()`.
    SeriesLabels(SeriesLabelStyle),
    /// `draw_secondary_series(...)`.
    SecondarySeries(Series),
    /// `configure_secondary_axes()...draw()`: the setters of `SecondaryMeshStyle`.
    SecondaryAxes(MeshStyle),
}

/// The settings of a `MeshStyle`, as the list of setter calls in chain order.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MeshStyle {
    pub settings: Vec<MeshSetting>,
}

/// One `MeshStyle` setter call, named as plotters names it. Further setters are appended as
/// new variants.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum MeshSetting {
    XDesc(String),
    YDesc(String),
    AxisDescStyle(Font),
    XLabels(u32),
    YLabels(u32),
    /// A `format()` template or strftime pattern, checked by
    /// [`crate::label_format::LabelFormat::parse`] when set.
    XLabelFormatter(String),
    YLabelFormatter(String),
    LabelStyle(Font),
    XLabelStyle(Font),
    YLabelStyle(Font),
    /// px; plotters' `SizeDesc` allows negative offsets.
    XLabelOffset(i32),
    YLabelOffset(i32),
    XMaxLightLines(u32),
    YMaxLightLines(u32),
    MaxLightLines(u32),
    LightLineStyle(LineStyle),
    BoldLineStyle(LineStyle),
    AxisStyle(LineStyle),
    DisableXMesh,
    DisableYMesh,
    DisableMesh,
    DisableXAxis,
    DisableYAxis,
    DisableAxes,
    /// px; a negative size points the ticks into the plotting area.
    SetTickMarkSize(TickPosition, i32),
    SetAllTickMarkSize(i32),
}

impl MeshSetting {
    /// The font the setting names, if it takes one.
    pub fn font(&self) -> Option<&Font> {
        match self {
            MeshSetting::AxisDescStyle(f)
            | MeshSetting::LabelStyle(f)
            | MeshSetting::XLabelStyle(f)
            | MeshSetting::YLabelStyle(f) => Some(f),
            _ => None,
        }
    }
}

/// An `Into<ShapeStyle>` for lines: a colour and a stroke width, not filled.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct LineStyle {
    pub color: Color,
    pub stroke_width: u32,
}

/// `LabelAreaPosition`, the label area a tick mark size applies to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickPosition {
    Top,
    Bottom,
    Left,
    Right,
}

impl TickPosition {
    pub(crate) const NAMED: [(&'static str, TickPosition); 4] = [
        ("top", TickPosition::Top),
        ("bottom", TickPosition::Bottom),
        ("left", TickPosition::Left),
        ("right", TickPosition::Right),
    ];

    pub fn name(self) -> &'static str {
        TickPosition::NAMED
            .iter()
            .find(|(_, p)| *p == self)
            .map(|(n, _)| *n)
            .expect("every position is named")
    }
}

/// The settings of a `SeriesLabelStyle`, as the list of setter calls in chain order.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SeriesLabelStyle {
    pub settings: Vec<SeriesLabelSetting>,
}

/// One `SeriesLabelStyle` setter call. Further setters are appended as new variants.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SeriesLabelSetting {
    Position(LabelPosition),
    /// `SeriesLabelStyle::border_style`.
    BorderStyle {
        color: Color,
        stroke_width: u32,
    },
    /// `SeriesLabelStyle::background_style`, drawn filled as plotters does.
    BackgroundStyle(Color),
    /// `SeriesLabelStyle::margin`, px of padding inside the box.
    Margin(u32),
    /// `SeriesLabelStyle::legend_area_size`, px width of the glyph column.
    LegendAreaSize(u32),
    /// `SeriesLabelStyle::label_font`.
    LabelFont(Font),
}

impl SeriesLabelSetting {
    /// The font the setting names, if it takes one.
    pub fn font(&self) -> Option<&Font> {
        match self {
            SeriesLabelSetting::LabelFont(f) => Some(f),
            _ => None,
        }
    }
}

/// `SeriesLabelPosition`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelPosition {
    UpperLeft,
    MiddleLeft,
    LowerLeft,
    UpperMiddle,
    MiddleMiddle,
    LowerMiddle,
    UpperRight,
    MiddleRight,
    LowerRight,
    /// px from the plotting-area origin to the top left of the legend box.
    Coordinate(i32, i32),
}

impl LabelPosition {
    pub(crate) const NAMED: [(&'static str, LabelPosition); 9] = [
        ("upper_left", LabelPosition::UpperLeft),
        ("middle_left", LabelPosition::MiddleLeft),
        ("lower_left", LabelPosition::LowerLeft),
        ("upper_middle", LabelPosition::UpperMiddle),
        ("middle_middle", LabelPosition::MiddleMiddle),
        ("lower_middle", LabelPosition::LowerMiddle),
        ("upper_right", LabelPosition::UpperRight),
        ("middle_right", LabelPosition::MiddleRight),
        ("lower_right", LabelPosition::LowerRight),
    ];
}

/// A `MESH` value: the mesh settings so far and the chart they were taken from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mesh {
    pub chart: Chart,
    pub style: MeshStyle,
    /// Taken by `configure_secondary_axes()`: a `SecondaryMeshStyle`, which has a subset of
    /// the setters.
    pub secondary: bool,
}

/// A `SERIES_LABELS` value: the legend settings so far and the chart they were taken from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SeriesLabels {
    pub chart: Chart,
    pub style: SeriesLabelStyle,
}

impl Chart {
    /// The series drawn so far, in draw order.
    pub fn series(&self) -> impl Iterator<Item = &Series> {
        self.ops.iter().filter_map(|op| match op {
            DrawOp::Series(s) => Some(s),
            _ => None,
        })
    }

    /// The series drawn on the secondary coordinate system so far, in draw order.
    pub fn secondary_series(&self) -> impl Iterator<Item = &Series> {
        self.ops.iter().filter_map(|op| match op {
            DrawOp::SecondarySeries(s) => Some(s),
            _ => None,
        })
    }

    /// The kind of the secondary `axis` set by the secondary series drawn so far, if any.
    pub fn secondary_kind_on(&self, axis: Axis) -> Option<AxisKind> {
        self.secondary_series().next().map(|s| s.kind_on(axis))
    }

    /// The x-axis kind set by the series drawn so far, if any.
    pub fn x_kind(&self) -> Option<AxisKind> {
        self.kind_on(Axis::X)
    }

    /// The y-axis kind set by the series drawn so far, if any.
    pub fn y_kind(&self) -> Option<AxisKind> {
        self.kind_on(Axis::Y)
    }

    /// The kind of `axis` set by the series drawn so far, if any.
    pub fn kind_on(&self, axis: Axis) -> Option<AxisKind> {
        self.series().next().map(|s| s.kind_on(axis))
    }

    /// The settings of `axis`.
    pub fn axis(&self, axis: Axis) -> &AxisSpec {
        match axis {
            Axis::X => &self.x_axis,
            Axis::Y => &self.y_axis,
        }
    }

    pub(crate) fn axis_mut(&mut self, axis: Axis) -> &mut AxisSpec {
        match axis {
            Axis::X => &mut self.x_axis,
            Axis::Y => &mut self.y_axis,
        }
    }
}

impl Series {
    /// The kind of x axis this series needs.
    pub fn x_kind(&self) -> AxisKind {
        self.x.axis_kind()
    }

    /// The kind of y axis this series needs.
    pub fn y_kind(&self) -> AxisKind {
        self.y.axis_kind()
    }

    /// The kind of `axis` this series needs.
    pub fn kind_on(&self, axis: Axis) -> AxisKind {
        self.column(axis).axis_kind()
    }

    /// The column of the series on `axis`.
    pub fn column(&self, axis: Axis) -> &Column {
        match axis {
            Axis::X => &self.x,
            Axis::Y => &self.y,
        }
    }

    /// The axis a histogram's buckets or a boxplot's keys are on, which is a band axis;
    /// `None` for other kinds.
    pub fn bucket_axis(&self) -> Option<Axis> {
        match self.kind {
            SeriesKind::Histogram(_) | SeriesKind::BoxplotVertical(_) => Some(Axis::X),
            SeriesKind::HistogramHorizontal(_) | SeriesKind::BoxplotHorizontal(_) => Some(Axis::Y),
            _ => None,
        }
    }

    /// The axis of the key for kinds that draw one element per key (error bars, candlesticks,
    /// boxplots); the other axis carries their values. `None` for other kinds.
    pub fn key_axis(&self) -> Option<Axis> {
        match self.kind {
            SeriesKind::ErrorBarVertical(_)
            | SeriesKind::CandleStick(_)
            | SeriesKind::BoxplotVertical(_) => Some(Axis::X),
            SeriesKind::ErrorBarHorizontal(_) | SeriesKind::BoxplotHorizontal(_) => Some(Axis::Y),
            _ => None,
        }
    }

    /// The numbers the series has on `axis` beyond its column there, which the axis' default
    /// extent includes: an area's baseline, the extremes of error bars and candlesticks, the
    /// quartiles and fences of boxplots. Histograms are left to the renderer, which sums their
    /// buckets.
    pub fn extra_numbers(&self, axis: Axis) -> Vec<f64> {
        let on_values = self.key_axis().map(Axis::other) == Some(axis);
        match &self.kind {
            SeriesKind::Area(o) if axis == Axis::Y => vec![o.baseline],
            SeriesKind::ErrorBarVertical(o) | SeriesKind::ErrorBarHorizontal(o) if on_values => {
                o.min.iter().chain(&o.max).copied().collect()
            }
            SeriesKind::CandleStick(o) if on_values => o
                .open
                .iter()
                .chain(&o.high)
                .chain(&o.low)
                .copied()
                .collect(),
            SeriesKind::BoxplotVertical(o) | SeriesKind::BoxplotHorizontal(o) if on_values => {
                o.quartiles.iter().flatten().copied().collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }
}
