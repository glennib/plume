//! The SQL methods, as functions on the spec types.
//!
//! Each takes the value first and returns a new value, as the SQL functions do. Integer
//! arguments are taken as `i64`, as SQL passes them, and checked here, so every range error
//! reads the same whichever SQL integer type the caller used.

use crate::color::Color;
use crate::error::{Error, Result};
use crate::spec::{
    AxisKind, AxisRange, AxisSpec, Caption, Chart, DrawOp, Font, FontStyle, LabelPosition, Mesh,
    MeshSetting, MeshStyle, Scale, Series, SeriesKind, SeriesLabelSetting, SeriesLabelStyle,
    SeriesLabels, Sides,
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
            SeriesKind::Line(_) | SeriesKind::Point(_) | SeriesKind::Histogram(_) => {
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

    /// `x_range(lo, hi)`: the x range argument of `build_cartesian_2d`.
    pub fn x_range(mut self, lo: RangeValue, hi: RangeValue) -> Result<Chart> {
        let range = axis_range("x_range", lo, hi)?;
        if let Some(kind) = self.x_kind() {
            check_range_fits("x_range", "x", range, kind)?;
        }
        self.x_axis.range = Some(range);
        Ok(self)
    }

    /// `y_range(lo, hi)`: the y range argument of `build_cartesian_2d`.
    pub fn y_range(mut self, lo: RangeValue, hi: RangeValue) -> Result<Chart> {
        let range = axis_range("y_range", lo, hi)?;
        check_range_fits("y_range", "y", range, AxisKind::Numeric)?;
        self.y_axis.range = Some(range);
        Ok(self)
    }

    /// `fill(color)`: `root.fill(&color)`.
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
        let x = series.x_kind();
        if let Some(kind) = self.x_kind()
            && kind != x
        {
            return Err(Error::invalid(format!(
                "draw_series: cannot draw {} with {x} x values on a chart whose x axis is {kind}",
                series.kind.aggregate_name()
            )));
        }
        if let Some(range) = self.x_axis.range {
            check_range_fits("draw_series: x_range", "x", range, x)?;
        }
        if let Some(range) = self.y_axis.range {
            check_range_fits("draw_series: y_range", "y", range, series.y_kind())?;
        }
        Ok(())
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
            y: vec![1.0; x.len()],
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
            .y_range(RangeValue::Date(1), RangeValue::Date(3))
            .unwrap_err();
        assert_eq!(
            err.message(),
            "y_range bounds are date but the y axis is numeric"
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
    fn methods_check_the_series_kind() {
        let err = histogram(Column::Integer(vec![1]))
            .point_size(3)
            .unwrap_err();
        assert_eq!(
            err.message(),
            "point_size does not apply to histogram values: it is LineSeries::point_size and \
             applies to line_series"
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
    fn fonts() {
        let f = Font::new("serif", 20, Some("Bold")).unwrap();
        assert_eq!(f.style, FontStyle::Bold);
        assert!(Font::new("serif", 0, None).is_err());
        assert!(Font::new("serif", 10, Some("heavy")).is_err());
        assert!(f.color("blue").is_ok());
    }
}
