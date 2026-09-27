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
/// New kinds (area, dashed line, error bars, candlesticks, boxplots) are appended as new
/// variants carrying their own options and extra value columns.
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
}

impl SeriesKind {
    /// The name used in summaries and error messages.
    pub fn name(&self) -> &'static str {
        match self {
            SeriesKind::Line(_) => "line",
            SeriesKind::Point(_) => "point",
            SeriesKind::Histogram(_) => "histogram",
            SeriesKind::HistogramHorizontal(_) => "horizontal histogram",
        }
    }

    /// The SQL aggregate that makes this kind.
    pub fn aggregate_name(&self) -> &'static str {
        match self {
            SeriesKind::Line(_) => "line_series",
            SeriesKind::Point(_) => "point_series",
            SeriesKind::Histogram(_) => "histogram_vertical",
            SeriesKind::HistogramHorizontal(_) => "histogram_horizontal",
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

/// A `CHART` value: the `ChartBuilder` settings, the root fill, and the drawing calls made on
/// the `ChartContext`, in chain order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chart {
    pub caption: Option<Caption>,
    /// `ChartBuilder::margin*`.
    pub margin: Sides,
    /// `ChartBuilder::*_label_area_size`: `bottom` is `x_label_area_size`, `left` is
    /// `y_label_area_size`, `top` is `top_x_label_area_size`, `right` is
    /// `right_y_label_area_size`.
    pub label_area: Sides,
    pub x_axis: AxisSpec,
    pub y_axis: AxisSpec,
    /// `root.fill(...)`.
    pub fill: Color,
    /// Draw order is chain order.
    pub ops: Vec<DrawOp>,
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
}

/// The settings of a `MeshStyle`, as the list of setter calls in chain order.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MeshStyle {
    pub settings: Vec<MeshSetting>,
}

/// One `MeshStyle` setter call. Further setters are appended as new variants.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum MeshSetting {
    XDesc(String),
    YDesc(String),
}

impl MeshSetting {
    /// The font the setting names, if it takes one.
    pub fn font(&self) -> Option<&Font> {
        None
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
}

impl SeriesLabelSetting {
    /// The font the setting names, if it takes one.
    pub fn font(&self) -> Option<&Font> {
        None
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

    /// The axis a histogram's buckets are on; `None` for other kinds.
    pub fn bucket_axis(&self) -> Option<Axis> {
        match self.kind {
            SeriesKind::Histogram(_) => Some(Axis::X),
            SeriesKind::HistogramHorizontal(_) => Some(Axis::Y),
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }
}
