#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `observe` — the Observe phase: the structured tool-result contract, the
//! per-turn [`Episode`] evidence, secret redaction, and a [`Tracer`]. Depends
//! only on `config` (ARCHITECTURE §4-5).
//!
//! Phase 2 scope: `Episode`/`ToolEvent` evidence consumed by the `compose`
//! verifier, and redaction-by-default (ADR-0026). The M-HIR `InterventionLogger`
//! and `EntropyAuditor` are tracked separately within this phase.

mod entropy;
mod error;
mod h3;
mod intervention;
mod outcome;
pub mod redact;

pub use entropy::{EntropyAudit, EntropyAuditor, EntropyCategory, EntropyFinding, EntropyLog};
pub use error::ObserveError;
pub use h3::{AgentAttribution, H3Scratch, ReproductionLog, Requirement};
pub use intervention::{
    Avoidability, InterventionKind, InterventionLogger, InterventionRecord, MhirReport,
};
pub use outcome::{ToolOutcome, ToolStatus};

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One recorded tool dispatch (PRD 04). `args` are redacted before journaling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEvent {
    /// Tool name.
    pub name: String,
    /// The call arguments (redact before persisting — [`redact::redact_value`]).
    pub args: Value,
    /// Structured result + status (ADR-0022).
    pub outcome: ToolOutcome,
}

/// The complete per-turn evidence the verifier consumes (PRD 04). Phase-2
/// subset: tool events + whether the loop produced a final answer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Episode {
    /// Every tool call made during the turn, in order.
    pub tool_events: Vec<ToolEvent>,
    /// True when the kernel produced a final reply (not cut off at max steps).
    pub final_reached: bool,
}

/// Captures the [`Episode`] for one turn. Thread-safe so the dispatch path can
/// record from the tool bridge while the session holds a handle.
#[derive(Default)]
pub struct Tracer {
    episode: Mutex<Episode>,
}

impl Tracer {
    /// New tracer with an empty episode.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset for a new turn (retains nothing; tokens are out of Phase-2 scope).
    pub fn start_episode(&self) {
        let mut ep = self.episode.lock().unwrap_or_else(|p| p.into_inner());
        ep.tool_events.clear();
        ep.final_reached = false;
    }

    /// Record one tool dispatch. `args` are stored as given; redaction is applied
    /// at the journaling boundary, not here, so live attribution sees real values.
    pub fn record_tool(&self, name: &str, args: Value, outcome: &ToolOutcome) {
        let mut ep = self.episode.lock().unwrap_or_else(|p| p.into_inner());
        ep.tool_events.push(ToolEvent {
            name: name.to_string(),
            args,
            outcome: outcome.clone(),
        });
    }

    /// Mark whether the turn reached a final reply.
    pub fn set_final_reached(&self, reached: bool) {
        self.episode
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .final_reached = reached;
    }

    /// Snapshot the current episode.
    pub fn episode(&self) -> Episode {
        self.episode
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracer_collects_episode() {
        let t = Tracer::new();
        t.start_episode();
        t.record_tool(
            "read_file",
            serde_json::json!({"path": "a"}),
            &ToolOutcome::ok("data"),
        );
        t.record_tool(
            "write",
            serde_json::json!({}),
            &ToolOutcome::blocked("nope"),
        );
        t.set_final_reached(true);

        let ep = t.episode();
        assert_eq!(ep.tool_events.len(), 2);
        assert!(ep.final_reached);
        assert_eq!(ep.tool_events[0].outcome.status, ToolStatus::Ok);
        assert_eq!(ep.tool_events[1].outcome.status, ToolStatus::Blocked);
    }
}
