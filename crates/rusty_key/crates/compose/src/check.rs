//! Deterministic verification checks (PRD 05). Synchronous, no I/O — they read
//! the [`Episode`] evidence and the final reply, and return a verdict.

use std::sync::Arc;

use rk_observe::{Episode, H3Scratch, ToolStatus};
use serde::{Deserialize, Serialize};

/// One check's verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    /// Stable snake_case check name (e.g. `no_tool_errors`).
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Human-readable detail (what failed, or a brief pass note).
    pub detail: String,
}

/// A deterministic check over the reply and the episode evidence.
pub trait Check: Send + Sync {
    /// Stable snake_case name.
    fn name(&self) -> &str;
    /// Inspect the reply + evidence and return a verdict.
    fn run(&self, reply: &str, episode: &Episode) -> CheckResult;
}

/// Fails if any tool event ended in a non-`ok` status (read structurally from
/// `ToolOutcome`, ADR-0022 — never sniffed from the result string). Covers
/// `error`/`blocked` plus the `timeout`/`truncated` fault classes (eval-plan §7:
/// the resilience invariant requires this check to fire under every injected
/// fault), so the harness can never report verified-success on top of one.
pub struct NoToolErrors;

impl Check for NoToolErrors {
    fn name(&self) -> &str {
        "no_tool_errors"
    }

    fn run(&self, _reply: &str, episode: &Episode) -> CheckResult {
        let bad: Vec<&str> = episode
            .tool_events
            .iter()
            .filter(|e| e.outcome.status != ToolStatus::Ok)
            .map(|e| e.name.as_str())
            .collect();
        CheckResult {
            name: self.name().to_string(),
            passed: bad.is_empty(),
            detail: if bad.is_empty() {
                "no tool errors".to_string()
            } else {
                format!("failed/blocked tools: {}", bad.join(", "))
            },
        }
    }
}

/// Fails if the loop did not produce a final answer (hit `max_steps`).
pub struct CleanTermination;

impl Check for CleanTermination {
    fn name(&self) -> &str {
        "clean_termination"
    }

    fn run(&self, _reply: &str, episode: &Episode) -> CheckResult {
        CheckResult {
            name: self.name().to_string(),
            passed: episode.final_reached,
            detail: if episode.final_reached {
                "reached a final reply".to_string()
            } else {
                "loop ended without a final reply".to_string()
            },
        }
    }
}

/// H3: fails if the turn edited a file without first reproducing the failure
/// (PRD 05). Reads the per-turn [`H3Scratch`] set by the `reproduce` tool.
pub struct ReproduceBeforeEdit {
    scratch: Arc<H3Scratch>,
}

impl ReproduceBeforeEdit {
    /// Build the check over the turn's scratch.
    pub fn new(scratch: Arc<H3Scratch>) -> Self {
        Self { scratch }
    }
}

impl Check for ReproduceBeforeEdit {
    fn name(&self) -> &str {
        "reproduce_before_edit"
    }

    fn run(&self, _reply: &str, episode: &Episode) -> CheckResult {
        let edited = episode
            .tool_events
            .iter()
            .any(|e| matches!(e.name.as_str(), "write_file" | "edit_file"));
        let reproduced = self.scratch.reproduction().is_some();
        let passed = !edited || reproduced;
        CheckResult {
            name: self.name().to_string(),
            passed,
            detail: if passed {
                "no edit, or a reproduction preceded it".to_string()
            } else {
                "edited a file without first reproducing the failure".to_string()
            },
        }
    }
}

/// H3: fails if the turn produced no verification report (PRD 05). Reads the
/// per-turn [`H3Scratch`] populated by the `verification_report` tool.
pub struct VerificationReportRequired {
    scratch: Arc<H3Scratch>,
}

impl VerificationReportRequired {
    /// Build the check over the turn's scratch.
    pub fn new(scratch: Arc<H3Scratch>) -> Self {
        Self { scratch }
    }
}

impl Check for VerificationReportRequired {
    fn name(&self) -> &str {
        "verification_report_required"
    }

    fn run(&self, _reply: &str, _episode: &Episode) -> CheckResult {
        let has_report = self.scratch.has_report();
        CheckResult {
            name: self.name().to_string(),
            passed: has_report,
            detail: if has_report {
                "verification report produced".to_string()
            } else {
                "no verification report produced this turn".to_string()
            },
        }
    }
}
