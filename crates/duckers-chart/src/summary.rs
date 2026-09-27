//! One-line summaries, the `VARCHAR` casts of the SQL values.

use crate::spec::{
    Chart, Font, FontStyle, LabelPosition, Mesh, MeshSetting, Series, SeriesKind,
    SeriesLabelSetting, SeriesLabels,
};

/// A SQL-style string literal: single quotes, embedded quotes doubled.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
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
        let count = match self.kind {
            SeriesKind::Histogram(_) | SeriesKind::HistogramHorizontal(_) => {
                plural(self.len(), "bucket", "buckets")
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
