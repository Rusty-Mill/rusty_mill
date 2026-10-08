use std::fmt;

/// Why decoding failed.
#[derive(Clone, Debug, PartialEq)]
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
            Error::Json(why) => write!(f, "invalid JSON: {why}"),
            Error::Decode { what, reason } => write!(f, "invalid {what}: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusty_json::Error> for Error {
    fn from(e: rusty_json::Error) -> Self {
        Error::Json(e.to_string())
    }
}
