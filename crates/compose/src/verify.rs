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

/// Limits once the criteria judge has run (PRD 05).
pub const SEMANTIC_LIMITS: &str =
    "LLM-judge on active task criteria included; output quality beyond stated goals not evaluated";

/// The evidentiary verdict for a turn (PRD 05). Phase-2/4 subset: the H3
/// episode package + entropy/outcome fields are added in later phases.
#[derive(Debug, Clone, Serialize)]
pub struct VerificationReport {
    /// True iff every check passed (incl. the criteria judge, when it ran).
    pub verified: bool,
    /// Each check's verdict.
    pub checks: Vec<CheckResult>,
    /// One or more attributions per failed check (empty when verified).
    pub attributions: Vec<Attribution>,
    /// Whether the criteria judge ran; `false` ⇒ `judge_unavailable` (it never
    /// reads as a silent pass, and it bars `AutonomousVerifiedSuccess`).
    pub judge_ran: bool,
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

    /// Fold a criteria-judge result into the report (PRD 05). Adds a
    /// `criteria_judge` check, the matching semantic attribution
    /// (`criteria_unmet`→`f_model` / `judge_unavailable`→`f_verify`), threads
    /// `judge_ran`, and gates `verified` on the judge passing *and* running — an
    /// unavailable judge is never read as verified.
    pub fn with_judge(mut self, jr: crate::judge::JudgeResult) -> Self {
        self.judge_ran = jr.judge_ran;
        self.limits = SEMANTIC_LIMITS;
        self.checks.push(CheckResult {
            name: "criteria_judge".to_string(),
            passed: jr.judge_ran && jr.passed,
            detail: jr.detail.clone(),
        });
        if !jr.judge_ran {
            self.attributions.push(Attribution {
                check: "criteria_judge".to_string(),
                failure_type: FailureType::FVerify,
                category: "judge_unavailable".to_string(),
                layer: "compose/semantic".to_string(),
                evidence: jr.detail,
            });
        } else if !jr.passed {
            self.attributions.push(Attribution {
                check: "criteria_judge".to_string(),
                failure_type: FailureType::FModel,
                category: "criteria_unmet".to_string(),
                layer: "compose/semantic".to_string(),
                evidence: jr.detail,
            });
        }
        self.verified = self.verified && jr.judge_ran && jr.passed;
        self
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

    /// Append the H3 process checks (`reproduce_before_edit`,
    /// `verification_report_required`) backed by this turn's scratch (PRD 05).
    pub fn with_h3(mut self, scratch: std::sync::Arc<rk_observe::H3Scratch>) -> Self {
        self.checks
            .push(Box::new(crate::check::ReproduceBeforeEdit::new(
                scratch.clone(),
            )));
        self.checks
            .push(Box::new(crate::check::VerificationReportRequired::new(
                scratch,
            )));
        self
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
            judge_ran: true, // no judge ran ⇒ not blocked by an unavailable one
            limits: self.limits,
        }
    }
}

/// Map a failed check to its `(category, layer, FailureType)` attribution(s)
/// per the frozen matrix (PRD 05). A new row is an explicit code change.
fn attribute(check: &str, episode: &Episode) -> Vec<Attribution> {
    match check {
        "no_tool_errors" => {
            // One attribution per distinct failing status (frozen matrix; the
            // timeout/truncated rows extend it for the chaos fault classes).
            let names = |status: ToolStatus| -> Vec<&str> {
                episode
                    .tool_events
                    .iter()
                    .filter(|e| e.outcome.status == status)
                    .map(|e| e.name.as_str())
                    .collect()
            };
            let mut out = Vec::new();
            for (status, category, layer) in [
                (ToolStatus::Blocked, "permission_block", "constrain/policy"),
                (ToolStatus::Error, "tool_error", "feed/tools"),
                (ToolStatus::Timeout, "tool_timeout", "feed/tools"),
                (ToolStatus::Truncated, "tool_truncated", "feed/tools"),
            ] {
                let hits = names(status);
                if !hits.is_empty() {
                    out.push(Attribution {
                        check: check.to_string(),
                        failure_type: FailureType::FTool,
                        category: category.to_string(),
                        layer: layer.to_string(),
                        evidence: format!("{}: {}", status.as_str(), hits.join(", ")),
                    });
                }
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
        "reproduce_before_edit" => vec![Attribution {
            check: check.to_string(),
            failure_type: FailureType::FVerify,
            category: "reproduction_skipped".to_string(),
            layer: "compose/h3".to_string(),
            evidence: "edited a file without first reproducing the failure".to_string(),
        }],
        "verification_report_required" => vec![Attribution {
            check: check.to_string(),
            failure_type: FailureType::FVerify,
            category: "report_missing".to_string(),
            layer: "compose/h3".to_string(),
            evidence: "no verification report produced this turn".to_string(),
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
