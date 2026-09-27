//! The label formats of `x_label_formatter(fmt)` and `y_label_formatter(fmt)`, which stand in
//! for plotters' formatter closures.
//!
//! A format is one of:
//!
//! - a DuckDB `format()`-style template for numeric, integer-bucket, log and category axes:
//!   literal text around exactly one `{}` placeholder, `{{` and `}}` for literal braces. The
//!   placeholder takes an optional spec, `{:[[fill]align][sign][0][width][,][.precision][type]}`,
//!   with `align` one of `<`, `>`, `^`, `sign` one of `+`, `-`, space, and `type` one of `f`
//!   (fixed), `e` (exponent), `g` (general), `d` (rounded to an integer) and `%` (times 100,
//!   fixed, with a percent sign). `{}` without a spec is the axis's own label text, as plotters
//!   formats it; a spec with a precision and no type is `g`.
//! - a chrono `strftime` pattern for date and timestamp axes, such as `'%b %d'`.
//!
//! A category axis takes a template whose spec has only fill, alignment, width and precision
//! (which truncates the name); the numeric parts are for numbers.

use crate::error::{Error, Result};
use chrono::NaiveDateTime;
use chrono::format::{Item, StrftimeItems};
use std::fmt::Write;

/// A parsed `x_label_formatter`/`y_label_formatter` argument.
#[derive(Clone, Debug, PartialEq)]
pub enum LabelFormat {
    Template(Template),
    Strftime(String),
}

/// A `format()` template with one placeholder.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    prefix: String,
    spec: Option<Spec>,
    suffix: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Align {
    Left,
    Right,
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sign {
    Minus,
    Plus,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Fixed,
    Exponent,
    General,
    Integer,
    Percent,
}

/// `[[fill]align][sign][0][width][,][.precision][type]`.
#[derive(Clone, Debug, PartialEq)]
struct Spec {
    fill: char,
    align: Option<Align>,
    sign: Sign,
    zero: bool,
    width: usize,
    comma: bool,
    precision: Option<usize>,
    kind: Option<Kind>,
}

/// Widths and precisions above this are a mistake (a label is at most a chart wide).
const MAX_WIDTH: usize = 100;

impl LabelFormat {
    /// Parses a formatter argument: a template if it has a placeholder, else a strftime
    /// pattern if it has a conversion. `what` names the function in errors.
    pub fn parse(what: &str, text: &str) -> Result<LabelFormat> {
        if text.contains('{') || text.contains('}') {
            return Template::parse(text)
                .map(LabelFormat::Template)
                .map_err(|e| Error::invalid(format!("{what}: {e}")));
        }
        let items: Vec<Item> = StrftimeItems::new(text).collect();
        if items.iter().any(|i| matches!(i, Item::Error)) {
            return Err(Error::invalid(format!(
                "{what}: '{text}' is not a valid strftime pattern"
            )));
        }
        let converts = items
            .iter()
            .any(|i| !matches!(i, Item::Literal(_) | Item::OwnedLiteral(_) | Item::Space(_)));
        if !converts {
            return Err(Error::invalid(format!(
                "{what}: '{text}' is neither a format() template with one {{}} placeholder \
                 (numeric and category axes, e.g. '{{:.1f}} °C') nor a strftime pattern (date \
                 and timestamp axes, e.g. '%b %d')"
            )));
        }
        let sample = NaiveDateTime::default();
        let mut out = String::new();
        if write!(out, "{}", sample.format(text)).is_err() {
            return Err(Error::invalid(format!(
                "{what}: the strftime pattern '{text}' needs a time zone (%z, %Z), which \
                 duckers' dates and timestamps do not have"
            )));
        }
        Ok(LabelFormat::Strftime(text.to_string()))
    }

    /// `"template"` or `"strftime pattern"`, for error messages.
    pub fn kind_name(&self) -> &'static str {
        match self {
            LabelFormat::Template(_) => "format() template",
            LabelFormat::Strftime(_) => "strftime pattern",
        }
    }
}

/// Formats a date or timestamp with a validated strftime pattern.
pub fn strftime(pattern: &str, value: NaiveDateTime) -> String {
    let mut out = String::new();
    // `LabelFormat::parse` rejects the patterns that fail on a naive date-time.
    let _ = write!(out, "{}", value.format(pattern));
    out
}

impl Template {
    fn parse(text: &str) -> std::result::Result<Template, String> {
        let mut parts: Vec<String> = vec![String::new()];
        let mut specs: Vec<Option<Spec>> = Vec::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    parts.last_mut().expect("one part").push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    parts.last_mut().expect("one part").push('}');
                }
                '{' => {
                    let mut inner = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(c) => inner.push(c),
                            None => {
                                return Err(format!(
                                    "the template '{text}' has a '{{' without a closing '}}' \
                                     (write '{{{{' for a literal brace)"
                                ));
                            }
                        }
                    }
                    specs.push(match inner.strip_prefix(':') {
                        _ if inner.is_empty() => None,
                        Some("") => None,
                        Some(spec) => Some(Spec::parse(spec).map_err(|e| {
                            format!("the template '{text}' has an unsupported spec ':{spec}': {e}")
                        })?),
                        None => {
                            return Err(format!(
                                "the template '{text}' has the placeholder '{{{inner}}}'; label \
                                 templates take '{{}}' or '{{:spec}}' only"
                            ));
                        }
                    });
                    parts.push(String::new());
                }
                '}' => {
                    return Err(format!(
                        "the template '{text}' has a '}}' without an opening '{{' (write '}}}}' \
                         for a literal brace)"
                    ));
                }
                c => parts.last_mut().expect("one part").push(c),
            }
        }
        if specs.len() != 1 {
            return Err(format!(
                "the template '{text}' has {} placeholders; a label template takes exactly one \
                 '{{}}'",
                specs.len()
            ));
        }
        let suffix = parts.pop().expect("two parts");
        let prefix = parts.pop().expect("two parts");
        Ok(Template {
            prefix,
            spec: specs.pop().expect("one spec"),
            suffix,
        })
    }

    /// Whether the template can label a category: no sign, zero padding, grouping or type.
    pub fn fits_text(&self) -> bool {
        self.spec
            .as_ref()
            .is_none_or(|s| s.sign == Sign::Minus && !s.zero && !s.comma && s.kind.is_none())
    }

    /// The label of a number. `default` is the axis's own label text for it, which `{}`
    /// without a spec (or a spec with neither precision nor type) shows.
    pub fn number(&self, value: f64, default: &str) -> String {
        let body = match &self.spec {
            None => default.to_string(),
            Some(spec) => spec.number(value, default),
        };
        format!("{}{body}{}", self.prefix, self.suffix)
    }

    /// The label of a category name. Call only when [`Template::fits_text`] holds.
    pub fn text(&self, name: &str) -> String {
        let body = match &self.spec {
            None => name.to_string(),
            Some(spec) => {
                let name: String = match spec.precision {
                    Some(p) => name.chars().take(p).collect(),
                    None => name.to_string(),
                };
                spec.pad("", &name, Align::Left)
            }
        };
        format!("{}{body}{}", self.prefix, self.suffix)
    }
}

impl Spec {
    fn parse(spec: &str) -> std::result::Result<Spec, String> {
        let chars: Vec<char> = spec.chars().collect();
        let mut i = 0;
        let align_of = |c: char| match c {
            '<' => Some(Align::Left),
            '>' => Some(Align::Right),
            '^' => Some(Align::Center),
            _ => None,
        };
        let (mut fill, mut align) = (' ', None);
        if chars.len() >= 2
            && let Some(a) = align_of(chars[1])
        {
            (fill, align) = (chars[0], Some(a));
            i = 2;
        } else if let Some(a) = chars.first().copied().and_then(align_of) {
            align = Some(a);
            i = 1;
        }
        let mut sign = Sign::Minus;
        match chars.get(i) {
            Some('+') => (sign, i) = (Sign::Plus, i + 1),
            Some('-') => i += 1,
            Some(' ') => (sign, i) = (Sign::Space, i + 1),
            _ => {}
        }
        let zero = chars.get(i) == Some(&'0');
        if zero {
            i += 1;
        }
        let digits = |i: &mut usize| -> std::result::Result<Option<usize>, String> {
            let start = *i;
            while chars.get(*i).is_some_and(|c| c.is_ascii_digit()) {
                *i += 1;
            }
            if *i == start {
                return Ok(None);
            }
            let n: String = chars[start..*i].iter().collect();
            match n.parse::<usize>() {
                Ok(n) if n <= MAX_WIDTH => Ok(Some(n)),
                _ => Err(format!("{n} is more than {MAX_WIDTH}")),
            }
        };
        let width = digits(&mut i)?.unwrap_or(0);
        let comma = chars.get(i) == Some(&',');
        if comma {
            i += 1;
        }
        let mut precision = None;
        if chars.get(i) == Some(&'.') {
            i += 1;
            precision = Some(digits(&mut i)?.ok_or("'.' needs a precision")?);
        }
        let kind = match chars.get(i) {
            None => None,
            Some(c) => {
                i += 1;
                Some(match c {
                    'f' | 'F' => Kind::Fixed,
                    'e' | 'E' => Kind::Exponent,
                    'g' | 'G' => Kind::General,
                    'd' => Kind::Integer,
                    '%' => Kind::Percent,
                    _ => return Err(format!("unknown type '{c}' (expected f, e, g, d or %)")),
                })
            }
        };
        if i != chars.len() {
            return Err("expected [[fill]align][sign][0][width][,][.precision][type]".into());
        }
        if kind == Some(Kind::Integer) && precision.is_some() {
            return Err("type d takes no precision".into());
        }
        Ok(Spec {
            fill,
            align,
            sign,
            zero,
            width,
            comma,
            precision,
            kind,
        })
    }

    fn number(&self, value: f64, default: &str) -> String {
        let negative = value < 0.0;
        let abs = value.abs();
        let body = match (self.kind, self.precision) {
            (None, None) => default.strip_prefix('-').unwrap_or(default).to_string(),
            (None, Some(p)) | (Some(Kind::General), Some(p)) => general(abs, p),
            (Some(Kind::General), None) => general(abs, 6),
            (Some(Kind::Fixed), p) => format!("{abs:.*}", p.unwrap_or(6)),
            (Some(Kind::Exponent), p) => exponent(abs, p.unwrap_or(6)),
            (Some(Kind::Integer), _) => format!("{abs:.0}"),
            (Some(Kind::Percent), p) => format!("{:.*}%", p.unwrap_or(6), abs * 100.0),
        };
        let body = if self.comma { group(&body) } else { body };
        let sign = match (negative, self.sign) {
            (true, _) => "-",
            (false, Sign::Plus) => "+",
            (false, Sign::Space) => " ",
            (false, Sign::Minus) => "",
        };
        if self.zero && self.align.is_none() {
            let pad = self.width.saturating_sub(sign.len() + body.chars().count());
            format!("{sign}{}{body}", "0".repeat(pad))
        } else {
            self.pad(sign, &body, Align::Right)
        }
    }

    /// `sign` and `body` padded with the fill to the width, aligned as the spec says or as
    /// `default`.
    fn pad(&self, sign: &str, body: &str, default: Align) -> String {
        let len = sign.chars().count() + body.chars().count();
        let pad = self.width.saturating_sub(len);
        let fill = |n: usize| self.fill.to_string().repeat(n);
        match self.align.unwrap_or(default) {
            Align::Left => format!("{sign}{body}{}", fill(pad)),
            Align::Right => format!("{}{sign}{body}", fill(pad)),
            Align::Center => format!("{}{sign}{body}{}", fill(pad / 2), fill(pad - pad / 2)),
        }
    }
}

/// `{:e}` in the style of fmt and Python: `1.500000e+03`.
fn exponent(abs: f64, precision: usize) -> String {
    let text = format!("{abs:.precision$e}");
    let (mantissa, exp) = text.split_once('e').expect("Rust's {:e} has an exponent");
    let exp: i32 = exp.parse().expect("an integer exponent");
    let sign = if exp < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exp.abs())
}

/// `{:g}`: `precision` significant digits, fixed or exponent notation by the exponent,
/// trailing zeros removed.
fn general(abs: f64, precision: usize) -> String {
    let p = precision.max(1);
    if abs == 0.0 {
        return "0".into();
    }
    // The exponent after rounding to p significant digits.
    let rounded = format!("{abs:.*e}", p - 1);
    let exp: i32 = rounded
        .split_once('e')
        .and_then(|(_, e)| e.parse().ok())
        .expect("Rust's {:e} has an exponent");
    let strip = |s: String| -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    if exp < -4 || exp >= p as i32 {
        let text = exponent(abs, p - 1);
        let (mantissa, exp) = text.split_once('e').expect("an exponent");
        format!("{}e{exp}", strip(mantissa.to_string()))
    } else {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        strip(format!("{abs:.decimals$}"))
    }
}

/// Thousands separators in the integer part of a number's text.
fn group(body: &str) -> String {
    let end = body
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(body.len());
    let (int, rest) = body.split_at(end);
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out + rest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(text: &str) -> Template {
        match LabelFormat::parse("f", text).unwrap() {
            LabelFormat::Template(t) => t,
            other => panic!("not a template: {other:?}"),
        }
    }

    fn num(text: &str, value: f64) -> String {
        template(text).number(value, "DEFAULT")
    }

    #[test]
    fn plain_placeholder_is_the_default_text() {
        assert_eq!(num("{}", 1.5), "DEFAULT");
        assert_eq!(num("{:}", 1.5), "DEFAULT");
        assert_eq!(num("{} °C", 1.5), "DEFAULT °C");
        assert_eq!(template("[{}]").number(-2.0, "-2.0"), "[-2.0]");
        assert_eq!(template("{:>6}").number(-2.0, "-2.0"), "  -2.0");
        assert_eq!(template("{:+}").number(2.0, "2.0"), "+2.0");
        assert_eq!(
            template("{:,}").number(1234567.0, "1234567.0"),
            "1,234,567.0"
        );
        assert_eq!(template("{:06}").number(-2.5, "-2.5"), "-002.5");
    }

    #[test]
    fn fixed_exponent_general_integer_percent() {
        assert_eq!(num("{:.1f}", 3.14259), "3.1");
        assert_eq!(num("{:f}", 2.0), "2.000000");
        assert_eq!(num("{:.0f}", 2.5), "2");
        assert_eq!(num("{:.2e}", 1234.5), "1.23e+03");
        assert_eq!(num("{:e}", 0.00012), "1.200000e-04");
        assert_eq!(num("{:g}", 1234.5), "1234.5");
        assert_eq!(num("{:g}", 1234567.0), "1.23457e+06");
        assert_eq!(num("{:.3g}", 0.0001234), "0.000123");
        assert_eq!(num("{:g}", 0.00001), "1e-05");
        assert_eq!(num("{:.2}", 3.14259), "3.1");
        assert_eq!(num("{:g}", 0.0), "0");
        assert_eq!(num("{:d}", 41.6), "42");
        assert_eq!(num("{:.0%}", 0.25), "25%");
        assert_eq!(num("{:.1%}", 0.1234), "12.3%");
        assert_eq!(num("{:.1f}", -0.04), "-0.0");
    }

    #[test]
    fn sign_width_fill_alignment_grouping() {
        assert_eq!(num("{:+.1f}", 2.0), "+2.0");
        assert_eq!(num("{: .1f}", 2.0), " 2.0");
        assert_eq!(num("{:+.1f}", -2.0), "-2.0");
        assert_eq!(num("{:8.2f}", 3.14259), "    3.14");
        assert_eq!(num("{:<8.2f}|", 3.14259), "3.14    |");
        assert_eq!(num("{:^8.2f}|", 3.14259), "  3.14  |");
        assert_eq!(num("{:*>8.2f}", 3.14259), "****3.14");
        assert_eq!(num("{:08.2f}", -3.14259), "-0003.14");
        assert_eq!(num("{:,.0f}", 1234567.0), "1,234,567");
        assert_eq!(num("{:,.2f}", 999.0), "999.00");
        assert_eq!(num("{:,d}", -1234.0), "-1,234");
        assert_eq!(num("{:,}", 100.0), "DEFAULT");
    }

    #[test]
    fn braces_and_literal_text() {
        assert_eq!(num("{{{:.0f}}}", 7.0), "{7}");
        assert_eq!(num("≈ {:.1f} %", 7.0), "≈ 7.0 %");
        assert_eq!(num("}}{}{{", 7.0), "}DEFAULT{");
    }

    #[test]
    fn categories() {
        assert!(template("{}").fits_text());
        assert!(template("<{:>8.3}>").fits_text());
        assert_eq!(template("<{:>8.3}>").text("economy"), "<     eco>");
        assert_eq!(template("{:^7}").text("abc"), "  abc  ");
        assert_eq!(template("{:7}|").text("abc"), "abc    |");
        assert!(!template("{:.1f}").fits_text());
        assert!(!template("{:+}").fits_text());
        assert!(!template("{:,}").fits_text());
        assert!(!template("{:05}").fits_text());
    }

    #[test]
    fn strftime_patterns() {
        assert_eq!(
            LabelFormat::parse("f", "%b %d").unwrap(),
            LabelFormat::Strftime("%b %d".into())
        );
        let t = NaiveDateTime::parse_from_str("2024-03-05 14:30:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(strftime("%b %d, %H:%M", t), "Mar 05, 14:30");
        // A literal percent sign after a placeholder is a template, not a pattern.
        assert!(matches!(
            LabelFormat::parse("f", "{:.0f}%").unwrap(),
            LabelFormat::Template(_)
        ));
    }

    #[test]
    fn errors() {
        let err = |text: &str| {
            LabelFormat::parse("x_label_formatter", text)
                .unwrap_err()
                .message()
                .to_string()
        };
        assert_eq!(
            err("abc"),
            "x_label_formatter: 'abc' is neither a format() template with one {} placeholder \
             (numeric and category axes, e.g. '{:.1f} °C') nor a strftime pattern (date and \
             timestamp axes, e.g. '%b %d')"
        );
        assert_eq!(
            err("{} {}"),
            "x_label_formatter: the template '{} {}' has 2 placeholders; a label template takes \
             exactly one '{}'"
        );
        assert!(err("{").contains("without a closing"), "{}", err("{"));
        assert!(err("a}").contains("without an opening"), "{}", err("a}"));
        assert!(
            err("{0}").contains("take '{}' or '{:spec}' only"),
            "{}",
            err("{0}")
        );
        assert!(
            err("{:.1x}").contains("unknown type 'x'"),
            "{}",
            err("{:.1x}")
        );
        assert!(
            err("{:.f}").contains("'.' needs a precision"),
            "{}",
            err("{:.f}")
        );
        assert!(err("{:.1d}").contains("type d takes no precision"));
        assert!(err("{:1000}").contains("1000 is more than 100"));
        assert!(err("{:>>>}").contains("unsupported spec"));
        assert!(
            err("%Q").contains("not a valid strftime pattern"),
            "{}",
            err("%Q")
        );
        assert!(err("%Z").contains("needs a time zone"), "{}", err("%Z"));
        assert!(err("").contains("is neither"));
    }
}
