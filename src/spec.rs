//! The chart specification carried in `CHART` values.
//!
//! A `CHART` is a BLOB holding [`MAGIC`] followed by a postcard-encoded [`Chart`]. Series
//! constructors produce one, modifiers decode, change and re-encode it, and outputs render it.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Prefix of every encoded chart. The trailing byte is the format version.
const MAGIC: &[u8; 4] = b"DKC\x01";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Chart {
    pub series: Vec<Series>,
    pub caption: Option<Caption>,
    pub x_desc: Option<String>,
    pub y_desc: Option<String>,
    pub x_range: Option<XRange>,
    pub y_range: Option<(f64, f64)>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Caption {
    pub text: String,
    pub size: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesKind {
    Line,
    Point,
    Bar,
}

impl SeriesKind {
    pub fn name(self) -> &'static str {
        match self {
            SeriesKind::Line => "line",
            SeriesKind::Point => "point",
            SeriesKind::Bar => "bar",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Series {
    pub kind: SeriesKind,
    pub label: Option<String>,
    pub x: XData,
    pub y: Vec<f64>,
}

/// The x values of a series. The variant decides the kind of x axis the chart gets.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum XData {
    F64(Vec<f64>),
    /// Microseconds since the Unix epoch, as DuckDB stores `TIMESTAMP`.
    Time(Vec<i64>),
    Category(Vec<String>),
}

impl XData {
    pub fn axis_kind(&self) -> AxisKind {
        match self {
            XData::F64(_) => AxisKind::F64,
            XData::Time(_) => AxisKind::Time,
            XData::Category(_) => AxisKind::Category,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisKind {
    F64,
    Time,
    Category,
}

impl fmt::Display for AxisKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AxisKind::F64 => "numeric",
            AxisKind::Time => "timestamp",
            AxisKind::Category => "categorical",
        })
    }
}

/// An explicit x-axis range, in the units of the axis it applies to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum XRange {
    F64(f64, f64),
    Time(i64, i64),
}

impl XRange {
    pub fn axis_kind(&self) -> AxisKind {
        match self {
            XRange::F64(..) => AxisKind::F64,
            XRange::Time(..) => AxisKind::Time,
        }
    }
}

#[derive(Debug)]
pub struct DecodeError;

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("value is not a CHART produced by this version of duckers")
    }
}

impl std::error::Error for DecodeError {}

impl Chart {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        // Serializing plain owned data into a Vec cannot fail.
        out.extend(postcard::to_allocvec(self).expect("chart serialization"));
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let body = bytes.strip_prefix(MAGIC).ok_or(DecodeError)?;
        postcard::from_bytes(body).map_err(|_| DecodeError)
    }

    /// The x-axis kind shared by all series, or an error naming the first mismatch.
    pub fn axis_kind(&self) -> Result<Option<AxisKind>, String> {
        let mut kinds = self.series.iter().map(|s| s.x.axis_kind());
        let Some(first) = kinds.next() else {
            return Ok(None);
        };
        if let Some(other) = kinds.find(|k| *k != first) {
            return Err(format!(
                "cannot draw {first} and {other} x values on the same chart"
            ));
        }
        Ok(Some(first))
    }

    /// A one-line description, used when a `CHART` is cast to `VARCHAR`.
    pub fn summary(&self) -> String {
        let mut kinds: Vec<&str> = Vec::new();
        for s in &self.series {
            if !kinds.contains(&s.kind.name()) {
                kinds.push(s.kind.name());
            }
        }
        let n = self.series.len();
        let points: usize = self.series.iter().map(|s| s.y.len()).sum();
        let mut out = format!("CHART({}, {n} series, {points} points", kinds.join("+"));
        if let Some(caption) = &self.caption {
            out.push_str(&format!(", caption '{}'", caption.text));
        }
        out.push(')');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Chart {
        Chart {
            series: vec![Series {
                kind: SeriesKind::Line,
                label: Some("a".into()),
                x: XData::F64(vec![1.0, 2.0]),
                y: vec![3.0, 4.0],
            }],
            caption: Some(Caption {
                text: "t".into(),
                size: 30,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn round_trip() {
        let chart = sample();
        assert_eq!(Chart::decode(&chart.encode()).unwrap(), chart);
    }

    #[test]
    fn rejects_foreign_blob() {
        assert!(Chart::decode(b"hello").is_err());
    }

    #[test]
    fn summary() {
        assert_eq!(
            sample().summary(),
            "CHART(line, 1 series, 2 points, caption 't')"
        );
    }

    #[test]
    fn mixed_axes() {
        let mut chart = sample();
        chart.series.push(Series {
            kind: SeriesKind::Bar,
            label: None,
            x: XData::Category(vec!["a".into()]),
            y: vec![1.0],
        });
        assert!(chart.axis_kind().is_err());
    }
}
