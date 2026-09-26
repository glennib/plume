//! Drawing a [`Chart`] with plotters.
//!
//! plotters picks coordinate types at compile time, so [`draw`] matches on the chart's x-axis
//! kind and builds a `Cartesian2d` for that kind. The y axis is always `f64`.

use super::spec::{AxisKind, Chart, SeriesKind, XData, XRange};
use chrono::{DateTime, Utc};
use plotters::{
    chart::MeshStyle,
    coord::{
        Shift,
        ranged1d::SegmentedCoord,
        types::{RangedCoordf64, RangedSlice},
    },
    prelude::*,
};
use std::fmt::Display;

pub const DEFAULT_SIZE: (u32, u32) = (640, 480);
pub const MAX_DIMENSION: u32 = 8192;

const LINE_WIDTH: u32 = 2;
const POINT_SIZE: u32 = 3;
const BAR_GAP: u32 = 5;

type Result<T> = std::result::Result<T, String>;

fn err(e: impl Display) -> String {
    e.to_string()
}

/// The embedded font, registered as every generic family plotters asks for.
#[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"))))]
pub fn register_fonts() -> Result<()> {
    const FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
    for family in ["sans-serif", "serif", "monospace"] {
        plotters::style::register_font(family, FontStyle::Normal, FONT)
            .map_err(|_| "embedded font is invalid".to_string())?;
    }
    Ok(())
}

/// On wasm, plotters measures text through the browser and has no font registry.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
pub fn register_fonts() -> Result<()> {
    Ok(())
}

pub fn check_size((w, h): (u32, u32)) -> Result<()> {
    if (1..=MAX_DIMENSION).contains(&w) && (1..=MAX_DIMENSION).contains(&h) {
        Ok(())
    } else {
        Err(format!(
            "chart size must be between 1 and {MAX_DIMENSION} pixels per side, got {w}x{h}"
        ))
    }
}

pub fn to_svg(chart: &Chart, size: (u32, u32)) -> Result<String> {
    check_size(size)?;
    let mut out = String::new();
    draw(
        chart,
        SVGBackend::with_string(&mut out, size).into_drawing_area(),
    )?;
    Ok(out)
}

pub fn to_png(chart: &Chart, size: (u32, u32)) -> Result<Vec<u8>> {
    check_size(size)?;
    let (w, h) = size;
    let mut rgb = vec![0u8; w as usize * h as usize * 3];
    draw(
        chart,
        BitMapBackend::with_buffer(&mut rgb, size).into_drawing_area(),
    )?;

    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(err)?;
    writer.write_image_data(&rgb).map_err(err)?;
    writer.finish().map_err(err)?;
    Ok(out)
}

pub fn draw<DB: DrawingBackend>(chart: &Chart, root: DrawingArea<DB, Shift>) -> Result<()> {
    let kind = chart.axis_kind()?.ok_or("chart has no series")?;
    if let Some(range) = &chart.x_range
        && range.axis_kind() != kind
    {
        return Err(format!(
            "x_range is {} but the chart's x axis is {kind}",
            range.axis_kind()
        ));
    }
    let (y_lo, y_hi) = y_range(chart)?;

    root.fill(&WHITE).map_err(err)?;
    let mut builder = ChartBuilder::on(&root);
    builder
        .margin(10)
        .x_label_area_size(40)
        .y_label_area_size(60);
    if let Some(caption) = &chart.caption {
        builder.caption(&caption.text, ("sans-serif", caption.size));
    }

    match kind {
        AxisKind::F64 => {
            let (x_lo, x_hi) = x_range_f64(chart)?;
            let mut ctx = builder
                .build_cartesian_2d(x_lo..x_hi, y_lo..y_hi)
                .map_err(err)?;
            let mut mesh = ctx.configure_mesh();
            describe(&mut mesh, chart);
            mesh.draw().map_err(err)?;
            draw_xy(&mut ctx, chart, |x| match x {
                XData::F64(v) => v.iter().copied().map(Some).collect(),
                _ => unreachable!("axis kind checked"),
            })?;
            legend(&mut ctx, chart)?;
        }
        AxisKind::Time => {
            let (x_lo, x_hi) = x_range_time(chart)?;
            let format = time_format(x_hi - x_lo);
            let formatter = |t: &DateTime<Utc>| t.format(format).to_string();
            let mut ctx = builder
                .build_cartesian_2d(x_lo..x_hi, y_lo..y_hi)
                .map_err(err)?;
            let mut mesh = ctx.configure_mesh();
            describe(&mut mesh, chart);
            mesh.x_label_formatter(&formatter);
            mesh.draw().map_err(err)?;
            draw_xy(&mut ctx, chart, |x| match x {
                XData::Time(v) => v.iter().map(|t| micros_to_time(*t)).collect(),
                _ => unreachable!("axis kind checked"),
            })?;
            legend(&mut ctx, chart)?;
        }
        AxisKind::Category => {
            let categories = categories(chart);
            let formatter = |v: &SegmentValue<&String>| match v {
                SegmentValue::CenterOf(c) => (*c).clone(),
                _ => String::new(),
            };
            let mut ctx = builder
                .build_cartesian_2d(categories.as_slice().into_segmented(), y_lo..y_hi)
                .map_err(err)?;
            let mut mesh = ctx.configure_mesh();
            describe(&mut mesh, chart);
            mesh.disable_x_mesh()
                .x_labels(categories.len())
                .x_label_formatter(&formatter);
            mesh.draw().map_err(err)?;
            draw_bars(&mut ctx, chart, &categories)?;
            legend(&mut ctx, chart)?;
        }
    }
    root.present().map_err(err)
}

fn describe<DB, X, Y>(mesh: &mut MeshStyle<'_, '_, X, Y, DB>, chart: &Chart)
where
    DB: DrawingBackend,
    X: Ranged,
    Y: Ranged,
{
    if let Some(desc) = &chart.x_desc {
        mesh.x_desc(desc);
    }
    if let Some(desc) = &chart.y_desc {
        mesh.y_desc(desc);
    }
}

fn color(index: usize) -> RGBAColor {
    Palette99::pick(index).to_rgba()
}

/// Draws the line and point series of a chart whose x axis is `X`. Points whose x has no
/// axis value or whose y is not finite can't be placed and are left out.
fn draw_xy<DB, X>(
    ctx: &mut ChartContext<'_, DB, Cartesian2d<X, RangedCoordf64>>,
    chart: &Chart,
    xs: impl Fn(&XData) -> Vec<Option<X::ValueType>>,
) -> Result<()>
where
    DB: DrawingBackend,
    X: Ranged,
    X::ValueType: Clone + 'static,
{
    for (i, series) in chart.series.iter().enumerate() {
        let color = color(i);
        let points: Vec<(X::ValueType, f64)> = xs(&series.x)
            .into_iter()
            .zip(series.y.iter().copied())
            .filter_map(|(x, y)| y.is_finite().then_some((x?, y)))
            .collect();
        let anno = match series.kind {
            SeriesKind::Line => ctx
                .draw_series(LineSeries::new(points, color.stroke_width(LINE_WIDTH)))
                .map_err(err)?,
            SeriesKind::Point => ctx
                .draw_series(
                    points
                        .into_iter()
                        .map(|p| Circle::new(p, POINT_SIZE, color.filled())),
                )
                .map_err(err)?,
            SeriesKind::Bar => unreachable!("bar series always have a categorical x axis"),
        };
        if let Some(label) = &series.label {
            anno.label(label);
            match series.kind {
                SeriesKind::Line => anno.legend(move |(x, y)| {
                    PathElement::new(vec![(x, y), (x + 20, y)], color.stroke_width(LINE_WIDTH))
                }),
                _ => {
                    anno.legend(move |(x, y)| Circle::new((x + 10, y), POINT_SIZE, color.filled()))
                }
            };
        }
    }
    Ok(())
}

fn draw_bars<'c, DB: DrawingBackend>(
    ctx: &mut ChartContext<
        '_,
        DB,
        Cartesian2d<SegmentedCoord<RangedSlice<'c, String>>, RangedCoordf64>,
    >,
    chart: &Chart,
    categories: &'c [String],
) -> Result<()> {
    for (i, series) in chart.series.iter().enumerate() {
        let XData::Category(xs) = &series.x else {
            unreachable!("axis kind checked")
        };
        let color = color(i);
        let bars = xs
            .iter()
            .zip(series.y.iter().copied())
            .filter(|(_, y)| y.is_finite())
            .filter_map(|(x, y)| {
                let at = categories.iter().position(|c| c == x)?;
                let left = SegmentValue::Exact(&categories[at]);
                let right = categories
                    .get(at + 1)
                    .map_or(SegmentValue::Last, SegmentValue::Exact);
                let mut bar = Rectangle::new([(left, 0.0), (right, y)], color.filled());
                bar.set_margin(0, 0, BAR_GAP, BAR_GAP);
                Some(bar)
            });
        let anno = ctx.draw_series(bars).map_err(err)?;
        if let Some(label) = &series.label {
            anno.label(label).legend(move |(x, y)| {
                Rectangle::new([(x, y - 5), (x + 20, y + 5)], color.filled())
            });
        }
    }
    Ok(())
}

fn legend<'a, DB, X, Y>(
    ctx: &mut ChartContext<'a, DB, Cartesian2d<X, Y>>,
    chart: &Chart,
) -> Result<()>
where
    DB: DrawingBackend + 'a,
    X: Ranged,
    Y: Ranged,
{
    if chart.series.iter().all(|s| s.label.is_none()) {
        return Ok(());
    }
    ctx.configure_series_labels()
        .background_style(WHITE.mix(0.8))
        .border_style(BLACK)
        .draw()
        .map_err(err)
}

/// Distinct categories across all series, in order of first appearance.
fn categories(chart: &Chart) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for series in &chart.series {
        if let XData::Category(xs) = &series.x {
            for x in xs {
                if !out.contains(x) {
                    out.push(x.clone());
                }
            }
        }
    }
    out
}

/// Min and max of the finite values, or `None` when there are none.
fn extent(values: impl IntoIterator<Item = f64>) -> Option<(f64, f64)> {
    values
        .into_iter()
        .filter(|v| v.is_finite())
        .fold(None, |acc, v| match acc {
            None => Some((v, v)),
            Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
        })
}

/// Widens a range that is empty or a single point so the axis has some extent.
fn widen((lo, hi): (f64, f64)) -> (f64, f64) {
    if lo < hi {
        (lo, hi)
    } else {
        let d = if lo == 0.0 { 1.0 } else { lo.abs() * 0.5 };
        (lo - d, hi + d)
    }
}

fn pad((lo, hi): (f64, f64), fraction: f64) -> (f64, f64) {
    let d = (hi - lo) * fraction;
    (lo - d, hi + d)
}

/// The y range: explicit, or the data extent with 5% padding. Bar charts keep 0 as baseline.
fn y_range(chart: &Chart) -> Result<(f64, f64)> {
    if let Some(range) = chart.y_range {
        return Ok(range);
    }
    let has_bars = chart.series.iter().any(|s| s.kind == SeriesKind::Bar);
    let mut ext =
        extent(chart.series.iter().flat_map(|s| s.y.iter().copied())).unwrap_or((0.0, 1.0));
    if has_bars {
        ext = (ext.0.min(0.0), ext.1.max(0.0));
    }
    let (mut lo, mut hi) = pad(widen(ext), 0.05);
    if has_bars && ext.0 == 0.0 {
        lo = 0.0;
    }
    if has_bars && ext.1 == 0.0 {
        hi = 0.0;
    }
    Ok((lo, hi))
}

/// The numeric x range: explicit, or the data extent. Point series get 2% padding so markers
/// at the edges aren't clipped.
fn x_range_f64(chart: &Chart) -> Result<(f64, f64)> {
    if let Some(XRange::F64(lo, hi)) = chart.x_range {
        return Ok((lo, hi));
    }
    let values = chart.series.iter().flat_map(|s| match &s.x {
        XData::F64(v) => v.clone(),
        _ => Vec::new(),
    });
    let ext = widen(extent(values).unwrap_or((0.0, 1.0)));
    let has_points = chart.series.iter().any(|s| s.kind == SeriesKind::Point);
    Ok(if has_points { pad(ext, 0.02) } else { ext })
}

fn micros_to_time(micros: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_micros(micros)
}

fn x_range_time(chart: &Chart) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let (lo, hi) = match chart.x_range {
        Some(XRange::Time(lo, hi)) => (lo, hi),
        _ => {
            let values = chart.series.iter().flat_map(|s| match &s.x {
                XData::Time(v) => v.clone(),
                _ => Vec::new(),
            });
            let (lo, hi) = values
                .fold(None, |acc: Option<(i64, i64)>, v| match acc {
                    None => Some((v, v)),
                    Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
                })
                .unwrap_or((0, 0));
            if lo == hi {
                (lo - 1_000_000, hi + 1_000_000)
            } else {
                (lo, hi)
            }
        }
    };
    let out_of_range = || "timestamp outside the supported range".to_string();
    Ok((
        micros_to_time(lo).ok_or_else(out_of_range)?,
        micros_to_time(hi).ok_or_else(out_of_range)?,
    ))
}

/// A tick label format that fits the time span shown.
fn time_format(span: chrono::TimeDelta) -> &'static str {
    if span >= chrono::TimeDelta::days(2) {
        "%Y-%m-%d"
    } else if span >= chrono::TimeDelta::minutes(2) {
        "%Y-%m-%d %H:%M"
    } else {
        "%H:%M:%S%.3f"
    }
}

#[cfg(test)]
mod tests {
    use super::super::spec::{Caption, Series};
    use super::*;

    fn line(label: Option<&str>) -> Series {
        Series {
            kind: SeriesKind::Line,
            label: label.map(Into::into),
            x: XData::F64(vec![0.0, 1.0, 2.0]),
            y: vec![1.0, 3.0, 2.0],
        }
    }

    fn setup() {
        register_fonts().unwrap();
    }

    #[test]
    fn svg_has_caption_and_legend() {
        setup();
        let chart = Chart {
            series: vec![line(Some("alpha")), line(Some("beta"))],
            caption: Some(Caption {
                text: "Hello".into(),
                size: 30,
            }),
            ..Default::default()
        };
        let svg = to_svg(&chart, DEFAULT_SIZE).unwrap();
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("Hello"));
        assert!(svg.contains("alpha") && svg.contains("beta"));
    }

    #[test]
    fn png_bars() {
        setup();
        let chart = Chart {
            series: vec![Series {
                kind: SeriesKind::Bar,
                label: None,
                x: XData::Category(vec!["a".into(), "b".into()]),
                y: vec![1.0, 2.0],
            }],
            ..Default::default()
        };
        let png = to_png(&chart, (200, 100)).unwrap();
        assert_eq!(&png[..4], b"\x89PNG");
    }

    #[test]
    fn time_axis() {
        setup();
        let chart = Chart {
            series: vec![Series {
                kind: SeriesKind::Point,
                label: None,
                x: XData::Time(vec![0, 86_400_000_000 * 3]),
                y: vec![1.0, 2.0],
            }],
            ..Default::default()
        };
        let svg = to_svg(&chart, DEFAULT_SIZE).unwrap();
        assert!(svg.contains("1970-01-02"));
    }

    #[test]
    fn y_range_bars_keep_baseline() {
        let chart = Chart {
            series: vec![Series {
                kind: SeriesKind::Bar,
                label: None,
                x: XData::Category(vec!["a".into()]),
                y: vec![10.0],
            }],
            ..Default::default()
        };
        let (lo, hi) = y_range(&chart).unwrap();
        assert_eq!(lo, 0.0);
        assert!(hi > 10.0);
    }

    #[test]
    fn rejects_bad_size() {
        assert!(check_size((0, 10)).is_err());
        assert!(check_size((10, MAX_DIMENSION + 1)).is_err());
    }
}
