//! The crate's one error type.

use std::fmt;

/// Why decoding a message failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The bytes are not JSON.
    Json(String),
    /// JSON that is not the MCP shape: `what` names the type or field.
    Decode {
        /// The type or field being decoded.
        what: &'static str,
        /// What was wrong with it.
        reason: String,
    },
}

impl Error {
    pub(crate) fn decode(what: &'static str, reason: impl Into<String>) -> Self {
        Error::Decode {
            what,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Json(reason) => write!(f, "invalid JSON: {reason}"),
            Error::Decode { what, reason } => write!(f, "invalid {what}: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

/// Shorthand for `Result<T, Error>`.
pub type Result<T> = core::result::Result<T, Error>;
