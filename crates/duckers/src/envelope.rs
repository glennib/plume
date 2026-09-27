//! The byte layout of duckers values, shared by every custom type.
//!
//! A `CHART`, `MESH`, `SERIES_LABELS`, `SERIES` or `FONT` value is a `BLOB` holding
//!
//! ```text
//! b"DKRS" | kind tag (u8) | format version (u16, little endian) | payload
//! ```
//!
//! The payload is the postcard-encoded spec from `duckers-chart`. The envelope lets duckers
//! reject bytes that are not one of its values (a `BLOB` cast to `CHART`, or a `MESH` passed where
//! a `CHART` belongs), and values written by an incompatible duckers, with a clear message instead
//! of a decoding error deep in the payload.

use std::fmt;

/// The kinds of duckers values, one per custom SQL type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueKind {
    Chart,
    Mesh,
    SeriesLabels,
    Series,
    Font,
}

impl ValueKind {
    pub const ALL: [ValueKind; 5] = [
        ValueKind::Chart,
        ValueKind::Mesh,
        ValueKind::SeriesLabels,
        ValueKind::Series,
        ValueKind::Font,
    ];

    /// The SQL type name.
    pub fn sql_name(self) -> &'static str {
        match self {
            ValueKind::Chart => "CHART",
            ValueKind::Mesh => "MESH",
            ValueKind::SeriesLabels => "SERIES_LABELS",
            ValueKind::Series => "SERIES",
            ValueKind::Font => "FONT",
        }
    }

    /// The tag byte in the envelope. Stable: tags are never reused.
    pub fn tag(self) -> u8 {
        match self {
            ValueKind::Chart => 1,
            ValueKind::Mesh => 2,
            ValueKind::SeriesLabels => 3,
            ValueKind::Series => 4,
            ValueKind::Font => 5,
        }
    }

    pub fn from_tag(tag: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.tag() == tag)
    }

    /// Looks a kind up by SQL type name, case-insensitively.
    pub fn from_sql_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.sql_name().eq_ignore_ascii_case(name))
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.sql_name())
    }
}

pub const MAGIC: [u8; 4] = *b"DKRS";

/// The payload format this duckers reads and writes. Bump it when the payload encoding changes
/// incompatibly.
pub const FORMAT_VERSION: u16 = 1;

const HEADER_LEN: usize = MAGIC.len() + 1 + 2;

/// Why bytes are not a valid value of the expected kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// Not a duckers value at all.
    NotDuckers { expected: ValueKind },
    /// A duckers value of another kind.
    WrongKind {
        expected: ValueKind,
        found: Option<ValueKind>,
    },
    /// Written by a duckers with a different payload format.
    UnsupportedVersion { kind: ValueKind, version: u16 },
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvelopeError::NotDuckers { expected } => {
                write!(f, "not a duckers {expected} value")
            }
            EnvelopeError::WrongKind {
                expected,
                found: Some(found),
            } => write!(f, "expected a {expected} value, got a {found} value"),
            EnvelopeError::WrongKind {
                expected,
                found: None,
            } => write!(
                f,
                "expected a {expected} value, got an unknown duckers value"
            ),
            EnvelopeError::UnsupportedVersion { kind, version } => write!(
                f,
                "{kind} value has format version {version}, but this duckers reads version \
                 {FORMAT_VERSION}; rebuild it with this duckers"
            ),
        }
    }
}

impl std::error::Error for EnvelopeError {}

/// Wraps a payload in the envelope for `kind`.
pub fn encode(kind: ValueKind, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&MAGIC);
    out.push(kind.tag());
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Checks the envelope of a `kind` value and returns its payload.
pub fn decode(kind: ValueKind, bytes: &[u8]) -> Result<&[u8], EnvelopeError> {
    if bytes.len() < HEADER_LEN || bytes[..MAGIC.len()] != MAGIC {
        return Err(EnvelopeError::NotDuckers { expected: kind });
    }
    let tag = bytes[MAGIC.len()];
    if tag != kind.tag() {
        return Err(EnvelopeError::WrongKind {
            expected: kind,
            found: ValueKind::from_tag(tag),
        });
    }
    let version = u16::from_le_bytes([bytes[MAGIC.len() + 1], bytes[MAGIC.len() + 2]]);
    if version != FORMAT_VERSION {
        return Err(EnvelopeError::UnsupportedVersion { kind, version });
    }
    Ok(&bytes[HEADER_LEN..])
}

/// Produces the one-line text of a value, which is what `value::VARCHAR` returns and what the
/// DuckDB CLI shows for the value in every output mode. It receives the checked payload.
///
/// The spec types live in `duckers-chart`; its summary (`CHART(line, 2 series, 240 points)`)
/// plugs in here. [`byte_count_summary`] stands in until then.
pub type Summarizer = fn(kind: ValueKind, payload: &[u8]) -> Result<String, String>;

/// `CHART(<n> bytes)`: the summary for payloads duckers cannot decode yet.
pub fn byte_count_summary(kind: ValueKind, payload: &[u8]) -> Result<String, String> {
    Ok(format!("{kind}({} bytes)", payload.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for kind in ValueKind::ALL {
            let bytes = encode(kind, b"payload");
            assert_eq!(decode(kind, &bytes), Ok(&b"payload"[..]));
        }
    }

    #[test]
    fn empty_payload() {
        let bytes = encode(ValueKind::Chart, b"");
        assert_eq!(bytes.len(), HEADER_LEN);
        assert_eq!(decode(ValueKind::Chart, &bytes), Ok(&b""[..]));
    }

    #[test]
    fn rejects_foreign_bytes() {
        assert_eq!(
            decode(ValueKind::Chart, b"abc"),
            Err(EnvelopeError::NotDuckers {
                expected: ValueKind::Chart
            })
        );
        assert_eq!(
            decode(ValueKind::Chart, b"XXXX\x01\x01\x00"),
            Err(EnvelopeError::NotDuckers {
                expected: ValueKind::Chart
            })
        );
    }

    #[test]
    fn rejects_other_kinds() {
        let mesh = encode(ValueKind::Mesh, b"m");
        let err = decode(ValueKind::Chart, &mesh).unwrap_err();
        assert_eq!(err.to_string(), "expected a CHART value, got a MESH value");
        let mut unknown = mesh.clone();
        unknown[4] = 200;
        assert!(matches!(
            decode(ValueKind::Chart, &unknown),
            Err(EnvelopeError::WrongKind { found: None, .. })
        ));
    }

    #[test]
    fn rejects_other_versions() {
        let mut bytes = encode(ValueKind::Series, b"s");
        bytes[5..7].copy_from_slice(&7u16.to_le_bytes());
        assert_eq!(
            decode(ValueKind::Series, &bytes),
            Err(EnvelopeError::UnsupportedVersion {
                kind: ValueKind::Series,
                version: 7
            })
        );
    }

    #[test]
    fn layout_is_stable() {
        assert_eq!(
            encode(ValueKind::SeriesLabels, b"\xff"),
            b"DKRS\x03\x01\x00\xff".to_vec()
        );
    }

    #[test]
    fn names_and_tags() {
        for kind in ValueKind::ALL {
            assert_eq!(ValueKind::from_tag(kind.tag()), Some(kind));
            assert_eq!(
                ValueKind::from_sql_name(&kind.sql_name().to_lowercase()),
                Some(kind)
            );
        }
        assert_eq!(
            byte_count_summary(ValueKind::Font, b"abc").unwrap(),
            "FONT(3 bytes)"
        );
    }
}
