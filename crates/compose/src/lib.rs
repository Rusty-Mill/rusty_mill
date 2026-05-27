#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `compose` — the verify/journal/classify layer (ARCHITECTURE §4; PRD 05).
//!
//! Phase 2 scope: the deterministic verification tier — the [`Check`] trait,
//! [`NoToolErrors`]/[`CleanTermination`], the [`Verifier`] and its
//! [`VerificationReport`], and the fixed [`FailureType`] taxonomy with the frozen
//! attribution matrix. The criteria judge (Phase 4), entropy/outcome fields, and
//! the H3 episode package + evidence journal land in their phases.

mod check;
mod failure;
mod journal;
mod judge;
mod verify;

pub use check::{Check, CheckResult, CleanTermination, NoToolErrors};
pub use failure::{Attribution, FailureType};
pub use journal::EvidenceJournal;
pub use judge::{judge_prompt, parse_judge, JudgeResult};
pub use verify::{VerificationReport, Verifier, DETERMINISTIC_LIMITS, SEMANTIC_LIMITS};

/// `compose` error taxonomy (ADR-0023; error-handling §2), composing downhill.
#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    /// Filesystem failure (evidence journal).
    #[error("compose io error: {0}")]
    Io(#[from] std::io::Error),
    /// (De)serialization failure.
    #[error("compose serde error: {0}")]
    Serde(#[from] serde_json::Error),
    /// The criteria judge was unavailable (Phase 4); never read as verified.
    #[error("criteria judge unavailable")]
    JudgeUnavailable,
    /// A configuration error surfaced through compose.
    #[error(transparent)]
    Config(#[from] rk_config::ConfigError),
    /// An observe-layer error surfaced through compose.
    #[error(transparent)]
    Observe(#[from] rk_observe::ObserveError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rk_observe::{Episode, ToolEvent, ToolOutcome};
    use serde_json::json;

    fn ev(name: &str, outcome: ToolOutcome) -> ToolEvent {
        ToolEvent {
            name: name.to_string(),
            args: json!({}),
            outcome,
        }
    }

    #[test]
    fn clean_turn_is_verified() {
        let ep = Episode {
            tool_events: vec![ev("read_file", ToolOutcome::ok("data"))],
            final_reached: true,
        };
        let report = Verifier::deterministic().verify("done", &ep);
        assert!(report.verified);
        assert!(report.attributions.is_empty());
    }

    #[test]
    fn tool_error_marks_unverified_with_tool_error_attribution() {
        let ep = Episode {
            tool_events: vec![ev("bash", ToolOutcome::error("exit 1"))],
            final_reached: true,
        };
        let report = Verifier::deterministic().verify("done", &ep);
        assert!(!report.verified);
        let a = report
            .attributions
            .iter()
            .find(|a| a.category == "tool_error")
            .unwrap();
        assert_eq!(a.failure_type, FailureType::FTool);
        assert_eq!(a.layer, "feed/tools");
    }

    #[test]
    fn blocked_tool_attributes_permission_block() {
        let ep = Episode {
            tool_events: vec![ev("read_file", ToolOutcome::blocked("outside root"))],
            final_reached: true,
        };
        let report = Verifier::deterministic().verify("done", &ep);
        assert!(!report.verified);
        let a = report
            .attributions
            .iter()
            .find(|a| a.category == "permission_block")
            .unwrap();
        assert_eq!(a.failure_type, FailureType::FTool);
        assert_eq!(a.layer, "constrain/policy");
    }

    #[test]
    fn non_termination_attributes_recovery() {
        let ep = Episode {
            tool_events: vec![],
            final_reached: false,
        };
        let report = Verifier::deterministic().verify("", &ep);
        assert!(!report.verified);
        let a = report
            .attributions
            .iter()
            .find(|a| a.category == "non_termination")
            .unwrap();
        assert_eq!(a.failure_type, FailureType::FRecovery);
    }

    #[test]
    fn failure_type_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(FailureType::FVerify).unwrap(),
            json!("f_verify")
        );
    }

    #[test]
    fn observation_carries_verdict_and_limits() {
        let ep = Episode {
            tool_events: vec![ev("bash", ToolOutcome::error("x"))],
            final_reached: true,
        };
        let obs = Verifier::deterministic()
            .verify("done", &ep)
            .as_observation();
        assert!(obs.starts_with("UNVERIFIED [f_tool]"));
        assert!(obs.contains("deterministic checks only"));
    }
}
