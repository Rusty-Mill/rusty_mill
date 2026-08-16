//! One error type for the composition root.

use std::path::PathBuf;

use sessionmgr_protocol::ErrorKind;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The user asked for something that isn't a valid invocation.
    #[error("{message}")]
    Usage { message: String },

    #[error("no such session `{id}`")]
    NotFound { id: String },

    /// The request is well-formed but not legal right now.
    #[error("{message}")]
    Conflict { message: String },

    /// A peer sent something unparseable, or sent a request on the wrong
    /// transport.
    #[error("protocol error: {message}")]
    Protocol { message: String },

    /// Carries the path where one is known: an I/O error whose message
    /// doesn't say *which* file it was about is the classic unactionable
    /// error report.
    #[error("{context}{}: {source}", path.as_ref().map(|p| format!(" ({})", p.display())).unwrap_or_default())]
    Io {
        context: &'static str,
        path: Option<PathBuf>,
        #[source]
        source: std::io::Error,
    },

    #[error("could not encode/decode message: {0}")]
    Json(#[from] serde_json::Error),
}

/// A rejected state transition is a conflict, not an internal failure:
/// it means the caller asked for something that is not legal right now
/// (closing an already-closed session, resuming an exited one), which is
/// a user-facing answer rather than a bug.
impl From<sessionmgr_core::TransitionError> for Error {
    fn from(e: sessionmgr_core::TransitionError) -> Self {
        Error::Conflict {
            message: e.to_string(),
        }
    }
}

impl Error {
    pub fn io(context: &'static str, path: impl Into<Option<PathBuf>>, source: std::io::Error) -> Self {
        Error::Io {
            context,
            path: path.into(),
            source,
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Error::Usage {
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Error::Conflict {
            message: message.into(),
        }
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Error::Protocol {
            message: message.into(),
        }
    }

    /// The wire-visible classification of this error.
    ///
    /// `NotFound` in particular must survive the trip to the client as
    /// its own kind, because Phase 4's `__hook-fire` has to treat an
    /// unrecognised session id as a silent no-op rather than a failure.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Error::NotFound { .. } => ErrorKind::NotFound,
            Error::Conflict { .. } => ErrorKind::Conflict,
            Error::Protocol { .. } | Error::Usage { .. } | Error::Json(_) => ErrorKind::Protocol,
            Error::Io { .. } => ErrorKind::Internal,
        }
    }

    /// Process exit code for this error when it reaches `main`.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage { .. } => 2,
            Error::Conflict { .. } => 3,
            Error::NotFound { .. } => 4,
            _ => 1,
        }
    }
}
