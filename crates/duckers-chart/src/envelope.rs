//! The BLOB layout of the SQL values.
//!
//! Every value is a 4-byte magic naming its type, the format version as a little-endian `u16`,
//! then the postcard encoding of the spec.

use crate::error::{Error, Result};
use crate::spec::{Chart, Font, Mesh, Series, SeriesLabels};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The format version this build writes and the only one it reads.
pub const FORMAT_VERSION: u16 = 2;

const HEADER_LEN: usize = 6;

/// The magics of all duckers value types, to name the type of a BLOB that is not the expected one.
const TYPES: [(&[u8; 4], &str); 5] = [
    (Chart::MAGIC, Chart::TYPE_NAME),
    (Series::MAGIC, Series::TYPE_NAME),
    (Mesh::MAGIC, Mesh::TYPE_NAME),
    (SeriesLabels::MAGIC, SeriesLabels::TYPE_NAME),
    (Font::MAGIC, Font::TYPE_NAME),
];

/// A spec type stored in a DuckDB custom type over `BLOB`.
pub trait Value: Serialize + DeserializeOwned {
    /// The SQL type name.
    const TYPE_NAME: &'static str;
    /// The first four bytes of every encoded value of this type.
    const MAGIC: &'static [u8; 4];

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(Self::MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        // Serializing owned plain data into a Vec has no failure mode.
        postcard::to_extend(self, out).expect("postcard serialization into a Vec")
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let name = Self::TYPE_NAME;
        if bytes.len() < HEADER_LEN || &bytes[..4] != Self::MAGIC {
            let other = TYPES
                .iter()
                .find(|(magic, _)| bytes.len() >= HEADER_LEN && &bytes[..4] == *magic);
            return Err(Error::decode(match other {
                Some((_, other)) => {
                    format!("not a duckers {name} value: the BLOB holds a {other} value")
                }
                None => format!("not a duckers {name} value"),
            }));
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != FORMAT_VERSION {
            return Err(Error::decode(format!(
                "{name} format version {version} not supported by this duckers \
                 (it reads version {FORMAT_VERSION})"
            )));
        }
        let (value, rest) = postcard::take_from_bytes(&bytes[HEADER_LEN..])
            .map_err(|e| Error::decode(format!("corrupt {name} value: {e}")))?;
        if !rest.is_empty() {
            return Err(Error::decode(format!(
                "corrupt {name} value: {} trailing bytes",
                rest.len()
            )));
        }
        Ok(value)
    }
}

impl Value for Chart {
    const TYPE_NAME: &'static str = "CHART";
    const MAGIC: &'static [u8; 4] = b"DKch";
}

impl Value for Series {
    const TYPE_NAME: &'static str = "SERIES";
    const MAGIC: &'static [u8; 4] = b"DKse";
}

impl Value for Mesh {
    const TYPE_NAME: &'static str = "MESH";
    const MAGIC: &'static [u8; 4] = b"DKme";
}

impl Value for SeriesLabels {
    const TYPE_NAME: &'static str = "SERIES_LABELS";
    const MAGIC: &'static [u8; 4] = b"DKsl";
}

impl Value for Font {
    const TYPE_NAME: &'static str = "FONT";
    const MAGIC: &'static [u8; 4] = b"DKfo";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accumulate::{Accumulator, SeriesAggregate, SeriesBinding, SqlType, XValue};

    fn sample_chart() -> Chart {
        let binding = SeriesBinding::new(SeriesAggregate::LineSeries, SqlType::Float).unwrap();
        let mut acc = Accumulator::default();
        for i in 0..3 {
            acc.push(
                &binding,
                Some(XValue::Number(i as f64)),
                Some(1.5),
                None,
                None,
            )
            .unwrap();
        }
        let series = acc.finish(&binding).unwrap().label("a");
        Chart::new()
            .caption("t", None)
            .unwrap()
            .configure_mesh()
            .x_desc("x")
            .draw()
            .draw_series(series)
            .unwrap()
    }

    #[test]
    fn round_trips() {
        let chart = sample_chart();
        assert_eq!(Chart::decode(&chart.encode()).unwrap(), chart);
        let mesh = chart.clone().configure_mesh().y_desc("y");
        assert_eq!(Mesh::decode(&mesh.encode()).unwrap(), mesh);
        let labels = chart.clone().configure_series_labels();
        assert_eq!(SeriesLabels::decode(&labels.encode()).unwrap(), labels);
        let series = chart.series().next().unwrap().clone();
        assert_eq!(Series::decode(&series.encode()).unwrap(), series);
        let font = Font::new("serif", 20, Some("bold")).unwrap();
        assert_eq!(Font::decode(&font.encode()).unwrap(), font);
    }

    #[test]
    fn encoding_is_deterministic() {
        assert_eq!(sample_chart().encode(), sample_chart().encode());
    }

    #[test]
    fn rejects_foreign_blobs() {
        for bytes in [&b""[..], b"hello", b"DKch", b"DKxx\x01\x00"] {
            let err = Chart::decode(bytes).unwrap_err();
            assert_eq!(err.message(), "not a duckers CHART value");
        }
    }

    #[test]
    fn names_the_other_type() {
        let series = sample_chart().series().next().unwrap().encode();
        let err = Chart::decode(&series).unwrap_err();
        assert_eq!(
            err.message(),
            "not a duckers CHART value: the BLOB holds a SERIES value"
        );
    }

    #[test]
    fn rejects_other_versions() {
        let mut bytes = sample_chart().encode();
        bytes[4..6].copy_from_slice(&7u16.to_le_bytes());
        let err = Chart::decode(&bytes).unwrap_err();
        assert_eq!(
            err.message(),
            "CHART format version 7 not supported by this duckers (it reads version 2)"
        );
        assert_eq!(err.kind(), crate::ErrorKind::Decode);
    }

    #[test]
    fn rejects_corrupt_payloads() {
        let bytes = sample_chart().encode();
        let err = Chart::decode(&bytes[..bytes.len() - 3]).unwrap_err();
        assert!(err.message().starts_with("corrupt CHART value"), "{err}");
        let mut longer = bytes.clone();
        longer.push(0);
        let err = Chart::decode(&longer).unwrap_err();
        assert_eq!(err.message(), "corrupt CHART value: 1 trailing bytes");
    }
}
