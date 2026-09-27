//! The SQL methods, as functions on the spec types.
//!
//! Each takes the value first and returns a new value, as the SQL functions do. Integer
//! arguments are taken as `i64`, as SQL passes them, and checked here, so every range error
//! reads the same whichever SQL integer type the caller used.

use crate::color::Color;
use crate::error::{Error, Result};
use crate::spec::{
    Axis, AxisKind, AxisRange, AxisSpec, Caption, Chart, DrawOp, Font, FontStyle, LabelPosition,
    Marker, Mesh, MeshSetting, MeshStyle, Scale, Series, SeriesKind, SeriesLabelSetting,
    SeriesLabelStyle, SeriesLabels, Sides,
};

/// The largest pixel size any size argument accepts, the same as the largest image side.
pub const MAX_PX: u32 = 8192;

/// The caption size when `caption(text)` is called without one.
pub const DEFAULT_CAPTION_SIZE: u32 = 30;

/// One bound of `x_range`/`y_range`, as the SQL layer receives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RangeValue {
    /// Any numeric SQL type.
    Number(f64),
    /// `DATE`, days since 1970-01-01.
    Date(i32),
    /// `TIMESTAMP` and friends, microseconds since 1970-01-01 00:00:00.
    Timestamp(i64),
}

impl RangeValue {
    fn kind_name(self) -> &'static str {
        match self {
            RangeValue::Number(_) => "numeric",
            RangeValue::Date(_) => "date",
            RangeValue::Timestamp(_) => "timestamp",
        }
    }
}

impl AxisRange {
    fn kind_name(self) -> &'static str {
        match self {
            AxisRange::Numeric(..) => "numeric",
            AxisRange::Date(..) => "date",
            AxisRange::Timestamp(..) => "timestamp",
        }
    }
}

pub(crate) fn px(what: &str, value: i64) -> Result<u32> {
    u32::try_from(value)
        .ok()
        .filter(|v| *v <= MAX_PX)
        .ok_or_else(|| {
            Error::invalid(format!(
                "{what} must be between 0 and {MAX_PX} px, got {value}"
            ))
        })
}

fn not_applicable(method: &str, plotters: &str, series: &Series) -> Error {
    Error::invalid(format!(
        "{method} does not apply to {} values: it is {plotters}",
        series.kind.aggregate_name()
    ))
}

impl Series {
    /// `style(color [, stroke_width])`: the `Into<ShapeStyle>` argument of the series.
    pub fn style(mut self, color: &str, stroke_width: Option<i64>) -> Result<Series> {
        self.style.color = Some(Color::parse(color)?);
        if let Some(w) = stroke_width {
            self.style.stroke_width = px("stroke_width", w)?;
        }
        Ok(self)
    }

    /// `ShapeStyle::stroke_width`.
    pub fn stroke_width(mut self, width: i64) -> Result<Series> {
        self.style.stroke_width = px("stroke_width", width)?;
        Ok(self)
    }

    /// `ShapeStyle::filled`: point markers, line markers and histogram bars.
    pub fn filled(mut self) -> Result<Series> {
        match self.kind {
            SeriesKind::Line(_)
            | SeriesKind::Point(_)
            | SeriesKind::Histogram(_)
            | SeriesKind::HistogramHorizontal(_) => {
                self.style.filled = true;
                Ok(self)
            }
        }
    }

    /// `SeriesAnno::label`.
    pub fn label(mut self, text: impl Into<String>) -> Series {
        self.label = Some(text.into());
        self
    }

    /// `LineSeries::point_size`.
    pub fn point_size(mut self, size: i64) -> Result<Series> {
        match &mut self.kind {
            SeriesKind::Line(options) => {
                options.point_size = px("point_size", size)?;
                Ok(self)
            }
            _ => Err(not_applicable(
                "point_size",
                "LineSeries::point_size and applies to line_series",
                &self,
            )),
        }
    }

    /// The `size` argument of `PointSeries::new`.
    pub fn size(mut self, size: i64) -> Result<Series> {
        match &mut self.kind {
            SeriesKind::Point(options) => {
                options.size = px("size", size)?;
                Ok(self)
            }
            _ => Err(not_applicable(
                "size",
                "the size argument of PointSeries::new and applies to point_series",
                &self,
            )),
        }
    }

    /// `marker(name)`: the element type of `PointSeries::new` (`Circle`, `Cross`,
    /// `TriangleMarker`, `Pixel`).
    pub fn marker(mut self, name: &str) -> Result<Series> {
        match &mut self.kind {
            SeriesKind::Point(options) => {
                options.marker = Marker::parse(name)?;
                Ok(self)
            }
            _ => Err(not_applicable(
                "marker",
                "the element type of PointSeries::new and applies to point_series",
                &self,
            )),
        }
    }

    /// `Histogram::margin`: px left free on each side of a bar.
    pub fn margin(mut self, margin: i64) -> Result<Series> {
        match &mut self.kind {
            SeriesKind::Histogram(options) | SeriesKind::HistogramHorizontal(options) => {
                options.margin = px("margin", margin)?;
                Ok(self)
            }
            _ => Err(not_applicable(
                "margin",
                "Histogram::margin and applies to histogram_vertical and histogram_horizontal",
                &self,
            )),
        }
    }

    /// `Histogram::baseline`: where the bars start, in the units of the value axis.
    pub fn baseline(mut self, baseline: f64) -> Result<Series> {
        if !baseline.is_finite() {
            return Err(Error::invalid(format!(
                "baseline must be finite, got {baseline}"
            )));
        }
        match &mut self.kind {
            SeriesKind::Histogram(options) | SeriesKind::HistogramHorizontal(options) => {
                options.baseline = baseline;
                Ok(self)
            }
            _ => Err(not_applicable(
                "baseline",
                "Histogram::baseline and applies to histogram_vertical and histogram_horizontal",
                &self,
            )),
        }
    }
}

impl Marker {
    pub(crate) const NAMED: [(&'static str, Marker); 4] = [
        ("circle", Marker::Circle),
        ("cross", Marker::Cross),
        ("triangle", Marker::Triangle),
        ("pixel", Marker::Pixel),
    ];

    /// A marker name, case-insensitive.
    pub fn parse(name: &str) -> Result<Marker> {
        let key = name.trim().to_ascii_lowercase();
        Marker::NAMED
            .iter()
            .find(|(n, _)| *n == key)
            .map(|(_, m)| *m)
            .ok_or_else(|| {
                Error::invalid(format!(
                    "unknown PointSeries marker '{name}': expected 'circle' (Circle), \
                     'cross' (Cross), 'triangle' (TriangleMarker) or 'pixel' (Pixel)"
                ))
            })
    }
}

impl Default for Chart {
    fn default() -> Self {
        Chart {
            caption: None,
            margin: Sides {
                top: 10,
                bottom: 10,
                left: 10,
                right: 10,
            },
            label_area: Sides {
                top: 0,
                bottom: 30,
                left: 40,
                right: 0,
            },
            x_axis: AxisSpec {
                range: None,
                scale: Scale::Linear,
            },
            y_axis: AxisSpec {
                range: None,
                scale: Scale::Linear,
            },
            fill: Color::WHITE,
            ops: Vec::new(),
        }
    }
}

impl Chart {
    /// `chart()`: `ChartBuilder::on(&root)` with duckers' defaults.
    pub fn new() -> Chart {
        Chart::default()
    }

    /// `caption(text [, size])`: `ChartBuilder::caption(text, ("sans-serif", size))`.
    pub fn caption(self, text: impl Into<String>, size: Option<i64>) -> Result<Chart> {
        let font = Font {
            size: match size {
                Some(size) => font_size(size)?,
                None => DEFAULT_CAPTION_SIZE,
            },
            ..Font::default()
        };
        Ok(self.caption_font(text, font))
    }

    /// `caption(text, font)`: `ChartBuilder::caption(text, font)`.
    pub fn caption_font(mut self, text: impl Into<String>, font: Font) -> Chart {
        self.caption = Some(Caption {
            text: text.into(),
            font,
        });
        self
    }

    /// `margin(px)`: `ChartBuilder::margin`, all four sides.
    pub fn margin(mut self, px_value: i64) -> Result<Chart> {
        let m = px("margin", px_value)?;
        self.margin = Sides {
            top: m,
            bottom: m,
            left: m,
            right: m,
        };
        Ok(self)
    }

    /// `margin_top(px)`: `ChartBuilder::margin_top`.
    pub fn margin_top(mut self, size: i64) -> Result<Chart> {
        self.margin.top = px("margin_top", size)?;
        Ok(self)
    }

    /// `margin_bottom(px)`: `ChartBuilder::margin_bottom`.
    pub fn margin_bottom(mut self, size: i64) -> Result<Chart> {
        self.margin.bottom = px("margin_bottom", size)?;
        Ok(self)
    }

    /// `margin_left(px)`: `ChartBuilder::margin_left`.
    pub fn margin_left(mut self, size: i64) -> Result<Chart> {
        self.margin.left = px("margin_left", size)?;
        Ok(self)
    }

    /// `margin_right(px)`: `ChartBuilder::margin_right`.
    pub fn margin_right(mut self, size: i64) -> Result<Chart> {
        self.margin.right = px("margin_right", size)?;
        Ok(self)
    }

    /// `x_label_area_size(px)`: `ChartBuilder::x_label_area_size`, the bottom label area.
    pub fn x_label_area_size(mut self, size: i64) -> Result<Chart> {
        self.label_area.bottom = px("x_label_area_size", size)?;
        Ok(self)
    }

    /// `y_label_area_size(px)`: `ChartBuilder::y_label_area_size`, the left label area.
    pub fn y_label_area_size(mut self, size: i64) -> Result<Chart> {
        self.label_area.left = px("y_label_area_size", size)?;
        Ok(self)
    }

    /// `top_x_label_area_size(px)`: `ChartBuilder::top_x_label_area_size`.
    pub fn top_x_label_area_size(mut self, size: i64) -> Result<Chart> {
        self.label_area.top = px("top_x_label_area_size", size)?;
        Ok(self)
    }

    /// `right_y_label_area_size(px)`: `ChartBuilder::right_y_label_area_size`.
    pub fn right_y_label_area_size(mut self, size: i64) -> Result<Chart> {
        self.label_area.right = px("right_y_label_area_size", size)?;
        Ok(self)
    }

    /// `set_all_label_area_size(px)`: `ChartBuilder::set_all_label_area_size`, all four.
    pub fn set_all_label_area_size(mut self, size: i64) -> Result<Chart> {
        let s = px("set_all_label_area_size", size)?;
        self.label_area = Sides {
            top: s,
            bottom: s,
            left: s,
            right: s,
        };
        Ok(self)
    }

    /// `set_left_and_bottom_label_area_size(px)`:
    /// `ChartBuilder::set_left_and_bottom_label_area_size`.
    pub fn set_left_and_bottom_label_area_size(mut self, size: i64) -> Result<Chart> {
        let s = px("set_left_and_bottom_label_area_size", size)?;
        self.label_area.left = s;
        self.label_area.bottom = s;
        Ok(self)
    }

    /// `x_log_scale([base])`: `(lo..hi).log_scale().base(base)` for the x range.
    pub fn x_log_scale(self, base: Option<f64>) -> Result<Chart> {
        self.log_scale(Axis::X, base)
    }

    /// `y_log_scale([base])`: `(lo..hi).log_scale().base(base)` for the y range.
    pub fn y_log_scale(self, base: Option<f64>) -> Result<Chart> {
        self.log_scale(Axis::Y, base)
    }

    fn log_scale(mut self, axis: Axis, base: Option<f64>) -> Result<Chart> {
        let what = format!("{}_log_scale", axis.name());
        let base = base.unwrap_or(10.0);
        if !(base.is_finite() && base > 1.0) {
            return Err(Error::invalid(format!(
                "{what}: LogRangeExt::base must be a finite number above 1, got {base}"
            )));
        }
        let scale = Scale::Log { base };
        let kind = self
            .kind_on(axis)
            .or(self.axis(axis).range.map(AxisRange::kind));
        if let Some(kind) = kind {
            check_scale_fits(&what, axis, scale, kind)?;
        }
        self.axis_mut(axis).scale = scale;
        Ok(self)
    }

    /// `x_range(lo, hi)`: the x range argument of `build_cartesian_2d`.
    pub fn x_range(self, lo: RangeValue, hi: RangeValue) -> Result<Chart> {
        self.range(Axis::X, lo, hi)
    }

    /// `y_range(lo, hi)`: the y range argument of `build_cartesian_2d`.
    pub fn y_range(self, lo: RangeValue, hi: RangeValue) -> Result<Chart> {
        self.range(Axis::Y, lo, hi)
    }

    fn range(mut self, axis: Axis, lo: RangeValue, hi: RangeValue) -> Result<Chart> {
        let what = format!("{}_range", axis.name());
        let range = axis_range(&what, lo, hi)?;
        if let Some(kind) = self.kind_on(axis) {
            check_range_fits(&what, axis.name(), range, kind)?;
        }
        let kind = self.kind_on(axis).unwrap_or(range.kind());
        let scale = self.axis(axis).scale;
        check_scale_fits(&format!("{}_log_scale", axis.name()), axis, scale, kind)?;
        self.axis_mut(axis).range = Some(range);
        Ok(self)
    }

    /// `root_fill(color)`: `root.fill(&color)`.
    pub fn fill(mut self, color: &str) -> Result<Chart> {
        self.fill = Color::parse(color)?;
        Ok(self)
    }

    /// `draw_series(series)`: `ChartContext::draw_series`.
    pub fn draw_series(mut self, series: Series) -> Result<Chart> {
        self.check_series(&series)?;
        self.ops.push(DrawOp::Series(series));
        Ok(self)
    }

    /// `draw_series(series[])`: one `draw_series` call per element, in list order.
    pub fn draw_series_list(self, series: impl IntoIterator<Item = Series>) -> Result<Chart> {
        series.into_iter().try_fold(self, Chart::draw_series)
    }

    /// `configure_mesh()`.
    pub fn configure_mesh(self) -> Mesh {
        Mesh {
            chart: self,
            style: MeshStyle::default(),
        }
    }

    /// `configure_series_labels()`.
    pub fn configure_series_labels(self) -> SeriesLabels {
        SeriesLabels {
            chart: self,
            style: SeriesLabelStyle::default(),
        }
    }

    fn check_series(&self, series: &Series) -> Result<()> {
        for axis in [Axis::X, Axis::Y] {
            let (name, new) = (axis.name(), series.kind_on(axis));
            if let Some(kind) = self.kind_on(axis)
                && kind != new
            {
                return Err(Error::invalid(format!(
                    "draw_series: cannot draw {} with {new} {name} values on a chart whose {name} \
                     axis is {kind}",
                    series.kind.aggregate_name()
                )));
            }
        }
        for axis in [Axis::X, Axis::Y] {
            if let Some(range) = self.axis(axis).range {
                let what = format!("draw_series: {}_range", axis.name());
                check_range_fits(&what, axis.name(), range, series.kind_on(axis))?;
            }
            let what = format!("draw_series: {}_log_scale", axis.name());
            check_scale_fits(&what, axis, self.axis(axis).scale, series.kind_on(axis))?;
        }
        Ok(())
    }
}

/// Checks that a scale suits the kind of its axis: a log scale needs a numeric axis.
pub(crate) fn check_scale_fits(what: &str, axis: Axis, scale: Scale, kind: AxisKind) -> Result<()> {
    match scale {
        Scale::Log { .. } if kind != AxisKind::Numeric => Err(Error::invalid(format!(
            "{what} does not apply to the {kind} {} axis: plotters' log_scale (LogCoord) needs a \
             numeric axis",
            axis.name()
        ))),
        _ => Ok(()),
    }
}

fn axis_range(what: &str, lo: RangeValue, hi: RangeValue) -> Result<AxisRange> {
    let range = match (lo, hi) {
        (RangeValue::Number(lo), RangeValue::Number(hi)) => {
            if !lo.is_finite() || !hi.is_finite() {
                return Err(Error::invalid(format!(
                    "{what} bounds must be finite, got {lo} and {hi}"
                )));
            }
            AxisRange::Numeric(lo, hi)
        }
        (RangeValue::Date(lo), RangeValue::Date(hi)) => AxisRange::Date(lo, hi),
        (RangeValue::Timestamp(lo), RangeValue::Timestamp(hi)) => AxisRange::Timestamp(lo, hi),
        _ => {
            return Err(Error::invalid(format!(
                "{what} bounds must have the same type, got {} and {}",
                lo.kind_name(),
                hi.kind_name()
            )));
        }
    };
    let empty = match range {
        AxisRange::Numeric(lo, hi) => lo == hi,
        AxisRange::Date(lo, hi) => lo == hi,
        AxisRange::Timestamp(lo, hi) => lo == hi,
    };
    if empty {
        return Err(Error::invalid(format!(
            "{what} needs lo <> hi: plotters cannot map a range whose start equals its end \
             (lo > hi reverses the axis)"
        )));
    }
    Ok(range)
}

fn check_range_fits(what: &str, axis: &str, range: AxisRange, kind: AxisKind) -> Result<()> {
    let fits = match (range, kind) {
        (AxisRange::Numeric(..), AxisKind::Numeric) => true,
        (AxisRange::Numeric(lo, hi), AxisKind::Integer) => {
            if lo.fract() != 0.0 || hi.fract() != 0.0 {
                return Err(Error::invalid(format!(
                    "{what} on an integer bucket axis takes whole numbers, got {lo} and {hi}"
                )));
            }
            true
        }
        (AxisRange::Date(..), AxisKind::Date) => true,
        (AxisRange::Timestamp(..), AxisKind::Timestamp) => true,
        _ => false,
    };
    if fits {
        Ok(())
    } else if kind == AxisKind::Category {
        Err(Error::invalid(format!(
            "{what} does not apply to the category {axis} axis: its categories are the data's"
        )))
    } else {
        Err(Error::invalid(format!(
            "{what} bounds are {} but the {axis} axis is {kind}",
            range.kind_name()
        )))
    }
}

impl Mesh {
    /// `MeshStyle::x_desc`.
    pub fn x_desc(mut self, text: impl Into<String>) -> Mesh {
        self.style.settings.push(MeshSetting::XDesc(text.into()));
        self
    }

    /// `MeshStyle::y_desc`.
    pub fn y_desc(mut self, text: impl Into<String>) -> Mesh {
        self.style.settings.push(MeshSetting::YDesc(text.into()));
        self
    }

    /// `MeshStyle::draw`: draws the mesh at this point of the chain and returns the chart.
    pub fn draw(self) -> Chart {
        let mut chart = self.chart;
        chart.ops.push(DrawOp::Mesh(self.style));
        chart
    }
}

impl SeriesLabels {
    /// `SeriesLabelStyle::position` with a named `SeriesLabelPosition`.
    pub fn position(mut self, name: &str) -> Result<SeriesLabels> {
        let key = name.trim().to_ascii_lowercase();
        let position = LabelPosition::NAMED
            .iter()
            .find(|(n, _)| *n == key)
            .map(|(_, p)| *p)
            .ok_or_else(|| {
                let names: Vec<String> = LabelPosition::NAMED
                    .iter()
                    .map(|(n, _)| format!("'{n}'"))
                    .collect();
                Error::invalid(format!(
                    "unknown SeriesLabelPosition '{name}': expected one of {}, or position(x, y)",
                    names.join(", ")
                ))
            })?;
        self.style
            .settings
            .push(SeriesLabelSetting::Position(position));
        Ok(self)
    }

    /// `SeriesLabelStyle::position(SeriesLabelPosition::Coordinate(x, y))`, px from the
    /// plotting-area origin.
    pub fn position_at(mut self, x: i64, y: i64) -> Result<SeriesLabels> {
        let coord =
            |v: i64| {
                i32::try_from(v).ok().filter(|v| v.unsigned_abs() <= MAX_PX).ok_or_else(|| {
                Error::invalid(format!(
                    "SeriesLabelPosition::Coordinate takes px between -{MAX_PX} and {MAX_PX}, \
                     got {v}"
                ))
            })
            };
        let position = LabelPosition::Coordinate(coord(x)?, coord(y)?);
        self.style
            .settings
            .push(SeriesLabelSetting::Position(position));
        Ok(self)
    }

    /// `SeriesLabelStyle::border_style(color [, stroke_width])`.
    pub fn border_style(mut self, color: &str, stroke_width: Option<i64>) -> Result<SeriesLabels> {
        let color = Color::parse(color)?;
        let stroke_width = match stroke_width {
            Some(w) => px("stroke_width", w)?,
            None => 1,
        };
        self.style.settings.push(SeriesLabelSetting::BorderStyle {
            color,
            stroke_width,
        });
        Ok(self)
    }

    /// `SeriesLabelStyle::background_style(color)`.
    pub fn background_style(mut self, color: &str) -> Result<SeriesLabels> {
        let color = Color::parse(color)?;
        self.style
            .settings
            .push(SeriesLabelSetting::BackgroundStyle(color));
        Ok(self)
    }

    /// `SeriesLabelStyle::draw`: draws the legend at this point of the chain and returns the
    /// chart.
    pub fn draw(self) -> Chart {
        let mut chart = self.chart;
        chart.ops.push(DrawOp::SeriesLabels(self.style));
        chart
    }
}

fn font_size(size: i64) -> Result<u32> {
    match px("font size", size)? {
        0 => Err(Error::invalid("font size must be at least 1 px")),
        size => Ok(size),
    }
}

impl Default for Font {
    /// `("sans-serif", 12)`, plotters' default text style.
    fn default() -> Self {
        Font {
            family: "sans-serif".into(),
            size: 12,
            style: FontStyle::Normal,
            color: None,
        }
    }
}

impl Font {
    /// `font(family, size [, style])`: `FontDesc::new(family, size, style)`.
    pub fn new(family: impl Into<String>, size: i64, style: Option<&str>) -> Result<Font> {
        let family = family.into();
        if family.trim().is_empty() {
            return Err(Error::invalid("font family must not be empty"));
        }
        let style = match style.map(|s| s.trim().to_ascii_lowercase()) {
            None => FontStyle::Normal,
            Some(s) => match s.as_str() {
                "normal" => FontStyle::Normal,
                "bold" => FontStyle::Bold,
                "italic" => FontStyle::Italic,
                "oblique" => FontStyle::Oblique,
                _ => {
                    return Err(Error::invalid(format!(
                        "unknown FontStyle '{s}': expected 'normal', 'bold', 'italic' or 'oblique'"
                    )));
                }
            },
        };
        Ok(Font {
            family,
            size: font_size(size)?,
            style,
            color: None,
        })
    }

    /// `color(font, color)`: `FontDesc::color`.
    pub fn color(mut self, color: &str) -> Result<Font> {
        self.color = Some(Color::parse(color)?);
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Column, HistogramOptions, LineOptions, Style};

    fn line(x: Column) -> Series {
        Series {
            kind: SeriesKind::Line(LineOptions { point_size: 0 }),
            y: Column::Numeric(vec![1.0; x.len()]),
            x,
            style: Style {
                color: None,
                stroke_width: 1,
                filled: false,
            },
            label: None,
        }
    }

    fn histogram(x: Column) -> Series {
        Series {
            kind: SeriesKind::Histogram(HistogramOptions {
                margin: 5,
                baseline: 0.0,
            }),
            ..line(x)
        }
    }

    #[test]
    fn range_errors() {
        let n = RangeValue::Number;
        let err = Chart::new().x_range(n(1.0), n(1.0)).unwrap_err();
        assert!(err.message().starts_with("x_range needs lo <> hi"), "{err}");
        let err = Chart::new()
            .y_range(n(1.0), RangeValue::Date(3))
            .unwrap_err();
        assert_eq!(
            err.message(),
            "y_range bounds must have the same type, got numeric and date"
        );
        let err = Chart::new()
            .draw_series(line(Column::Numeric(vec![1.0])))
            .unwrap()
            .y_range(RangeValue::Date(1), RangeValue::Date(3))
            .unwrap_err();
        assert_eq!(
            err.message(),
            "y_range bounds are date but the y axis is numeric"
        );
        // Before any series the y axis kind is open, as the x axis kind is.
        let err = Chart::new()
            .y_range(RangeValue::Date(1), RangeValue::Date(3))
            .unwrap()
            .draw_series(line(Column::Numeric(vec![1.0])))
            .unwrap_err();
        assert_eq!(
            err.message(),
            "draw_series: y_range bounds are date but the y axis is numeric"
        );
        assert!(Chart::new().x_range(n(f64::NAN), n(1.0)).is_err());
        // Reversed ranges are allowed.
        assert!(Chart::new().x_range(n(10.0), n(0.0)).is_ok());
    }

    #[test]
    fn range_must_fit_the_series() {
        let chart = Chart::new()
            .draw_series(line(Column::Date(vec![1, 2])))
            .unwrap();
        let err = chart
            .clone()
            .x_range(RangeValue::Number(0.0), RangeValue::Number(1.0))
            .unwrap_err();
        assert_eq!(
            err.message(),
            "x_range bounds are numeric but the x axis is date"
        );
        assert!(
            chart
                .x_range(RangeValue::Date(0), RangeValue::Date(5))
                .is_ok()
        );

        let err = Chart::new()
            .x_range(RangeValue::Number(0.0), RangeValue::Number(1.0))
            .unwrap()
            .draw_series(line(Column::Timestamp(vec![1])))
            .unwrap_err();
        assert_eq!(
            err.message(),
            "draw_series: x_range bounds are numeric but the x axis is timestamp"
        );

        let err = Chart::new()
            .draw_series(histogram(Column::Category(vec!["a".into()])))
            .unwrap()
            .x_range(RangeValue::Number(0.0), RangeValue::Number(1.0))
            .unwrap_err();
        assert!(err.message().contains("category x axis"), "{err}");

        let integer = Chart::new()
            .draw_series(histogram(Column::Integer(vec![1, 2])))
            .unwrap();
        assert!(
            integer
                .clone()
                .x_range(RangeValue::Number(0.0), RangeValue::Number(5.0))
                .is_ok()
        );
        assert!(
            integer
                .x_range(RangeValue::Number(0.5), RangeValue::Number(5.0))
                .is_err()
        );
    }

    #[test]
    fn mixed_axis_kinds_name_both() {
        let err = Chart::new()
            .draw_series(line(Column::Numeric(vec![1.0])))
            .unwrap()
            .draw_series(line(Column::Date(vec![1])))
            .unwrap_err();
        assert_eq!(
            err.message(),
            "draw_series: cannot draw line_series with date x values on a chart whose x axis \
             is numeric"
        );
    }

    #[test]
    fn y_axis_kinds_must_agree() {
        let horizontal = Series {
            kind: SeriesKind::HistogramHorizontal(HistogramOptions {
                margin: 5,
                baseline: 0.0,
            }),
            x: Column::Numeric(vec![1.0]),
            y: Column::Category(vec!["a".into()]),
            ..line(Column::Numeric(vec![1.0]))
        };
        let chart = Chart::new().draw_series(horizontal).unwrap();
        let err = chart
            .clone()
            .draw_series(line(Column::Numeric(vec![1.0])))
            .unwrap_err();
        assert_eq!(
            err.message(),
            "draw_series: cannot draw line_series with numeric y values on a chart whose y axis \
             is category"
        );
        let err = chart
            .y_range(RangeValue::Number(0.0), RangeValue::Number(1.0))
            .unwrap_err();
        assert!(err.message().contains("category y axis"), "{err}");
    }

    #[test]
    fn markers_margins_and_baselines() {
        let point = Series {
            kind: SeriesKind::Point(crate::spec::PointOptions {
                size: 3,
                marker: Marker::Circle,
            }),
            ..line(Column::Numeric(vec![]))
        };
        for (name, marker) in [
            ("Cross", Marker::Cross),
            (" triangle", Marker::Triangle),
            ("PIXEL", Marker::Pixel),
            ("circle", Marker::Circle),
        ] {
            let s = point.clone().marker(name).unwrap();
            assert!(matches!(s.kind, SeriesKind::Point(o) if o.marker == marker));
        }
        let err = point.clone().marker("star").unwrap_err();
        assert!(
            err.message()
                .starts_with("unknown PointSeries marker 'star': expected 'circle'"),
            "{err}"
        );
        let err = line(Column::Numeric(vec![])).marker("cross").unwrap_err();
        assert_eq!(
            err.message(),
            "marker does not apply to line_series values: it is the element type of \
             PointSeries::new and applies to point_series"
        );

        let h = histogram(Column::Integer(vec![1]))
            .margin(0)
            .unwrap()
            .baseline(-2.5)
            .unwrap();
        assert_eq!(
            h.kind,
            SeriesKind::Histogram(HistogramOptions {
                margin: 0,
                baseline: -2.5
            })
        );
        assert!(histogram(Column::Integer(vec![1])).margin(-1).is_err());
        assert!(
            histogram(Column::Integer(vec![1]))
                .baseline(f64::NAN)
                .is_err()
        );
        let err = point.clone().margin(3).unwrap_err();
        assert_eq!(
            err.message(),
            "margin does not apply to point_series values: it is Histogram::margin and applies \
             to histogram_vertical and histogram_horizontal"
        );
        assert!(point.baseline(1.0).is_err());
    }

    #[test]
    fn methods_check_the_series_kind() {
        let err = histogram(Column::Integer(vec![1]))
            .point_size(3)
            .unwrap_err();
        assert_eq!(
            err.message(),
            "point_size does not apply to histogram_vertical values: it is LineSeries::point_size \
             and applies to line_series"
        );
        assert!(line(Column::Numeric(vec![])).size(3).is_err());
        assert!(line(Column::Numeric(vec![])).point_size(3).is_ok());
        assert!(line(Column::Numeric(vec![])).stroke_width(-1).is_err());
    }

    #[test]
    fn style_parses_the_colour() {
        let s = line(Column::Numeric(vec![])).style("red", Some(2)).unwrap();
        assert_eq!(s.style.color, Some(Color::rgb(255, 0, 0)));
        assert_eq!(s.style.stroke_width, 2);
        let err = line(Column::Numeric(vec![]))
            .style("nope", None)
            .unwrap_err();
        assert!(err.message().starts_with("unknown colour 'nope'"));
    }

    #[test]
    fn chain_order_is_kept() {
        let chart = Chart::new()
            .configure_mesh()
            .x_desc("x")
            .draw()
            .draw_series(line(Column::Numeric(vec![1.0])))
            .unwrap()
            .configure_series_labels()
            .position("upper_left")
            .unwrap()
            .draw();
        assert!(matches!(
            chart.ops.as_slice(),
            [DrawOp::Mesh(_), DrawOp::Series(_), DrawOp::SeriesLabels(_)]
        ));
    }

    #[test]
    fn label_positions() {
        let labels = Chart::new().configure_series_labels();
        assert!(labels.clone().position("Upper_Left").is_ok());
        let err = labels.clone().position("top").unwrap_err();
        assert!(
            err.message()
                .starts_with("unknown SeriesLabelPosition 'top'"),
            "{err}"
        );
        assert!(labels.clone().position_at(10, -20).is_ok());
        assert!(labels.position_at(1 << 40, 0).is_err());
    }

    #[test]
    fn builder_sizes() {
        let chart = Chart::new()
            .margin(4)
            .unwrap()
            .x_label_area_size(50)
            .unwrap()
            .y_label_area_size(60)
            .unwrap();
        assert_eq!(chart.margin.left, 4);
        assert_eq!(chart.margin.top, 4);
        assert_eq!(chart.label_area.bottom, 50);
        assert_eq!(chart.label_area.left, 60);
        let err = Chart::new().margin(-1).unwrap_err();
        assert_eq!(
            err.message(),
            "margin must be between 0 and 8192 px, got -1"
        );
    }

    #[test]
    fn every_builder_size() {
        let chart = Chart::new()
            .margin_top(1)
            .unwrap()
            .margin_bottom(2)
            .unwrap()
            .margin_left(3)
            .unwrap()
            .margin_right(4)
            .unwrap()
            .top_x_label_area_size(5)
            .unwrap()
            .right_y_label_area_size(6)
            .unwrap();
        assert_eq!(
            chart.margin,
            Sides {
                top: 1,
                bottom: 2,
                left: 3,
                right: 4
            }
        );
        assert_eq!(
            chart.label_area,
            Sides {
                top: 5,
                bottom: 30,
                left: 40,
                right: 6
            }
        );
        let all = chart.clone().set_all_label_area_size(7).unwrap();
        assert_eq!(
            all.label_area,
            Sides {
                top: 7,
                bottom: 7,
                left: 7,
                right: 7
            }
        );
        let lb = chart.set_left_and_bottom_label_area_size(8).unwrap();
        assert_eq!(
            lb.label_area,
            Sides {
                top: 5,
                bottom: 8,
                left: 8,
                right: 6
            }
        );
        let err = Chart::new().right_y_label_area_size(-1).unwrap_err();
        assert_eq!(
            err.message(),
            "right_y_label_area_size must be between 0 and 8192 px, got -1"
        );
    }

    #[test]
    fn log_scales() {
        let chart = Chart::new().x_log_scale(None).unwrap();
        assert_eq!(chart.x_axis.scale, Scale::Log { base: 10.0 });
        let chart = chart.y_log_scale(Some(2.0)).unwrap();
        assert_eq!(chart.y_axis.scale, Scale::Log { base: 2.0 });
        for base in [1.0, 0.5, f64::INFINITY, f64::NAN] {
            let err = Chart::new().x_log_scale(Some(base)).unwrap_err();
            assert!(
                err.message()
                    .starts_with("x_log_scale: LogRangeExt::base must be a finite number above 1"),
                "{err}"
            );
        }

        // Known axis kinds are checked at the call ...
        let dates = Chart::new()
            .draw_series(line(Column::Date(vec![1])))
            .unwrap();
        let err = dates.x_log_scale(None).unwrap_err();
        assert_eq!(
            err.message(),
            "x_log_scale does not apply to the date x axis: plotters' log_scale (LogCoord) \
             needs a numeric axis"
        );
        let err = Chart::new()
            .x_range(RangeValue::Date(1), RangeValue::Date(5))
            .unwrap()
            .x_log_scale(None)
            .unwrap_err();
        assert!(err.message().contains("the date x axis"), "{err}");
        let err = Chart::new()
            .x_log_scale(None)
            .unwrap()
            .x_range(RangeValue::Timestamp(1), RangeValue::Timestamp(5))
            .unwrap_err();
        assert!(err.message().contains("the timestamp x axis"), "{err}");

        // ... and otherwise at draw_series.
        let err = Chart::new()
            .x_log_scale(None)
            .unwrap()
            .draw_series(histogram(Column::Category(vec!["a".into()])))
            .unwrap_err();
        assert_eq!(
            err.message(),
            "draw_series: x_log_scale does not apply to the category x axis: plotters' \
             log_scale (LogCoord) needs a numeric axis"
        );
        let err = Chart::new()
            .draw_series(histogram(Column::Integer(vec![1])))
            .unwrap()
            .x_log_scale(None)
            .unwrap_err();
        assert!(err.message().contains("the integer bucket x axis"), "{err}");
        // The value axis of a histogram is numeric.
        assert!(
            Chart::new()
                .draw_series(histogram(Column::Integer(vec![1])))
                .unwrap()
                .y_log_scale(None)
                .is_ok()
        );
    }

    #[test]
    fn legend_styles() {
        let labels = Chart::new()
            .configure_series_labels()
            .border_style("black", Some(2))
            .unwrap()
            .background_style("white")
            .unwrap();
        assert_eq!(
            labels.style.settings,
            [
                SeriesLabelSetting::BorderStyle {
                    color: Color::BLACK,
                    stroke_width: 2
                },
                SeriesLabelSetting::BackgroundStyle(Color::WHITE)
            ]
        );
        assert!(
            Chart::new()
                .configure_series_labels()
                .border_style("nope", None)
                .is_err()
        );
    }

    #[test]
    fn fonts() {
        let f = Font::new("serif", 20, Some("Bold")).unwrap();
        assert_eq!(f.style, FontStyle::Bold);
        assert!(Font::new("serif", 0, None).is_err());
        assert!(Font::new("serif", 10, Some("heavy")).is_err());
        assert!(f.color("blue").is_ok());
    }
}
