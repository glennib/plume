//! Colour strings, parsed into the RGBA values plotters draws with.
//!
//! Accepted forms, all case-insensitive:
//!
//! - plotters' named colours (`white`, `black`, `red`, `green`, `blue`, `yellow`, `cyan`,
//!   `magenta`, `transparent`), which take precedence over `full_palette` names;
//! - `full_palette` names (`blue_400`, `deeporange`, `grey_a100`);
//! - `#rrggbb` and `#rrggbbaa`;
//! - `rgb(r, g, b)` with components 0..255 and `rgba(r, g, b, a)` with alpha 0..1;
//! - `hsl(h, s, l)` with every component in 0..1, as plotters' `HSLColor`;
//! - `palette99:n` for `Palette99::pick(n)`.

use crate::error::{Error, Result};
use crate::full_palette::FULL_PALETTE;
use plotters::style::{Color as _, HSLColor, Palette, Palette99, RGBAColor, RGBColor};
use serde::{Deserialize, Serialize};
use std::fmt;

/// An RGBA colour with alpha in 0..1, as plotters' `RGBAColor`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f64,
}

const PRELUDE: &[(&str, RGBColor)] = &[
    ("white", plotters::style::WHITE),
    ("black", plotters::style::BLACK),
    ("red", plotters::style::RED),
    ("green", plotters::style::GREEN),
    ("blue", plotters::style::BLUE),
    ("yellow", plotters::style::YELLOW),
    ("cyan", plotters::style::CYAN),
    ("magenta", plotters::style::MAGENTA),
];

impl Color {
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0.0,
    };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 1.0 }
    }

    /// `Palette99::pick(index)`, the default colour of the series at `index`.
    pub fn palette99(index: usize) -> Color {
        let (r, g, b) = Palette99::pick(index).rgb();
        Color::rgb(r, g, b)
    }

    /// Parses a colour string, see the module documentation for the forms.
    pub fn parse(text: &str) -> Result<Color> {
        let s = text.trim().to_ascii_lowercase();
        let unknown = || {
            Error::invalid(format!(
                "unknown colour '{text}': expected a plotters colour name ('red', 'blue_400'), \
                 '#rrggbb', '#rrggbbaa', 'rgb(r, g, b)', 'rgba(r, g, b, a)', 'hsl(h, s, l)' \
                 or 'palette99:n'"
            ))
        };
        if let Some(hex) = s.strip_prefix('#') {
            return parse_hex(hex).ok_or_else(unknown);
        }
        if let Some(args) = function_args(&s, "rgba") {
            let [r, g, b, a] = numbers::<4>(text, args)?;
            let a = unit(text, "alpha", a)?;
            return Ok(Color {
                a,
                ..Color::rgb(byte(text, r)?, byte(text, g)?, byte(text, b)?)
            });
        }
        if let Some(args) = function_args(&s, "rgb") {
            let [r, g, b] = numbers::<3>(text, args)?;
            return Ok(Color::rgb(byte(text, r)?, byte(text, g)?, byte(text, b)?));
        }
        if let Some(args) = function_args(&s, "hsl") {
            let [h, sat, l] = numbers::<3>(text, args)?;
            let hsl = HSLColor(
                unit(text, "hue", h)?,
                unit(text, "saturation", sat)?,
                unit(text, "lightness", l)?,
            );
            let (r, g, b) = hsl.rgb();
            return Ok(Color::rgb(r, g, b));
        }
        if let Some(n) = s.strip_prefix("palette99:") {
            let n: usize = n.trim().parse().map_err(|_| {
                Error::invalid(format!(
                    "bad colour '{text}': Palette99::pick takes a non-negative integer index"
                ))
            })?;
            return Ok(Color::palette99(n));
        }
        if s == "transparent" {
            return Ok(Color::TRANSPARENT);
        }
        PRELUDE
            .iter()
            .chain(FULL_PALETTE)
            .find(|(name, _)| *name == s)
            .map(|(_, c)| Color::rgb(c.0, c.1, c.2))
            .ok_or_else(unknown)
    }

    /// The colour with its alpha multiplied by `alpha`, as plotters' `Color::mix`.
    pub fn mix(self, alpha: f64) -> Result<Color> {
        if !(0.0..=1.0).contains(&alpha) {
            return Err(Error::invalid(format!(
                "mix alpha must be between 0 and 1, got {alpha}"
            )));
        }
        Ok(Color {
            a: self.a * alpha,
            ..self
        })
    }

    /// The colour as a string that [`Color::parse`] reads back to the same value.
    pub fn to_css(&self) -> String {
        if self.a == 1.0 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("rgba({}, {}, {}, {})", self.r, self.g, self.b, self.a)
        }
    }

    pub(crate) fn to_plotters(self) -> RGBAColor {
        RGBAColor(self.r, self.g, self.b, self.a)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_css())
    }
}

/// `mix(color, alpha)`: the colour string with its alpha multiplied, mirroring `WHITE.mix(0.8)`.
pub fn mix(color: &str, alpha: f64) -> Result<String> {
    Ok(Color::parse(color)?.mix(alpha)?.to_css())
}

fn parse_hex(hex: &str) -> Option<Color> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        6 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color {
            a: f64::from(byte(6)?) / 255.0,
            ..Color::rgb(byte(0)?, byte(2)?, byte(4)?)
        }),
        _ => None,
    }
}

fn function_args<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    s.strip_prefix(name)?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')
}

fn numbers<const N: usize>(text: &str, args: &str) -> Result<[f64; N]> {
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    let bad = || {
        Error::invalid(format!(
            "bad colour '{text}': expected {N} comma-separated numbers"
        ))
    };
    if parts.len() != N {
        return Err(bad());
    }
    let mut out = [0.0; N];
    for (slot, part) in out.iter_mut().zip(parts) {
        *slot = part.parse::<f64>().map_err(|_| bad())?;
        if !slot.is_finite() {
            return Err(bad());
        }
    }
    Ok(out)
}

fn byte(text: &str, v: f64) -> Result<u8> {
    if v.fract() == 0.0 && (0.0..=255.0).contains(&v) {
        Ok(v as u8)
    } else {
        Err(Error::invalid(format!(
            "bad colour '{text}': RGBColor components are integers from 0 to 255"
        )))
    }
}

fn unit(text: &str, what: &str, v: f64) -> Result<f64> {
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(Error::invalid(format!(
            "bad colour '{text}': {what} must be between 0 and 1"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Color {
        Color::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn prelude_names_win_over_full_palette() {
        assert_eq!(parse("red"), Color::rgb(255, 0, 0));
        assert_eq!(parse("RED"), Color::rgb(255, 0, 0));
        assert_eq!(parse("white"), Color::WHITE);
        assert_eq!(parse("transparent"), Color::TRANSPARENT);
    }

    #[test]
    fn full_palette_names() {
        assert_eq!(parse("blue_400"), Color::rgb(66, 165, 245));
        assert_eq!(parse("deeporange"), Color::rgb(255, 87, 34));
        assert_eq!(parse("red_500"), Color::rgb(244, 67, 54));
        assert_eq!(parse("grey_A100".trim()), parse("grey_a100"));
        assert_eq!(FULL_PALETTE.len(), 287);
    }

    #[test]
    fn hex() {
        assert_eq!(parse("#ff8000"), Color::rgb(255, 128, 0));
        assert_eq!(parse("#FF800080").a, 128.0 / 255.0);
        assert!(Color::parse("#ff80").is_err());
        assert!(Color::parse("#gg0000").is_err());
    }

    #[test]
    fn functions() {
        assert_eq!(parse("rgb(1, 2, 3)"), Color::rgb(1, 2, 3));
        assert_eq!(
            parse("rgba(1,2,3,0.5)"),
            Color {
                a: 0.5,
                ..Color::rgb(1, 2, 3)
            }
        );
        assert_eq!(parse("hsl(0, 1, 0.5)"), Color::rgb(255, 0, 0));
        assert!(Color::parse("rgb(256, 0, 0)").is_err());
        assert!(Color::parse("rgb(1.5, 0, 0)").is_err());
        assert!(Color::parse("rgb(1, 2)").is_err());
        assert!(Color::parse("rgba(1, 2, 3, 2)").is_err());
        assert!(Color::parse("hsl(120, 50, 50)").is_err());
    }

    #[test]
    fn palette99() {
        assert_eq!(parse("palette99:0"), Color::rgb(230, 25, 75));
        assert_eq!(parse("palette99:21"), parse("palette99:0"));
        assert!(Color::parse("palette99:-1").is_err());
    }

    #[test]
    fn unknown_colour_message() {
        let err = Color::parse("reddish").unwrap_err();
        assert!(
            err.message().starts_with("unknown colour 'reddish'"),
            "{err}"
        );
    }

    #[test]
    fn mix_multiplies_alpha() {
        assert_eq!(mix("white", 0.8).unwrap(), "rgba(255, 255, 255, 0.8)");
        assert_eq!(
            Color::parse(&mix("white", 0.8).unwrap()).unwrap(),
            Color::WHITE.mix(0.8).unwrap()
        );
        assert_eq!(
            mix("rgba(0, 0, 0, 0.5)", 0.5).unwrap(),
            "rgba(0, 0, 0, 0.25)"
        );
        assert!(mix("white", 1.5).is_err());
        assert!(mix("white", f64::NAN).is_err());
    }

    #[test]
    fn css_round_trip() {
        for s in ["#102030", "rgba(1, 2, 3, 0.25)"] {
            assert_eq!(parse(s).to_css(), s);
        }
    }
}
