//! Fault-injection fixture for the chaos / resilience eval tier (eval-plan §7).
//!
//! A [`FaultyTool`] is a [`ToolFn`] that deterministically returns a faulted
//! [`ToolOutcome`] — the injection chokepoint is the tool-dispatch / `ToolOutcome`
//! seam (ADR-0022), keyed off the fixture so runs are replayable in CI. The
//! resilience property the tier asserts: the harness degrades honestly and never
//! reports verified-success on top of an injected fault.

use async_trait::async_trait;
use rk_observe::{ToolOutcome, ToolStatus};
use serde_json::Value;

use crate::tool::ToolFn;

/// A deterministic fault class injected at the tool seam (eval-plan §7).
#[derive(Debug, Clone, Copy)]
pub enum Fault {
    /// Corrupt result: the tool reports `error`.
    Error,
    /// Latency/timeout: the tool reports `timeout`.
    Timeout,
    /// Mangled/truncated payload: the tool reports `truncated`.
    Truncated,
}

/// A tool whose every call returns the configured [`Fault`].
pub struct FaultyTool {
    name: String,
    fault: Fault,
}

impl FaultyTool {
    /// A tool named `name` that always injects `fault`.
    pub fn new(name: impl Into<String>, fault: Fault) -> Self {
        Self {
            name: name.into(),
            fault,
        }
    }
}

#[async_trait]
impl ToolFn for FaultyTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn schema(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    async fn call(&self, _args: Value) -> ToolOutcome {
        match self.fault {
            Fault::Error => ToolOutcome::error("injected fault: error"),
            Fault::Timeout => ToolOutcome::new(ToolStatus::Timeout, "injected fault: timeout"),
            Fault::Truncated => {
                ToolOutcome::new(ToolStatus::Truncated, "injected fault: truncated")
            }
        }
    }
}
