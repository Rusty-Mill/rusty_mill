#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `observe` — the Observe phase: structured tool-result contract and a minimal
//! tracer. Depends only on `config` (ARCHITECTURE §4-5).
//!
//! Phase 1 scope: [`ToolStatus`]/[`ToolOutcome`] and a [`Tracer`] that records
//! [`ToolEvent`]s. The evidence journal, M-HIR, and entropy auditor land later.

mod error;
mod outcome;

pub use error::ObserveError;
pub use outcome::{ToolOutcome, ToolStatus};

use std::sync::Mutex;

/// One observed tool dispatch.
#[derive(Debug, Clone)]
pub struct ToolEvent {
    /// Tool name.
    pub name: String,
    /// Resulting status (read structurally, not parsed).
    pub status: ToolStatus,
}

/// Collects [`ToolEvent`]s for a turn. Phase-1 minimal: in-memory, thread-safe.
#[derive(Default)]
pub struct Tracer {
    events: Mutex<Vec<ToolEvent>>,
}

impl Tracer {
    /// New empty tracer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the outcome of a tool dispatch.
    pub fn tool(&self, name: &str, outcome: &ToolOutcome) {
        self.events
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(ToolEvent {
                name: name.to_string(),
                status: outcome.status,
            });
    }

    /// Snapshot the recorded events.
    pub fn events(&self) -> Vec<ToolEvent> {
        self.events
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracer_records_status_structurally() {
        let t = Tracer::new();
        t.tool("read_file", &ToolOutcome::ok("data"));
        t.tool("read_file", &ToolOutcome::blocked("nope"));
        let ev = t.events();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].status, ToolStatus::Ok);
        assert_eq!(ev[1].status, ToolStatus::Blocked);
    }
}
