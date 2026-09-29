//! Rendering a `CHART` ([`Root`]) with plotters: `to_svg` is `SVGBackend`, `to_png` and `to_rgb` are
//! `BitMapBackend`.
//!
//! Text is laid out and rasterized with plotters' `ab_glyph` engine and an embedded DejaVu Sans,
//! registered under every family name a chart uses, so no system fonts are involved and the
//! output depends only on the chart value and the size.

mod axis;
mod draw;
mod layout;

use crate::error::{Error, ErrorKind, Result};
use crate::spec::Root;
use plotters::prelude::*;
use std::collections::BTreeSet;
use std::sync::Mutex;

/// The default image width in px.
pub const DEFAULT_WIDTH: u32 = 640;
/// The default image height in px.
pub const DEFAULT_HEIGHT: u32 = 480;
/// The largest width or height in px.
pub const MAX_SIDE: u32 = 8192;

static FONT: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans.ttf");

/// The plotters generic families, registered before every render.
const GENERIC_FAMILIES: [&str; 3] = ["sans-serif", "serif", "monospace"];

/// Resolves optional SQL `width`/`height` arguments to a checked image size.
pub fn image_size(width: Option<i64>, height: Option<i64>) -> Result<(u32, u32)> {
    let side = |v: Option<i64>, default: u32| -> Option<u32> {
        match v {
            None => Some(default),
            Some(v) => u32::try_from(v).ok().filter(|v| (1..=MAX_SIDE).contains(v)),
        }
    };
    match (side(width, DEFAULT_WIDTH), side(height, DEFAULT_HEIGHT)) {
        (Some(w), Some(h)) => Ok((w, h)),
        _ => {
            let show = |v: Option<i64>, default: u32| v.unwrap_or(i64::from(default));
            Err(size_error(
                show(width, DEFAULT_WIDTH),
                show(height, DEFAULT_HEIGHT),
            ))
        }
    }
}

fn size_error(w: impl std::fmt::Display, h: impl std::fmt::Display) -> Error {
    Error::invalid(format!(
        "chart size must be between 1 and {MAX_SIDE} px per side, got {w}x{h}"
    ))
}

fn check_size(width: u32, height: u32) -> Result<()> {
    if (1..=MAX_SIDE).contains(&width) && (1..=MAX_SIDE).contains(&height) {
        Ok(())
    } else {
        Err(size_error(width, height))
    }
}

/// Registers the embedded font under the generic families and every family the chart names.
/// `ab_glyph` looks fonts up by exact family name and fails for unregistered ones. The one
/// embedded face (DejaVu Sans) serves every family and style: plotters falls back to a
/// family's normal face for bold, italic and oblique.
fn register_fonts(chart: &Root) -> Result<()> {
    static REGISTERED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    let mut registered = REGISTERED.lock().unwrap_or_else(|e| e.into_inner());
    let used = chart
        .fonts()
        .into_iter()
        .map(|f| FontFamily::from(f.family.as_str()).as_str().to_string());
    for family in GENERIC_FAMILIES.map(String::from).into_iter().chain(used) {
        if registered.contains(&family) {
            continue;
        }
        plotters::style::register_font(&family, FontStyle::Normal, FONT)
            .map_err(|_| Error::render("the embedded font is invalid"))?;
        registered.insert(family);
    }
    Ok(())
}

/// `to_svg(chart, width, height)`: the chart as an SVG document.
pub fn to_svg(chart: &Root, width: u32, height: u32) -> Result<String> {
    check_size(width, height)?;
    register_fonts(chart)?;
    let mut out = String::new();
    {
        let root = SVGBackend::with_string(&mut out, (width, height)).into_drawing_area();
        layout::draw_root(&root, chart)?;
        root.present().map_err(Error::render)?;
    }
    Ok(out)
}

/// Renders the chart into an RGB buffer of `width * height * 3` bytes, row by row from the top
/// left, the layout of plotters' `BitMapBackend::with_buffer`. The buffer's previous content is
/// the background under the chart's fill.
pub fn render_rgb(chart: &Root, buffer: &mut [u8], width: u32, height: u32) -> Result<()> {
    check_size(width, height)?;
    let needed = width as usize * height as usize * 3;
    if buffer.len() != needed {
        return Err(Error::invalid(format!(
            "an RGB buffer for {width}x{height} px needs {needed} bytes, got {}",
            buffer.len()
        )));
    }
    register_fonts(chart)?;
    let root = BitMapBackend::with_buffer(buffer, (width, height)).into_drawing_area();
    layout::draw_root(&root, chart)?;
    root.present().map_err(Error::render)?;
    Ok(())
}

/// `to_rgb(chart, width, height)`: the chart as RGB bytes, `width * height * 3` of them.
/// The buffer starts black, as plotters' bitmaps do, so only a transparent fill shows it.
pub fn to_rgb(chart: &Root, width: u32, height: u32) -> Result<Vec<u8>> {
    check_size(width, height)?;
    let mut buffer = vec![0; width as usize * height as usize * 3];
    render_rgb(chart, &mut buffer, width, height)?;
    Ok(buffer)
}

/// `to_png(chart, width, height)`: the chart as a PNG file (8-bit RGB).
pub fn to_png(chart: &Root, width: u32, height: u32) -> Result<Vec<u8>> {
    let rgb = to_rgb(chart, width, height)?;
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(png_error)?;
    writer.write_image_data(&rgb).map_err(png_error)?;
    writer.finish().map_err(png_error)?;
    Ok(out)
}

fn png_error(e: png::EncodingError) -> Error {
    Error::new(ErrorKind::Render, format!("PNG encoding: {e}"))
}
