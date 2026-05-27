//! The structured tool-result contract (ADR-0022). Status is carried
//! *structurally* on [`ToolStatus`], never re-parsed from a string prefix.

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
///
/// Constructors are deliberately status-specific; the mapping from a tool's own
/// error enum lives in the crate that owns the tools (`feed`), keeping `observe`
/// a leaf above `config`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    /// Authoritative status, read directly by observers.
    pub status: ToolStatus,
    /// Textual payload (result, error message, or block reason).
    pub payload: String,
}

impl ToolOutcome {
    /// Construct with an explicit status.
    pub fn new(status: ToolStatus, payload: impl Into<String>) -> Self {
        Self { status, payload: payload.into() }
    }

    /// A successful result.
    pub fn ok(payload: impl Into<String>) -> Self {
        Self::new(ToolStatus::Ok, payload)
    }

    /// A policy block (the tool body never ran).
    pub fn blocked(reason: impl Into<String>) -> Self {
        Self::new(ToolStatus::Blocked, reason)
    }

    /// An execution error.
    pub fn error(msg: impl Into<String>) -> Self {
        Self::new(ToolStatus::Error, msg)
    }

    /// The single model-facing renderer. Non-`Ok` statuses get a structural
    /// prefix the model can read; observers read [`ToolOutcome::status`]
    /// directly, so the string is never parsed back.
    pub fn render(&self) -> String {
        match self.status {
            ToolStatus::Ok => self.payload.clone(),
            _ => format!("[{}] {}", self.status.as_str(), self.payload),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_carries_status_structurally() {
        assert_eq!(ToolOutcome::ok("p").render(), "p");
        assert_eq!(ToolOutcome::blocked("no").render(), "[blocked] no");
        assert_eq!(ToolOutcome::error("boom").render(), "[error] boom");
    }
}
