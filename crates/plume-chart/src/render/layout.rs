//! Drawing the roots of a `CHART`: a cartesian chart, a grid of roots (`split_evenly`), a
//! titled root (`titled`) and a pie.

use super::draw::{self, text_style};
use crate::color::Color;
use crate::error::{Error, Result};
use crate::spec::{Pie, PieSetting, Root};
use plotters::coord::Shift;
use plotters::prelude::*;
use std::f64::consts::PI;

/// px kept free between a pie's labels and the edge of its area when the radius is fitted.
const PIE_MARGIN: i32 = 10;

fn plotters_error(e: impl std::fmt::Display) -> Error {
    Error::render(e)
}

/// Draws a root on an area: its fill, then what it holds.
pub(crate) fn draw_root<DB: DrawingBackend>(
    area: &DrawingArea<DB, Shift>,
    root: &Root,
) -> Result<()> {
    match root {
        Root::Cartesian(chart) => draw::draw(area, chart),
        Root::Grid(grid) => {
            area.fill(&grid.fill.to_plotters())
                .map_err(plotters_error)?;
            let cells = area.split_evenly((grid.rows as usize, grid.cols as usize));
            for (cell, root) in cells.iter().zip(&grid.cells) {
                if let Some(root) = root {
                    draw_root(cell, root)?;
                }
            }
            Ok(())
        }
        Root::Titled(titled) => {
            area.fill(&titled.fill.to_plotters())
                .map_err(plotters_error)?;
            let below = area
                .titled(&titled.text, text_style(&titled.font))
                .map_err(plotters_error)?;
            draw_root(&below, &titled.inner)
        }
        Root::Pie(pie) => draw_pie(area, pie),
    }
}

/// The `Pie` settings after the chain: the last call of each setter wins.
struct PieOptions<'a> {
    start_angle: f64,
    label_style: Option<&'a crate::spec::Font>,
    percentages: Option<&'a crate::spec::Font>,
    label_offset: Option<f64>,
    radius: Option<f64>,
}

impl<'a> PieOptions<'a> {
    fn of(pie: &'a Pie) -> PieOptions<'a> {
        let mut options = PieOptions {
            start_angle: 0.0,
            label_style: None,
            percentages: None,
            label_offset: None,
            radius: None,
        };
        for setting in &pie.settings {
            match setting {
                PieSetting::StartAngle(a) => options.start_angle = *a,
                PieSetting::LabelStyle(f) => options.label_style = Some(f),
                PieSetting::Percentages(f) => options.percentages = Some(f),
                PieSetting::LabelOffset(px) => options.label_offset = Some(f64::from(*px)),
                PieSetting::Radius(px) => options.radius = Some(f64::from(*px)),
            }
        }
        options
    }

    /// The label style `Pie` uses at `radius`: the one set, or plotters' default,
    /// sans-serif at 5% of the radius.
    fn label_style(&self, radius: f64) -> TextStyle<'a> {
        match self.label_style {
            Some(font) => text_style(font),
            None => TextStyle::from(("sans-serif", radius * 0.05).into_font()).color(&BLACK),
        }
    }

    /// The label offset `Pie` uses at `radius`: the one set, or plotters' default of 5% of
    /// the radius.
    fn label_offset(&self, radius: f64) -> f64 {
        self.label_offset.unwrap_or(radius * 0.05)
    }
}

/// Draws a `Pie` in the middle of the area: `Pie::new(center, radius, sizes, colors, labels)`
/// with the colours from `Palette99` in slice order, then the setter calls.
fn draw_pie<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, pie: &Pie) -> Result<()> {
    area.fill(&pie.fill.to_plotters()).map_err(plotters_error)?;
    if pie.sizes.is_empty() {
        return Ok(());
    }
    let options = PieOptions::of(pie);
    let (w, h) = area.dim_in_pixel();
    // `Pie` draws on the backend directly, at backend coordinates, so the centre is the
    // area's centre in those.
    let (x0, y0) = area.get_base_pixel();
    let center = (x0 + w as i32 / 2, y0 + h as i32 / 2);
    let radius = match options.radius {
        Some(r) => r,
        None => fit_radius(area, pie, &options, (w, h))?,
    };
    let colors: Vec<RGBColor> = (0..pie.sizes.len())
        .map(|i| {
            let c = Color::palette99(i);
            RGBColor(c.r, c.g, c.b)
        })
        .collect();
    let mut element =
        plotters::element::Pie::new(&center, &radius, &pie.sizes, &colors, &pie.labels);
    element.start_angle(options.start_angle);
    element.label_style(options.label_style(radius));
    element.label_offset(options.label_offset(radius));
    if let Some(font) = options.percentages {
        element.percentages(text_style(font));
    }
    area.draw(&element).map_err(plotters_error)
}

/// The largest radius (in whole px) at which every label `Pie` draws stays inside the area,
/// with a margin. `Pie` places a label's top-left corner (its top-right, left of the centre)
/// at the label offset beyond the rim, at the middle angle of the slice.
fn fit_radius<DB: DrawingBackend>(
    area: &DrawingArea<DB, Shift>,
    pie: &Pie,
    options: &PieOptions<'_>,
    (w, h): (u32, u32),
) -> Result<f64> {
    let (w, h) = (w as i32, h as i32);
    let largest = (w.min(h) / 2 - PIE_MARGIN).max(1);
    let fits = |radius: i32| -> Result<bool> {
        let radius = f64::from(radius);
        let style = options.label_style(radius);
        let offset = options.label_offset(radius);
        let total: f64 = pie.sizes.iter().sum();
        let (cx, cy) = (w / 2, h / 2);
        let mut theta = options.start_angle.to_radians();
        for (size, label) in pie.sizes.iter().zip(&pie.labels) {
            let ratio = size / total;
            let middle = theta + ratio * PI;
            theta += ratio * 2.0 * PI;
            let (tw, th) = area
                .estimate_text_size(label, &style)
                .map_err(plotters_error)?;
            let (sin, cos) = middle.sin_cos();
            let mut x = ((radius + offset) * cos + f64::from(cx)).round() as i32;
            let y = ((radius + offset) * sin + f64::from(cy)).round() as i32;
            if x <= cx {
                x -= tw as i32;
            }
            let inside = x >= PIE_MARGIN
                && y >= PIE_MARGIN
                && x + tw as i32 <= w - PIE_MARGIN
                && y + th as i32 <= h - PIE_MARGIN;
            if !inside {
                return Ok(false);
            }
        }
        Ok(true)
    };
    // The labels move outwards as the radius grows, so the radii that fit are a prefix.
    let (mut lo, mut hi) = (1, largest);
    if fits(hi)? {
        return Ok(f64::from(hi));
    }
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if fits(mid)? {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(f64::from(lo))
}
