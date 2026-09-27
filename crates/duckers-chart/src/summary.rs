//! One-line summaries, the `VARCHAR` casts of the SQL values.

use crate::spec::{
    Chart, Column, Font, FontStyle, LabelPosition, LineStyle, Mesh, MeshSetting, Series,
    SeriesKind, SeriesLabelSetting, SeriesLabels, bin,
};
use std::collections::BTreeSet;

/// A SQL-style string literal: single quotes, embedded quotes doubled.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// A line style as `#rrggbb 2px`.
fn line(style: &LineStyle) -> String {
    format!("{} {}px", style.color, style.stroke_width)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Series {
    /// `SERIES(line 'temp', 120 points)`, `SERIES(histogram, 5 buckets)`.
    pub fn summary(&self) -> String {
        let mut out = format!("SERIES({}", self.kind.name());
        if let Some(label) = &self.label {
            out.push(' ');
            out.push_str(&quote(label));
        }
        let count = match &self.kind {
            SeriesKind::Histogram(o) | SeriesKind::HistogramHorizontal(o) => {
                // Stepped buckets count their bins.
                let bins = o.step.and_then(|step| {
                    let axis = self.bucket_axis()?;
                    let Column::Numeric(v) = self.column(axis) else {
                        return None;
                    };
                    let bins: BTreeSet<i64> = v.iter().filter_map(|v| bin(*v, step)).collect();
                    Some(bins.len())
                });
                plural(bins.unwrap_or(self.len()), "bucket", "buckets")
            }
            SeriesKind::ErrorBarVertical(_) | SeriesKind::ErrorBarHorizontal(_) => {
                plural(self.len(), "bar", "bars")
            }
            SeriesKind::CandleStick(_) => plural(self.len(), "candle", "candles"),
            SeriesKind::BoxplotVertical(_) | SeriesKind::BoxplotHorizontal(_) => {
                plural(self.len(), "box", "boxes")
            }
            _ => plural(self.len(), "point", "points"),
        };
        out.push_str(&format!(", {count})"));
        out
    }
}

impl Chart {
    /// `CHART(line, 2 series, 240 points)`, `CHART(line+point, 2 series, 20 points, caption
    /// 'x')`, `CHART(empty)`.
    pub fn summary(&self) -> String {
        let mut kinds: Vec<&str> = Vec::new();
        let mut count = 0;
        let mut points = 0;
        for series in self.series() {
            if !kinds.contains(&series.kind.name()) {
                kinds.push(series.kind.name());
            }
            count += 1;
            points += series.len();
        }
        let mut parts = Vec::new();
        if count == 0 {
            parts.push("empty".to_string());
        } else {
            parts.push(kinds.join("+"));
            parts.push(plural(count, "series", "series"));
            parts.push(plural(points, "point", "points"));
        }
        if let Some(caption) = &self.caption {
            parts.push(format!("caption {}", quote(&caption.text)));
        }
        format!("CHART({})", parts.join(", "))
    }
}

impl Mesh {
    /// `MESH(x_desc 'day', y_desc '°C') of CHART(...)`.
    pub fn summary(&self) -> String {
        let settings: Vec<String> = self
            .style
            .settings
            .iter()
            .map(|s| match s {
                MeshSetting::XDesc(text) => format!("x_desc {}", quote(text)),
                MeshSetting::YDesc(text) => format!("y_desc {}", quote(text)),
                MeshSetting::AxisDescStyle(f) => format!("axis_desc_style {}", f.summary()),
                MeshSetting::XLabels(n) => format!("x_labels {n}"),
                MeshSetting::YLabels(n) => format!("y_labels {n}"),
                MeshSetting::XLabelFormatter(f) => format!("x_label_formatter {}", quote(f)),
                MeshSetting::YLabelFormatter(f) => format!("y_label_formatter {}", quote(f)),
                MeshSetting::LabelStyle(f) => format!("label_style {}", f.summary()),
                MeshSetting::XLabelStyle(f) => format!("x_label_style {}", f.summary()),
                MeshSetting::YLabelStyle(f) => format!("y_label_style {}", f.summary()),
                MeshSetting::XLabelOffset(px) => format!("x_label_offset {px}px"),
                MeshSetting::YLabelOffset(px) => format!("y_label_offset {px}px"),
                MeshSetting::XMaxLightLines(n) => format!("x_max_light_lines {n}"),
                MeshSetting::YMaxLightLines(n) => format!("y_max_light_lines {n}"),
                MeshSetting::MaxLightLines(n) => format!("max_light_lines {n}"),
                MeshSetting::LightLineStyle(l) => format!("light_line_style {}", line(l)),
                MeshSetting::BoldLineStyle(l) => format!("bold_line_style {}", line(l)),
                MeshSetting::AxisStyle(l) => format!("axis_style {}", line(l)),
                MeshSetting::DisableXMesh => "disable_x_mesh".into(),
                MeshSetting::DisableYMesh => "disable_y_mesh".into(),
                MeshSetting::DisableMesh => "disable_mesh".into(),
                MeshSetting::DisableXAxis => "disable_x_axis".into(),
                MeshSetting::DisableYAxis => "disable_y_axis".into(),
                MeshSetting::DisableAxes => "disable_axes".into(),
                MeshSetting::SetTickMarkSize(p, px) => {
                    format!("set_tick_mark_size {} {px}px", p.name())
                }
                MeshSetting::SetAllTickMarkSize(px) => format!("set_all_tick_mark_size {px}px"),
            })
            .collect();
        format!("MESH({}) of {}", settings.join(", "), self.chart.summary())
    }
}

impl SeriesLabels {
    /// `SERIES_LABELS(position upper_left) of CHART(...)`.
    pub fn summary(&self) -> String {
        let settings: Vec<String> = self
            .style
            .settings
            .iter()
            .map(|s| match s {
                SeriesLabelSetting::Position(p) => format!("position {}", position_name(*p)),
                SeriesLabelSetting::BorderStyle {
                    color,
                    stroke_width,
                } => format!("border_style {color} {stroke_width}px"),
                SeriesLabelSetting::BackgroundStyle(color) => {
                    format!("background_style {color}")
                }
                SeriesLabelSetting::Margin(px) => format!("margin {px}px"),
                SeriesLabelSetting::LegendAreaSize(px) => format!("legend_area_size {px}px"),
                SeriesLabelSetting::LabelFont(f) => format!("label_font {}", f.summary()),
            })
            .collect();
        format!(
            "SERIES_LABELS({}) of {}",
            settings.join(", "),
            self.chart.summary()
        )
    }
}

fn position_name(p: LabelPosition) -> String {
    match p {
        LabelPosition::Coordinate(x, y) => format!("({x}, {y})"),
        named => LabelPosition::NAMED
            .iter()
            .find(|(_, q)| *q == named)
            .map(|(n, _)| (*n).to_string())
            .expect("every named position is listed"),
    }
}

impl Font {
    /// `FONT(sans-serif, 12)`, `FONT(serif, 20, bold, #ff0000)`.
    pub fn summary(&self) -> String {
        let mut out = format!("FONT({}, {}", self.family, self.size);
        if self.style != FontStyle::Normal {
            out.push_str(&format!(", {}", self.style.name()));
        }
        if let Some(color) = self.color {
            out.push_str(&format!(", {color}"));
        }
        out.push(')');
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::accumulate::SeriesAggregate;
    use crate::spec::{Chart, Column, Font, Series};

    fn line(n: usize) -> Series {
        Series::new(
            SeriesAggregate::LineSeries,
            Column::Numeric(vec![0.0; n]),
            Column::Numeric(vec![0.0; n]),
        )
        .unwrap()
    }

    #[test]
    fn series() {
        assert_eq!(
            line(120).label("temp").summary(),
            "SERIES(line 'temp', 120 points)"
        );
        assert_eq!(line(1).summary(), "SERIES(line, 1 point)");
        let h = Series::new(
            SeriesAggregate::Histogram,
            Column::Category(vec!["a".into(), "b".into()]),
            Column::Numeric(vec![1.0, 2.0]),
        )
        .unwrap()
        .label("it's");
        assert_eq!(h.summary(), "SERIES(histogram 'it''s', 2 buckets)");
        let h = Series::new(
            SeriesAggregate::HistogramHorizontal,
            Column::Numeric(vec![1.0]),
            Column::Integer(vec![3]),
        )
        .unwrap();
        assert_eq!(h.summary(), "SERIES(horizontal histogram, 1 bucket)");
    }

    #[test]
    fn m5_series() {
        let n = |k: usize| Column::Numeric(vec![1.0; k]);
        let summary = |a, x, y| Series::new(a, x, y).unwrap().summary();
        assert_eq!(
            summary(SeriesAggregate::AreaSeries, n(2), n(2)),
            "SERIES(area, 2 points)"
        );
        assert_eq!(
            summary(SeriesAggregate::DashedLineSeries, n(1), n(1)),
            "SERIES(dashed line, 1 point)"
        );
        assert_eq!(
            summary(SeriesAggregate::ErrorBarVertical, n(3), n(3)),
            "SERIES(error bar, 3 bars)"
        );
        assert_eq!(
            summary(SeriesAggregate::ErrorBarHorizontal, n(1), n(1)),
            "SERIES(horizontal error bar, 1 bar)"
        );
        assert_eq!(
            summary(SeriesAggregate::CandleStick, n(2), n(2)),
            "SERIES(candlestick, 2 candles)"
        );
        let keys = Column::Category(vec!["a".into(), "b".into()]);
        assert_eq!(
            summary(SeriesAggregate::BoxplotVertical, keys.clone(), n(2)),
            "SERIES(boxplot, 2 boxes)"
        );
        assert_eq!(
            summary(SeriesAggregate::BoxplotHorizontal, n(2), keys),
            "SERIES(horizontal boxplot, 2 boxes)"
        );
        // Stepped buckets count their bins.
        let h = Series::new(
            SeriesAggregate::Histogram,
            Column::Numeric(vec![1.0, 2.0, 7.0]),
            n(3),
        )
        .unwrap();
        assert_eq!(h.summary(), "SERIES(histogram, 3 buckets)");
        assert_eq!(
            h.step(5.0).unwrap().summary(),
            "SERIES(histogram, 2 buckets)"
        );
    }

    #[test]
    fn chart() {
        assert_eq!(Chart::new().summary(), "CHART(empty)");
        let chart = Chart::new()
            .draw_series_list([line(120), line(120)])
            .unwrap();
        assert_eq!(chart.summary(), "CHART(line, 2 series, 240 points)");
        let point = Series::new(
            SeriesAggregate::PointSeries,
            Column::Numeric(vec![1.0]),
            Column::Numeric(vec![1.0]),
        )
        .unwrap();
        let chart = chart
            .draw_series(point)
            .unwrap()
            .caption("Oslo", None)
            .unwrap();
        assert_eq!(
            chart.summary(),
            "CHART(line+point, 3 series, 241 points, caption 'Oslo')"
        );
    }

    #[test]
    fn builders() {
        let mesh = Chart::new().configure_mesh().x_desc("day").y_desc("°C");
        assert_eq!(
            mesh.summary(),
            "MESH(x_desc 'day', y_desc '°C') of CHART(empty)"
        );
        let styled = Chart::new()
            .configure_mesh()
            .x_label_formatter("{:.1f} °C")
            .unwrap()
            .label_style(Font::new("serif", 10, None).unwrap())
            .bold_line_style("red", Some(2))
            .unwrap()
            .set_tick_mark_size("bottom", -3)
            .unwrap()
            .set(crate::spec::MeshSetting::DisableAxes);
        assert_eq!(
            styled.summary(),
            "MESH(x_label_formatter '{:.1f} °C', label_style FONT(serif, 10), bold_line_style \
             #ff0000 2px, set_tick_mark_size bottom -3px, disable_axes) of CHART(empty)"
        );
        let labels = Chart::new()
            .configure_series_labels()
            .position("upper_left")
            .unwrap()
            .position_at(3, 4)
            .unwrap();
        assert_eq!(
            labels.summary(),
            "SERIES_LABELS(position upper_left, position (3, 4)) of CHART(empty)"
        );
        let styled = Chart::new()
            .configure_series_labels()
            .border_style("black", None)
            .unwrap()
            .background_style("rgba(255, 255, 255, 0.8)")
            .unwrap();
        assert_eq!(
            styled.summary(),
            "SERIES_LABELS(border_style #000000 1px, background_style rgba(255, 255, 255, 0.8)) \
             of CHART(empty)"
        );
        let font = Font::new("serif", 20, Some("bold"))
            .unwrap()
            .color("red")
            .unwrap();
        assert_eq!(font.summary(), "FONT(serif, 20, bold, #ff0000)");
        assert_eq!(Font::default().summary(), "FONT(sans-serif, 12)");
    }
}
