use std::fmt;

/// Why decoding, verifying or reducing failed.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The bytes are not JSON.
    Json(String),
    /// JSON that is not the AG-UI shape: `what` names the type or field.
    Decode {
        /// The type or field being decoded.
        what: &'static str,
        /// What was wrong with it.
        reason: String,
    },
    /// An event violates the protocol's ordering rules.
    Sequence(String),
    /// A `STATE_DELTA` or `ACTIVITY_DELTA` patch could not be applied.
    Patch(rusty_json_patch::Error),
    /// The event consumer went away (the client closed the stream).
    Closed,
    /// The agent itself failed; the message becomes `RUN_ERROR`.
    Agent(String),
    /// The connection, request or response framing failed (client).
    Transport(String),
    /// The server refused the run with a non-200 status (client).
    Status {
        /// The HTTP status.
        status: u16,
        /// The response body, for the message.
        body: String,
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
            Error::Sequence(why) => write!(f, "protocol sequence error: {why}"),
            Error::Patch(e) => write!(f, "state delta failed: {e}"),
            Error::Closed => f.write_str("event stream closed"),
            Error::Agent(why) => write!(f, "agent failed: {why}"),
            Error::Transport(why) => write!(f, "transport error: {why}"),
            Error::Status { status, body } => write!(f, "server answered {status}: {body}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusty_json::Error> for Error {
    fn from(e: rusty_json::Error) -> Self {
        Error::Json(e.to_string())
    }
}

impl From<rusty_json_patch::Error> for Error {
    fn from(e: rusty_json_patch::Error) -> Self {
        Error::Patch(e)
    }
}
