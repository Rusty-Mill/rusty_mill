//! The crate's error type.

use rusty_err::Error as DeriveError;

/// What can go wrong setting up or running a sandbox. A program that ran
/// and failed is an [`crate::ExecOutcome`], not an error.
#[derive(Debug, DeriveError)]
pub enum Error {
    /// A filesystem or process operation failed.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The sandbox could not be set up, so nothing was run (fail closed).
    #[error("sandbox unavailable: {0}")]
    Sandbox(String),
    /// A limit or a path in a [`crate::SandboxSpec`] was rejected.
    #[error("invalid {what}: `{value}`")]
    Invalid {
        /// What was being validated.
        what: &'static str,
        /// The rejected value.
        value: String,
    },
}

impl Error {
    /// Wraps an I/O error with what was being attempted.
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}
