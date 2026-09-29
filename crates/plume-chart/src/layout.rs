//! The roots of a `CHART` beyond the cartesian chart: grids (`split_evenly`), titled areas
//! (`titled`) and pies (`pie`), the methods every root has (`root_fill`), and the state of the
//! `pie` aggregate.

use crate::accumulate::SortKey;
use crate::color::Color;
use crate::error::{Error, Result};
use crate::methods::{MAX_PX, px};
use crate::spec::{Chart, Font, Grid, Pie, PieSetting, Root, Titled};

/// The most rows or columns `split_evenly` accepts.
pub const MAX_GRID_SIDE: u32 = 64;

/// The title size when `titled(chart, text)` is called without one. plotters' `titled` takes
/// a style and has no default.
pub const DEFAULT_TITLE_SIZE: u32 = 20;

impl Root {
    /// What the root is, for messages: `a grid 2x2 (split_evenly)`.
    pub fn describe(&self) -> String {
        match self {
            Root::Cartesian(_) => "a cartesian chart (chart())".into(),
            Root::Grid(g) => format!("a grid {}x{} (split_evenly)", g.rows, g.cols),
            Root::Titled(_) => "a titled chart (titled)".into(),
            Root::Pie(_) => "a pie (the pie aggregate)".into(),
        }
    }

    /// The cartesian chart, for a method of `ChartBuilder` or `ChartContext`; an error naming
    /// `method` and the root kind otherwise.
    pub fn cartesian(self, method: &str) -> Result<Chart> {
        match self {
            Root::Cartesian(chart) => Ok(chart),
            other if method == "caption" => Err(Error::invalid(format!(
                "caption applies to a cartesian chart (ChartBuilder::caption), and this CHART \
                 is {}, which is not drawn through a ChartBuilder; use titled(chart, text) for \
                 a title above it",
                other.describe()
            ))),
            other => Err(Error::invalid(format!(
                "{method} applies to a cartesian chart (ChartBuilder and ChartContext), and this \
                 CHART is {}",
                other.describe()
            ))),
        }
    }

    /// The pie, for a method of plotters' `Pie`; an error naming `method` and the root kind
    /// otherwise.
    pub fn into_pie(self, method: &str) -> Result<Pie> {
        match self {
            Root::Pie(pie) => Ok(pie),
            other => Err(Error::invalid(format!(
                "{method} applies to a pie (plotters' Pie, made by the pie aggregate), and this \
                 CHART is {}",
                other.describe()
            ))),
        }
    }

    /// The colour `root_fill` set on this root.
    pub fn fill_color(&self) -> Color {
        match self {
            Root::Cartesian(c) => c.fill,
            Root::Grid(g) => g.fill,
            Root::Titled(t) => t.fill,
            Root::Pie(p) => p.fill,
        }
    }

    /// `root_fill(color)`: `root.fill(&color)`, the background of the whole area.
    pub fn fill(mut self, color: &str) -> Result<Root> {
        let color = Color::parse(color)?;
        match &mut self {
            Root::Cartesian(c) => c.fill = color,
            Root::Grid(g) => g.fill = color,
            Root::Titled(t) => t.fill = color,
            Root::Pie(p) => p.fill = color,
        }
        Ok(self)
    }

    /// `titled(chart, text [, size])`: `root.titled(text, ("sans-serif", size))`, the title
    /// across the top and this root below it. The size defaults to 20.
    pub fn titled(self, text: impl Into<String>, size: Option<i64>) -> Result<Root> {
        let font = match size {
            Some(size) => Font::sans_serif(size)?,
            None => Font {
                size: DEFAULT_TITLE_SIZE,
                ..Font::default()
            },
        };
        Ok(self.titled_font(text, font))
    }

    /// `titled(chart, text, font)`: `root.titled(text, font)`. The area under the title is
    /// filled with this root's fill until `root_fill` changes it.
    pub fn titled_font(self, text: impl Into<String>, font: Font) -> Root {
        Root::Titled(Titled {
            text: text.into(),
            font,
            fill: self.fill_color(),
            inner: Box::new(self),
        })
    }

    /// `split_evenly(charts, rows, cols)`: `root.split_evenly((rows, cols))` with the charts
    /// drawn in the cells in row-major order. `None` and cells past the last chart are blank.
    pub fn split_evenly(cells: Vec<Option<Root>>, rows: i64, cols: i64) -> Result<Root> {
        let side = |what: &str, v: i64| {
            u32::try_from(v)
                .ok()
                .filter(|v| (1..=MAX_GRID_SIDE).contains(v))
                .ok_or_else(|| {
                    Error::invalid(format!(
                        "split_evenly: {what} must be between 1 and {MAX_GRID_SIDE}, got {v}"
                    ))
                })
        };
        let (rows, cols) = (side("rows", rows)?, side("cols", cols)?);
        let n = (rows * cols) as usize;
        if cells.len() > n {
            return Err(Error::invalid(format!(
                "split_evenly: {} charts do not fit a {rows}x{cols} grid of {n} cells",
                cells.len()
            )));
        }
        Ok(Root::Grid(Grid {
            rows,
            cols,
            cells,
            fill: Color::WHITE,
        }))
    }

    /// Every `FONT` the root uses, for registering the embedded font under their families.
    pub fn fonts(&self) -> Vec<&Font> {
        match self {
            Root::Cartesian(chart) => chart.fonts(),
            Root::Grid(g) => g.cells.iter().flatten().flat_map(Root::fonts).collect(),
            Root::Titled(t) => std::iter::once(&t.font).chain(t.inner.fonts()).collect(),
            Root::Pie(p) => p.settings.iter().filter_map(PieSetting::font).collect(),
        }
    }
}

impl Chart {
    /// Every `FONT` the chart uses: the caption's, and those of its mesh and legend settings.
    pub fn fonts(&self) -> Vec<&Font> {
        use crate::spec::DrawOp;
        let mut fonts: Vec<&Font> = self.caption.iter().map(|c| &c.font).collect();
        for op in &self.ops {
            match op {
                DrawOp::Mesh(style) | DrawOp::SecondaryAxes(style) => {
                    fonts.extend(style.settings.iter().filter_map(|s| s.font()))
                }
                DrawOp::SeriesLabels(style) => {
                    fonts.extend(style.settings.iter().filter_map(|s| s.font()))
                }
                DrawOp::Series(_) | DrawOp::SecondarySeries(_) => {}
            }
        }
        fonts
    }
}

impl Pie {
    fn set(mut self, setting: PieSetting) -> Pie {
        self.settings.push(setting);
        self
    }

    /// `Pie::start_angle(deg)`: where the first slice starts, in degrees clockwise from the
    /// positive x axis (plotters' default 0 is three o'clock; -90 is twelve o'clock).
    pub fn start_angle(self, degrees: f64) -> Result<Pie> {
        if !degrees.is_finite() {
            return Err(Error::invalid(format!(
                "start_angle must be a finite number of degrees, got {degrees}"
            )));
        }
        Ok(self.set(PieSetting::StartAngle(degrees)))
    }

    /// `Pie::label_style`: the style of the slice labels.
    pub fn label_style(self, font: Font) -> Pie {
        self.set(PieSetting::LabelStyle(font))
    }

    /// `Pie::percentages`: draws each slice's percentage inside it, in this style.
    pub fn percentages(self, font: Font) -> Pie {
        self.set(PieSetting::Percentages(font))
    }

    /// `Pie::label_offset`: px between the rim and the labels; may be negative.
    pub fn label_offset(self, offset: i64) -> Result<Pie> {
        let offset = i32::try_from(offset)
            .ok()
            .filter(|v| v.unsigned_abs() <= MAX_PX)
            .ok_or_else(|| {
                Error::invalid(format!(
                    "label_offset must be between -{MAX_PX} and {MAX_PX} px, got {offset}"
                ))
            })?;
        Ok(self.set(PieSetting::LabelOffset(offset)))
    }

    /// The `radius` argument of `Pie::new`, px.
    pub fn radius(self, radius: i64) -> Result<Pie> {
        match px("radius", radius)? {
            0 => Err(Error::invalid("radius must be at least 1 px")),
            r => Ok(self.set(PieSetting::Radius(r))),
        }
    }
}

/// One row of the `pie` aggregate.
#[derive(Clone, Debug)]
struct Slice {
    order: Option<SortKey>,
    label_key: SortKey,
    label: String,
    size: f64,
}

/// The state of the `pie(size, label)` aggregate for one SQL group.
///
/// Rows whose size is missing, NaN, infinite, zero or negative are skipped. The slices are
/// sorted by `order_by` (when given), then by label, then by size, so the result does not
/// depend on the order rows arrive in.
#[derive(Clone, Debug, Default)]
pub struct PieAccumulator {
    slices: Vec<Slice>,
}

impl PieAccumulator {
    pub fn new() -> PieAccumulator {
        PieAccumulator::default()
    }

    /// Adds one row: its size, the label as it sorts and as text (`label::VARCHAR`, `None`
    /// for a `NULL` label, which draws as an empty label), and the `order_by` value when the
    /// aggregate was called with one.
    pub fn push(
        &mut self,
        size: Option<f64>,
        label_key: SortKey,
        label: Option<String>,
        order_by: Option<SortKey>,
    ) {
        let Some(size) = size.filter(|s| s.is_finite() && *s > 0.0) else {
            return;
        };
        self.slices.push(Slice {
            order: order_by,
            label_key,
            label: label.unwrap_or_default(),
            size,
        });
    }

    /// Merges another partial state into this one.
    pub fn combine(&mut self, other: PieAccumulator) {
        self.slices.extend(other.slices);
    }

    /// The number of usable rows pushed so far.
    pub fn len(&self) -> usize {
        self.slices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slices.is_empty()
    }

    /// The `CHART` whose root is the pie. Zero usable rows give a pie without slices, which
    /// draws only the fill.
    pub fn finish(mut self) -> Root {
        self.slices.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| a.label_key.cmp(&b.label_key))
                .then_with(|| a.size.total_cmp(&b.size))
        });
        let (sizes, labels) = self.slices.into_iter().map(|s| (s.size, s.label)).unzip();
        Root::Pie(Pie {
            sizes,
            labels,
            fill: Color::WHITE,
            settings: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> SortKey {
        SortKey::Text(s.into())
    }

    fn pie(root: Root) -> Pie {
        root.into_pie("test").unwrap()
    }

    #[test]
    fn pie_orders_by_label_then_size_and_skips_bad_sizes() {
        let mut a = PieAccumulator::new();
        let mut b = PieAccumulator::new();
        a.push(Some(3.0), text("b"), Some("b".into()), None);
        b.push(Some(1.0), text("a"), Some("a".into()), None);
        a.push(Some(0.5), text("b"), Some("b".into()), None);
        b.push(Some(f64::NAN), text("c"), Some("c".into()), None);
        a.push(Some(0.0), text("d"), Some("d".into()), None);
        b.push(Some(-2.0), text("e"), Some("e".into()), None);
        a.push(None, text("f"), Some("f".into()), None);
        b.push(Some(2.0), SortKey::Null, None, None);
        a.combine(b);
        let p = pie(a.finish());
        assert_eq!(p.sizes, vec![1.0, 0.5, 3.0, 2.0]);
        assert_eq!(p.labels, vec!["a", "b", "b", ""]);
    }

    #[test]
    fn pie_order_by_comes_first() {
        let mut a = PieAccumulator::new();
        for (size, label) in [(1.0, "x"), (5.0, "y"), (3.0, "z")] {
            a.push(
                Some(size),
                text(label),
                Some(label.into()),
                Some(SortKey::Float(-size)),
            );
        }
        assert_eq!(pie(a.finish()).labels, vec!["y", "z", "x"]);
    }

    #[test]
    fn grids_check_their_sides_and_counts() {
        let chart = || Some(Root::from(Chart::new()));
        assert!(Root::split_evenly(vec![chart(), None, chart()], 2, 2).is_ok());
        let err = Root::split_evenly(vec![chart(); 5], 2, 2).unwrap_err();
        assert_eq!(
            err.message(),
            "split_evenly: 5 charts do not fit a 2x2 grid of 4 cells"
        );
        let err = Root::split_evenly(vec![], 0, 2).unwrap_err();
        assert_eq!(
            err.message(),
            "split_evenly: rows must be between 1 and 64, got 0"
        );
        assert!(Root::split_evenly(vec![], 1, 65).is_err());
    }

    #[test]
    fn methods_name_the_root_kind() {
        let grid = Root::split_evenly(vec![], 2, 3).unwrap();
        let err = grid.clone().cartesian("x_range").unwrap_err();
        assert_eq!(
            err.message(),
            "x_range applies to a cartesian chart (ChartBuilder and ChartContext), and this \
             CHART is a grid 2x3 (split_evenly)"
        );
        let err = PieAccumulator::new()
            .finish()
            .cartesian("caption")
            .unwrap_err();
        assert!(err.message().contains("use titled(chart, text)"), "{err}");
        let err = grid.into_pie("start_angle").unwrap_err();
        assert!(err.message().starts_with("start_angle applies to a pie"));
    }

    #[test]
    fn titled_keeps_the_inner_fill() {
        let inner = Root::from(Chart::new()).fill("grey").unwrap();
        let Root::Titled(t) = inner.titled("t", None).unwrap() else {
            panic!("not titled");
        };
        assert_eq!(t.fill, Color::parse("grey").unwrap());
        assert_eq!(t.font.size, DEFAULT_TITLE_SIZE);
    }
}
