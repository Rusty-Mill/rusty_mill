//! H3 episode packages (PRD 05 / Phase 10; data-model §5). The paper's central
//! output artifact: a **versioned** record carrying **eight typed traces**, plus
//! entropy and a five-label outcome. The traces are *projected* from raw evidence
//! by the [`EpisodeAssembler`] (ADR-0036) — the single named builder between raw
//! evidence and the package; nothing else constructs one.

use rk_config::HarnessLevel;
use rk_observe::{
    redact, AgentAttribution, Episode, H3Scratch, InterventionRecord, ReproductionLog, Requirement,
    ToolStatus,
};
use serde::Serialize;

use crate::failure::FailureType;
use crate::verify::VerificationReport;

/// The five-label episode outcome (PRD 05; AI Harness Engineering paper).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeOutcome {
    /// All checks pass, report produced, judge ran, no non-benign interventions.
    AutonomousVerifiedSuccess,
    /// Verified, but interventions were recorded during the turn.
    AssistedVerifiedSuccess,
    /// Appears done but verification could not be confirmed (no report, or the
    /// judge was unavailable).
    UnverifiedSuccess,
    /// Required checks fail or no usable reply.
    Failed,
    /// Tests weakened, unrelated destructive edits, or task bypassed.
    UnsafeInvalid,
}

/// Class of a verification-trace check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyType {
    /// Mechanical/deterministic check.
    Deterministic,
    /// LLM-judged / semantic check.
    NonDeterministic,
}

/// A verification-trace check result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyResult {
    /// The check passed.
    Pass,
    /// The check failed.
    Fail,
    /// The check could not run (e.g. judge unavailable).
    Unavailable,
}

/// One externally-meaningful operation (`action_trace[]`). Distinct from a tool
/// call: only file/report/task/exec ops are actions (ADR-0036 / F11).
#[derive(Debug, Clone, Serialize)]
pub struct ActionEvent {
    /// `read_file | edit_file | run_tool | write_report | update_task_state | declare_complete`.
    pub op: String,
    /// The file/report/task acted on.
    pub target: String,
    /// When the action was taken (epoch seconds).
    pub ts: f64,
}

/// One `tool_trace[]` element (F13): structural status plus recovery metrics.
#[derive(Debug, Clone, Serialize)]
pub struct ToolTraceEntry {
    /// Tool name surfaced to the model.
    pub name: String,
    /// Structural status (5-variant; ADR-0022).
    pub status: ToolStatus,
    /// Process exit for shell-backed tools (else `null`).
    pub exit_code: Option<i32>,
    /// Did the call hit its deadline?
    pub timeout: bool,
    /// Did a later step succeed after this one failed/timed out? (projected here).
    pub recovered: bool,
    /// The (redacted) result text.
    pub result: String,
}

/// One `context_trace[]` element (F12).
#[derive(Debug, Clone, Serialize)]
pub struct ContextEntry {
    /// A recalled memory title, a read file, or a static artifact.
    pub artifact: String,
    /// `primary | supporting | unused`.
    pub contribution: String,
    /// Did this artifact change what the agent did? (v1 heuristic).
    pub influenced_decision: bool,
}

/// One `verification_trace[]` element (F14).
#[derive(Debug, Clone, Serialize)]
pub struct VerifyEntry {
    /// Deterministic vs non-deterministic.
    pub r#type: VerifyType,
    /// Controlled-vocabulary method.
    pub method: String,
    /// pass | fail | unavailable.
    pub result: VerifyResult,
    /// Requirement ids this check evidences.
    pub covers: Vec<String>,
    /// What the result means for the verdict.
    pub interpretation: String,
}

/// One `attribution_log[]` element (the package-shaped attribution).
#[derive(Debug, Clone, Serialize)]
pub struct PackageAttribution {
    /// Observed behaviour.
    pub observed: String,
    /// Expected behaviour.
    pub expected: String,
    /// Fixed taxonomy bucket.
    pub failure_type: FailureType,
    /// Frozen layer vocabulary.
    pub layer: String,
    /// Evidence.
    pub evidence: String,
    /// Considered alternatives.
    pub alternatives: String,
    /// The next action.
    pub next_action: String,
}

/// The `verification_report` block (requirements + limits).
#[derive(Debug, Clone, Serialize)]
pub struct ReportBlock {
    /// Per-requirement verdicts.
    pub requirements: Vec<Requirement>,
    /// What was not verified.
    pub limits: String,
}

/// The entropy audit (PRD 04 / Phase 11). Empty until the auditor lands.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EntropyAudit {
    /// Net entropy delta (negative = degradation).
    pub delta: i64,
    /// Findings (each carries a category + severity; populated in Phase 11).
    pub findings: Vec<serde_json::Value>,
}

/// The task baseline at episode start.
#[derive(Debug, Clone, Serialize)]
pub struct InitialState {
    /// The git commit, if resolvable.
    pub commit: String,
    /// The workspace path.
    pub workspace: String,
}

/// The versioned, eight-trace episode package (data-model §5).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct EpisodePackage {
    /// Schema version (ADR-0027).
    pub schema_version: u32,
    /// `ep_<task_id>` — groups all turns of one task (ADR-0018).
    pub episode_id: String,
    /// This turn's id.
    pub turn_id: String,
    /// The task id.
    pub task_id: String,
    /// Harness level (h3).
    pub harness_level: HarnessLevel,
    /// Epoch seconds.
    pub ts: f64,
    /// Task baseline.
    pub initial_state: InitialState,

    /// 1 — externally-meaningful ops (NOT a copy of tool_trace).
    pub action_trace: Vec<ActionEvent>,
    /// 2 — every tool call, structural status + recovery metrics.
    pub tool_trace: Vec<ToolTraceEntry>,
    /// 3 — recall/orient provenance.
    pub context_trace: Vec<ContextEntry>,
    /// 4 — how the turn was verified.
    pub verification_trace: Vec<VerifyEntry>,
    /// 5 — failure attributions (matrix + agent-declared).
    pub attribution_log: Vec<PackageAttribution>,
    /// 6 — reproduction before the fix, if any.
    pub reproduction_log: Option<ReproductionLog>,
    /// 7 — requirements + limits.
    pub verification_report: ReportBlock,
    /// 8 — this turn's intervention slice.
    pub intervention_log: Vec<InterventionRecord>,

    /// Entropy audit.
    pub entropy: EntropyAudit,
    /// The five-label outcome.
    pub outcome: EpisodeOutcome,
}

/// Classify the turn outcome under the five-label taxonomy (PRD 05). Rule-based:
/// the `judge_ran` gate and the entropy precedence are load-bearing.
pub fn classify_outcome(
    report: &VerificationReport,
    nonbenign_interventions: usize,
    has_report: bool,
    entropy_unsafe: bool,
) -> EpisodeOutcome {
    // UnsafeInvalid takes precedence over any success label.
    if entropy_unsafe {
        return EpisodeOutcome::UnsafeInvalid;
    }
    if report.verified {
        // `verified` already implies the judge ran and passed (with_judge gate).
        if !has_report {
            return EpisodeOutcome::UnverifiedSuccess;
        }
        if nonbenign_interventions > 0 {
            return EpisodeOutcome::AssistedVerifiedSuccess;
        }
        return EpisodeOutcome::AutonomousVerifiedSuccess;
    }
    // Not verified: an unavailable judge over otherwise-passing checks is
    // "appears done, could not confirm" — UnverifiedSuccess; anything else Failed.
    let deterministic_passed = report
        .checks
        .iter()
        .filter(|c| c.name != "criteria_judge")
        .all(|c| c.passed);
    if !report.judge_ran && deterministic_passed {
        EpisodeOutcome::UnverifiedSuccess
    } else {
        EpisodeOutcome::Failed
    }
}

/// Evaluator-side outcome (ADR-0035 R5): assign the `EpisodeOutcome` from the
/// **evaluator's own** deterministic checks, independent of any agent
/// self-report. Runs at every level H0–H3 (paper Table 5), so "the agent
/// produced evidence" is never conflated with "the evaluator verified behaviour".
/// An empty registry means there is no evaluator evidence ⇒ `UnverifiedSuccess`.
pub fn evaluator_outcome(results: &[crate::registry::CheckRunResult]) -> EpisodeOutcome {
    if results.is_empty() {
        return EpisodeOutcome::UnverifiedSuccess;
    }
    if results.iter().all(|r| r.passed) {
        EpisodeOutcome::AutonomousVerifiedSuccess
    } else {
        EpisodeOutcome::Failed
    }
}

/// Metadata the assembler needs that is not in the evidence (ids, baseline).
pub struct EpisodeMeta {
    /// The task id (stable across a task's turns; `episode_id = ep_<task_id>`).
    pub task_id: String,
    /// This turn's id.
    pub turn_id: String,
    /// Harness level.
    pub harness_level: HarnessLevel,
    /// Task baseline.
    pub initial_state: InitialState,
}

/// Projects raw evidence into the eight typed traces and emits an
/// [`EpisodePackage`] (ADR-0036). A pure projection — it reads, never re-runs.
pub struct EpisodeAssembler<'a> {
    /// Raw tool events (PRD 04 tracer).
    pub episode: &'a Episode,
    /// The verification report (checks, attributions, judge_ran, verified).
    pub report: &'a VerificationReport,
    /// Recall/orient provenance for `context_trace`.
    pub context: &'a [ContextEntry],
    /// This turn's intervention slice (already filtered, F18).
    pub interventions: &'a [InterventionRecord],
    /// H3 scratch (reproduction, requirements, agent attributions).
    pub scratch: &'a H3Scratch,
    /// Registered `checks.toml` results (carry method + covers for the trace).
    pub registered: &'a [crate::registry::CheckRunResult],
    /// Ids + baseline.
    pub meta: EpisodeMeta,
}

/// Map a tool name to an externally-meaningful action op, or `None` if the call
/// is not itself an action (ADR-0036 — action_trace ⊊ tool_trace).
fn action_op(tool: &str) -> Option<&'static str> {
    match tool {
        "read_file" | "list_directory" | "glob" | "grep" => Some("read_file"),
        "write_file" | "edit_file" => Some("edit_file"),
        "bash" => Some("run_tool"),
        "verification_report" => Some("write_report"),
        "set_task" => Some("update_task_state"),
        "complete_task" => Some("declare_complete"),
        _ => None,
    }
}

fn action_target(tool: &str, args: &serde_json::Value) -> String {
    let key = match tool {
        "bash" => "command",
        "glob" => "pattern",
        _ => "path",
    };
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(tool)
        .to_string()
}

impl EpisodeAssembler<'_> {
    /// Project the raw evidence into the package.
    pub fn assemble(&self) -> EpisodePackage {
        let ts = now_secs();

        let action_trace = self
            .episode
            .tool_events
            .iter()
            .filter_map(|e| {
                action_op(&e.name).map(|op| ActionEvent {
                    op: op.to_string(),
                    target: action_target(&e.name, &e.args),
                    ts,
                })
            })
            .collect();

        // A later successful step recovers an earlier failed/timed-out one.
        let any_ok_after = |idx: usize| {
            self.episode.tool_events[idx + 1..]
                .iter()
                .any(|e| e.outcome.status == ToolStatus::Ok)
        };
        let tool_trace = self
            .episode
            .tool_events
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let failed = matches!(e.outcome.status, ToolStatus::Error | ToolStatus::Timeout);
                ToolTraceEntry {
                    name: e.name.clone(),
                    status: e.outcome.status,
                    exit_code: None,
                    timeout: e.outcome.status == ToolStatus::Timeout,
                    recovered: failed && any_ok_after(i),
                    result: redact::redact_text(&e.outcome.payload),
                }
            })
            .collect();

        // verification_trace: the built-in checks (excluding the registered
        // summaries, which we project from the richer CheckRunResult) + each
        // registered result with its real method + requirement coverage.
        let mut verification_trace: Vec<VerifyEntry> = self
            .report
            .checks
            .iter()
            .filter(|c| !c.name.starts_with("registered:"))
            .map(verify_entry)
            .collect();
        verification_trace.extend(self.registered.iter().map(registered_verify_entry));
        let attribution_log = self.attribution_log();
        let verification_report = ReportBlock {
            requirements: self.scratch.requirements(),
            limits: self.report.limits.to_string(),
        };
        let entropy = EntropyAudit::default();
        let outcome = classify_outcome(
            self.report,
            self.interventions
                .iter()
                .filter(|r| r.avoidability != rk_observe::Avoidability::Benign)
                .count(),
            self.scratch.has_report(),
            false, // entropy auditor lands in Phase 11
        );

        EpisodePackage {
            schema_version: 1,
            episode_id: format!("ep_{}", self.meta.task_id),
            turn_id: self.meta.turn_id.clone(),
            task_id: self.meta.task_id.clone(),
            harness_level: self.meta.harness_level,
            ts,
            initial_state: self.meta.initial_state.clone(),
            action_trace,
            tool_trace,
            context_trace: self.context.to_vec(),
            verification_trace,
            attribution_log,
            reproduction_log: self.scratch.reproduction(),
            verification_report,
            intervention_log: self.interventions.to_vec(),
            entropy,
            outcome,
        }
    }

    /// Merge the verifier's matrix attributions with the agent-declared ones.
    fn attribution_log(&self) -> Vec<PackageAttribution> {
        let mut out: Vec<PackageAttribution> = self
            .report
            .attributions
            .iter()
            .map(|a| PackageAttribution {
                observed: a.evidence.clone(),
                expected: String::new(),
                failure_type: a.failure_type,
                layer: a.layer.clone(),
                evidence: a.evidence.clone(),
                alternatives: String::new(),
                next_action: String::new(),
            })
            .collect();
        for a in self.scratch.attributions() {
            out.push(agent_attribution(&a));
        }
        out
    }
}

fn agent_attribution(a: &AgentAttribution) -> PackageAttribution {
    let failure_type =
        serde_json::from_value::<FailureType>(serde_json::Value::String(a.failure_type.clone()))
            .unwrap_or(FailureType::FUnknown);
    PackageAttribution {
        observed: a.observed.clone(),
        expected: a.expected.clone(),
        failure_type,
        layer: "agent".to_string(),
        evidence: a.evidence.clone(),
        alternatives: String::new(),
        next_action: a.next_action.clone(),
    }
}

/// Map a `CheckResult` to a `VerifyEntry` (F14). The method vocabulary keys off
/// the check name; the criteria judge is the only non-deterministic source.
fn verify_entry(c: &crate::check::CheckResult) -> VerifyEntry {
    let (vtype, method) = match c.name.as_str() {
        "criteria_judge" => (VerifyType::NonDeterministic, "manual"),
        "reproduce_before_edit" => (VerifyType::Deterministic, "bug_reproduction"),
        "verification_report_required" => (VerifyType::Deterministic, "patch_review"),
        _ => (VerifyType::Deterministic, "deterministic_check"),
    };
    // A non-deterministic judge that did not pass *and* produced no detail is
    // treated as unavailable; otherwise pass/fail follows `passed`.
    let result = if c.name == "criteria_judge" && c.detail.contains("unavailable") {
        VerifyResult::Unavailable
    } else if c.passed {
        VerifyResult::Pass
    } else {
        VerifyResult::Fail
    };
    VerifyEntry {
        r#type: vtype,
        method: method.to_string(),
        result,
        covers: Vec::new(),
        interpretation: c.detail.clone(),
    }
}

/// Project a registered `checks.toml` result into a `verification_trace` entry,
/// carrying its declared method + requirement coverage (F14).
fn registered_verify_entry(r: &crate::registry::CheckRunResult) -> VerifyEntry {
    VerifyEntry {
        r#type: VerifyType::Deterministic,
        method: r.method.clone(),
        result: if r.passed {
            VerifyResult::Pass
        } else {
            VerifyResult::Fail
        },
        covers: r.covers.clone(),
        interpretation: if r.passed {
            format!("{} passed", r.check)
        } else {
            format!("{}: expected '{}'", r.check, r.expected)
        },
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::CheckResult;

    fn report(verified: bool, judge_ran: bool, checks: Vec<CheckResult>) -> VerificationReport {
        VerificationReport {
            verified,
            checks,
            attributions: Vec::new(),
            judge_ran,
            limits: "x",
        }
    }

    #[test]
    fn classifier_covers_all_five_labels() {
        let pass = || CheckResult {
            name: "no_tool_errors".into(),
            passed: true,
            detail: String::new(),
        };
        let fail = || CheckResult {
            name: "no_tool_errors".into(),
            passed: false,
            detail: String::new(),
        };

        // Autonomous: verified + report + no interventions.
        assert_eq!(
            classify_outcome(&report(true, true, vec![pass()]), 0, true, false),
            EpisodeOutcome::AutonomousVerifiedSuccess
        );
        // Assisted: verified + report + interventions.
        assert_eq!(
            classify_outcome(&report(true, true, vec![pass()]), 1, true, false),
            EpisodeOutcome::AssistedVerifiedSuccess
        );
        // Unverified: verified but no report produced.
        assert_eq!(
            classify_outcome(&report(true, true, vec![pass()]), 0, false, false),
            EpisodeOutcome::UnverifiedSuccess
        );
        // Unverified: judge unavailable over otherwise-passing checks.
        assert_eq!(
            classify_outcome(&report(false, false, vec![pass()]), 0, true, false),
            EpisodeOutcome::UnverifiedSuccess
        );
        // Failed: a deterministic check failed.
        assert_eq!(
            classify_outcome(&report(false, true, vec![fail()]), 0, true, false),
            EpisodeOutcome::Failed
        );
        // UnsafeInvalid takes precedence even over a verified turn.
        assert_eq!(
            classify_outcome(&report(true, true, vec![pass()]), 0, true, true),
            EpisodeOutcome::UnsafeInvalid
        );
    }
}
