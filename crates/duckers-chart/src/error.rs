//! The one error type of the crate.

use std::fmt;

/// What went wrong, for callers that map errors to different SQL error classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// A BLOB that is not a duckers value of the expected type, or of an unsupported format
    /// version.
    Decode,
    /// An argument the call cannot accept: an unknown colour, a bad range, mismatched axis kinds,
    /// a method that does not apply to the series kind, a bad image size.
    Invalid,
    /// plotters failed while drawing. This does not happen for valid values short of a bug.
    Render,
}

/// An error with a message meant for the SQL user, naming the plotters term where one exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Error {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn decode(message: impl Into<String>) -> Self {
        Error::new(ErrorKind::Decode, message)
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Error::new(ErrorKind::Invalid, message)
    }

    pub(crate) fn render(message: impl fmt::Display) -> Self {
        Error::new(ErrorKind::Render, format!("plotters: {message}"))
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
