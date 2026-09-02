//! One error type for the TUI's own socket client and terminal setup.

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{context}: {source}")]
    Io {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("could not encode/decode a message: {0}")]
    Json(#[from] serde_json::Error),

    /// The daemon answered with `Response::Error`.
    #[error("{message}")]
    Daemon { message: String },

    /// The daemon sent something other than the response type a request
    /// expected, or closed the connection without answering.
    #[error("protocol error: {message}")]
    Protocol { message: String },
}

impl Error {
    pub fn io(context: &'static str, source: std::io::Error) -> Self {
        Error::Io { context, source }
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Error::Protocol {
            message: message.into(),
        }
    }
}
