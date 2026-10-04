//! The runtime's error type.

use rsi_core::CoreError;
use rusty_err::Error;

/// An infrastructure failure: something the harness could not do, as
/// opposed to a solution that ran and failed (which is an outcome).
#[derive(Debug, Error)]
pub enum RuntimeError {
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
    /// A task directory or manifest is malformed.
    #[error("invalid task: {0}")]
    Task(String),
    /// The out-of-process grader failed or answered nonsense.
    #[error("grader failed: {0}")]
    Grader(String),
    /// The model endpoint failed or answered nonsense.
    #[error("model endpoint: {0}")]
    Model(String),
    /// A broker transcript is malformed, or a replay diverged from it.
    #[error("broker: {0}")]
    Broker(String),
    /// A harness that must build (the baseline, or one being replayed)
    /// did not; the compiler's output.
    #[error("the harness does not build: {0}")]
    Harness(String),
    /// A git command failed.
    #[error("git: {0}")]
    Git(String),
    /// The run's lineage or a blob is malformed, inconsistent or altered.
    #[error("lineage: {0}")]
    Lineage(String),
    /// A domain value was rejected.
    #[error("{0}")]
    Core(#[from] CoreError),
}

impl RuntimeError {
    /// Wraps an I/O error with what was being attempted.
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}
