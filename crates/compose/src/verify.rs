//! The [`Verifier`] and its [`VerificationReport`]. Phase-2 scope is the
//! deterministic tier; the criteria judge (Phase 4) and H3 episode package
//! (Phase 10) extend the report later.

use rk_observe::{Episode, ToolStatus};
use serde::Serialize;

use crate::check::{Check, CheckResult, CleanTermination, NoToolErrors};
use crate::failure::{Attribution, FailureType};

/// What the deterministic tier did *not* verify — always surfaced so "verified"
/// is never over-read (ADR-0013).
pub const DETERMINISTIC_LIMITS: &str =
    "deterministic checks only; semantic correctness and task success not verified";

/// The evidentiary verdict for a turn (PRD 05). Phase-2 subset: the criteria
/// judge, entropy, and H3 outcome fields are added in later phases.
#[derive(Debug, Clone, Serialize)]
pub struct VerificationReport {
    /// True iff every check passed.
    pub verified: bool,
    /// Each check's verdict.
    pub checks: Vec<CheckResult>,
    /// One or more attributions per failed check (empty when verified).
    pub attributions: Vec<Attribution>,
    /// What was not verified.
    pub limits: &'static str,
}

impl VerificationReport {
    /// Human-readable multi-line output for `/verify`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(if self.verified {
            "VERIFIED\n"
        } else {
            "UNVERIFIED\n"
        });
        for c in &self.checks {
            out.push_str(&format!(
                "  [{}] {}: {}\n",
                if c.passed { "ok" } else { "fail" },
                c.name,
                c.detail
            ));
        }
        for a in &self.attributions {
            out.push_str(&format!(
                "  ! {} → {} ({}, {})\n",
                a.check,
                serde_json::to_value(a.failure_type)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default(),
                a.category,
                a.layer,
            ));
        }
        out.push_str(&format!("  limits: {}", self.limits));
        out
    }

    /// Compact one-line signal for the memory stream.
    pub fn as_observation(&self) -> String {
        let verdict = if self.verified {
            "VERIFIED"
        } else {
            "UNVERIFIED"
        };
        let types: Vec<String> = self
            .attributions
            .iter()
            .filter_map(|a| serde_json::to_value(a.failure_type).ok())
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        if types.is_empty() {
            format!("{verdict}; {}", self.limits)
        } else {
            format!("{verdict} [{}]; {}", types.join(","), self.limits)
        }
    }

    /// Serialized form for the evidence journal.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// Runs an ordered set of [`Check`]s and assembles a [`VerificationReport`],
/// attributing each failure via the frozen matrix.
pub struct Verifier {
    /// The checks to run, in order.
    pub checks: Vec<Box<dyn Check>>,
    /// What this verifier does not verify.
    pub limits: &'static str,
}

impl Verifier {
    /// The Phase-2 deterministic verifier: `no_tool_errors` + `clean_termination`.
    pub fn deterministic() -> Self {
        Self {
            checks: vec![Box::new(NoToolErrors), Box::new(CleanTermination)],
            limits: DETERMINISTIC_LIMITS,
        }
    }

    /// Run every check and build the report.
    pub fn verify(&self, reply: &str, episode: &Episode) -> VerificationReport {
        let checks: Vec<CheckResult> = self.checks.iter().map(|c| c.run(reply, episode)).collect();
        let verified = checks.iter().all(|c| c.passed);
        let attributions = checks
            .iter()
            .filter(|c| !c.passed)
            .flat_map(|c| attribute(&c.name, episode))
            .collect();
        VerificationReport {
            verified,
            checks,
            attributions,
            limits: self.limits,
        }
    }
}

/// Map a failed check to its `(category, layer, FailureType)` attribution(s)
/// per the frozen matrix (PRD 05). A new row is an explicit code change.
fn attribute(check: &str, episode: &Episode) -> Vec<Attribution> {
    match check {
        "no_tool_errors" => {
            let mut out = Vec::new();
            let blocked: Vec<&str> = episode
                .tool_events
                .iter()
                .filter(|e| e.outcome.status == ToolStatus::Blocked)
                .map(|e| e.name.as_str())
                .collect();
            let errored: Vec<&str> = episode
                .tool_events
                .iter()
                .filter(|e| e.outcome.status == ToolStatus::Error)
                .map(|e| e.name.as_str())
                .collect();
            if !blocked.is_empty() {
                out.push(Attribution {
                    check: check.to_string(),
                    failure_type: FailureType::FTool,
                    category: "permission_block".to_string(),
                    layer: "constrain/policy".to_string(),
                    evidence: format!("blocked: {}", blocked.join(", ")),
                });
            }
            if !errored.is_empty() {
                out.push(Attribution {
                    check: check.to_string(),
                    failure_type: FailureType::FTool,
                    category: "tool_error".to_string(),
                    layer: "feed/tools".to_string(),
                    evidence: format!("errored: {}", errored.join(", ")),
                });
            }
            out
        }
        "clean_termination" => vec![Attribution {
            check: check.to_string(),
            failure_type: FailureType::FRecovery,
            category: "non_termination".to_string(),
            layer: "kernel/loop".to_string(),
            evidence: "loop ended without a final reply".to_string(),
        }],
        other => vec![Attribution {
            check: other.to_string(),
            failure_type: FailureType::FUnknown,
            category: "unknown".to_string(),
            layer: "unknown".to_string(),
            evidence: "no matrix row".to_string(),
        }],
    }
}
