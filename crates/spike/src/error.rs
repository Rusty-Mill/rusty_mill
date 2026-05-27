//! Per-crate error enums (ADR-0023: one `thiserror` enum per library crate).

/// Errors raised by a tool body.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Filesystem / IO failure.
    #[error("io error: {0}")]
    Io(String),
    /// The model's JSON args did not match the tool's schema.
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    /// The tool exceeded its time budget.
    #[error("tool timed out")]
    Timeout,
    /// The result was produced but capped; the payload is the truncated text.
    #[error("result truncated")]
    Truncated(String),
    /// Anything else.
    #[error("{0}")]
    Other(String),
}

/// Errors raised by a [`crate::policy::Policy`] when it vetoes a tool call.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// The call was blocked; the string is a model-facing reason.
    #[error("{0}")]
    Blocked(String),
}

/// Errors raised by a [`crate::kernel::ChatModel`].
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// The underlying provider/model failed.
    #[error("model error: {0}")]
    Provider(String),
}
