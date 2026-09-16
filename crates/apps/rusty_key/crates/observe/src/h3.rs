//! H3 turn scratch (PRD 05 / Phase 10). Shared, thread-safe state the H3 tools
//! (`reproduce`, `attribute_failure`, `verification_report`) write during a turn
//! and the `compose` assembler + H3 checks read when building the episode
//! package. Lives in `observe` because `feed` (the tools) and `compose` (the
//! checks/assembler) are independent siblings that both depend on `observe`.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// A reproduced failure, before the fix (`reproduce` tool / `reproduction_log`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReproductionLog {
    /// The check or probe used to reproduce.
    pub check: String,
    /// What was observed (the bug).
    pub observed: String,
    /// What should have happened.
    pub expected: String,
}

/// One requirement verdict in the verification report (`verification_report`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Requirement {
    /// Requirement id (links to `covers[]` in the verification trace).
    pub requirement: String,
    /// Whether it is met.
    pub met: bool,
    /// Evidence for the verdict.
    pub evidence: String,
}

/// An agent-declared attribution (`attribute_failure` tool). Distinct from the
/// verifier's matrix attribution; both are merged into the package's
/// `attribution_log`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentAttribution {
    /// Observed behaviour.
    pub observed: String,
    /// Expected behaviour.
    pub expected: String,
    /// Fixed `FailureType` token (snake_case, e.g. `f_tool`).
    pub failure_type: String,
    /// Evidence.
    pub evidence: String,
    /// The next action the agent will take.
    pub next_action: String,
}

/// Per-turn H3 scratch: reproduction, requirements, and agent attributions
/// accumulated by the H3 tools, drained into the episode package at assembly.
#[derive(Default)]
pub struct H3Scratch {
    reproduction: Mutex<Option<ReproductionLog>>,
    requirements: Mutex<Vec<Requirement>>,
    attributions: Mutex<Vec<AgentAttribution>>,
}

impl H3Scratch {
    /// Empty scratch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all scratch (called at the start of each turn).
    pub fn reset(&self) {
        *self.reproduction.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.requirements
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        self.attributions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    /// Record (or replace) the reproduction log.
    pub fn set_reproduction(&self, log: ReproductionLog) {
        *self.reproduction.lock().unwrap_or_else(|p| p.into_inner()) = Some(log);
    }

    /// Snapshot the reproduction log.
    pub fn reproduction(&self) -> Option<ReproductionLog> {
        self.reproduction
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Append the verification-report requirements.
    pub fn set_requirements(&self, reqs: Vec<Requirement>) {
        *self.requirements.lock().unwrap_or_else(|p| p.into_inner()) = reqs;
    }

    /// Snapshot the requirements.
    pub fn requirements(&self) -> Vec<Requirement> {
        self.requirements
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Whether a verification report was produced this turn.
    pub fn has_report(&self) -> bool {
        !self
            .requirements
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
    }

    /// Append an agent attribution.
    pub fn add_attribution(&self, attr: AgentAttribution) {
        self.attributions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(attr);
    }

    /// Snapshot the agent attributions.
    pub fn attributions(&self) -> Vec<AgentAttribution> {
        self.attributions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}
