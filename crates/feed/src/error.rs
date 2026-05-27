//! `feed`'s tool-body error enum (ADR-0023; error-handling §2) and its mapping
//! into the structural [`ToolOutcome`] owned by `observe`. Composition is
//! downhill only: `feed` may wrap `constrain` and `observe` errors via `#[from]`.

use rk_observe::{ToolOutcome, ToolStatus};

/// Errors raised by a tool body or its dispatch.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Filesystem / IO failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A required argument was missing or the wrong shape.
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    /// The model's JSON args failed to deserialize.
    #[error("argument json error: {0}")]
    Json(#[from] serde_json::Error),
    /// No tool registered under this name.
    #[error("unknown tool {0}")]
    NotFound(String),
    /// The tool exceeded its time budget.
    #[error("tool timed out")]
    Timeout,
    /// The result was produced but capped; the payload is the truncated text.
    #[error("result truncated")]
    Truncated(String),
    /// A policy blocked the call (downhill: `feed` imports `constrain`).
    #[error(transparent)]
    Policy(#[from] rk_constrain::PolicyError),
    /// A storage/observe failure surfaced through a tool.
    #[error(transparent)]
    Storage(#[from] rk_observe::ObserveError),
    /// A memory database (SQLite) failure.
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),
    /// Anything else.
    #[error("{0}")]
    Other(String),
}

/// Map a tool-body error to a structural outcome. Lives here (not as a `From`
/// impl in `observe`) so `observe` stays a leaf above `config` and the orphan
/// rule is respected. Mirrors the error→model-surface table (error-handling §6).
pub fn outcome_from_error(e: ToolError) -> ToolOutcome {
    match e {
        ToolError::Timeout => ToolOutcome::new(ToolStatus::Timeout, e.to_string()),
        ToolError::Truncated(s) => ToolOutcome::new(ToolStatus::Truncated, s),
        ToolError::Policy(_) => ToolOutcome::new(ToolStatus::Blocked, e.to_string()),
        other => ToolOutcome::error(other.to_string()),
    }
}
