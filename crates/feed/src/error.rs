//! `feed`'s tool-body error enum (ADR-0023) and its mapping into the structural
//! [`ToolOutcome`] owned by `observe`.

use rk_observe::{ToolOutcome, ToolStatus};

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

/// Map a tool-body error to a structural outcome. Lives here (not as a `From`
/// impl in `observe`) so `observe` stays a leaf above `config` and the orphan
/// rule is respected.
pub fn outcome_from_error(e: ToolError) -> ToolOutcome {
    match e {
        ToolError::Timeout => ToolOutcome::new(ToolStatus::Timeout, e.to_string()),
        ToolError::Truncated(s) => ToolOutcome::new(ToolStatus::Truncated, s),
        other => ToolOutcome::error(other.to_string()),
    }
}
