//! Structured tool result contract (ADR-0022).
//!
//! Status is carried *structurally* on [`ToolStatus`], never re-parsed from a
//! magic string prefix. One [`ToolOutcome::render`] is the single place the
//! model-facing string is produced.

use crate::error::ToolError;

/// The reconciled 5-member tool status (data-model §7; ADR-0036).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// Tool ran and produced a usable result.
    Ok,
    /// Tool ran but failed.
    Error,
    /// Policy blocked the call before it ran.
    Blocked,
    /// Tool exceeded its time budget.
    Timeout,
    /// Result was produced but truncated to a cap.
    Truncated,
}

impl ToolStatus {
    /// snake_case wire token (ADR-0025).
    pub fn as_str(self) -> &'static str {
        match self {
            ToolStatus::Ok => "ok",
            ToolStatus::Error => "error",
            ToolStatus::Blocked => "blocked",
            ToolStatus::Timeout => "timeout",
            ToolStatus::Truncated => "truncated",
        }
    }
}

/// A structured tool result: status + payload. Replaces prefix-sniffing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    /// Authoritative status, read directly by the tracer/observe layer.
    pub status: ToolStatus,
    /// The tool's textual payload (result, error message, or block reason).
    pub payload: String,
}

impl ToolOutcome {
    /// A successful result.
    pub fn ok(payload: impl Into<String>) -> Self {
        Self { status: ToolStatus::Ok, payload: payload.into() }
    }

    /// A policy block (the tool body never ran).
    pub fn blocked(reason: impl Into<String>) -> Self {
        Self { status: ToolStatus::Blocked, payload: reason.into() }
    }

    /// An execution error.
    pub fn error(msg: impl Into<String>) -> Self {
        Self { status: ToolStatus::Error, payload: msg.into() }
    }

    /// The single model-facing renderer. The status is a structural prefix the
    /// model can read, but observe reads [`ToolOutcome::status`] directly — the
    /// string is never parsed back.
    pub fn render(&self) -> String {
        match self.status {
            ToolStatus::Ok => self.payload.clone(),
            _ => format!("[{}] {}", self.status.as_str(), self.payload),
        }
    }
}

impl From<ToolError> for ToolOutcome {
    fn from(e: ToolError) -> Self {
        match e {
            ToolError::Timeout => Self { status: ToolStatus::Timeout, payload: e.to_string() },
            ToolError::Truncated(s) => Self { status: ToolStatus::Truncated, payload: s },
            other => Self::error(other.to_string()),
        }
    }
}
