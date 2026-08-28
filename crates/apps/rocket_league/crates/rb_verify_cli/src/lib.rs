//! `rb-verify`'s composition-root logic, factored out of `main.rs` so it's
//! unit-testable without spawning a process — see `RB-VERIFY-003`'s
//! Architecture and interfaces.

use rb_capture_ingest::CaptureFileSource;
use rb_domain::divergence::DivergenceScore;
use rb_domain::{IngestError, PhysicsStateSource};
use rb_replay_ingest::ReplayFileSource;
use std::path::Path;

/// Ingests a replay file as the recorded trajectory and a capture file as
/// the candidate trajectory, then scores their divergence.
///
/// This is a mechanical pipeline check, not a meaningful fidelity
/// comparison: a replay and a capture recorded from different matches have
/// no physical reason to resemble each other. What this proves is that
/// ingestion → `rb_domain::divergence::score` runs end-to-end across both
/// real adapters without erroring — `PHASE-0-EXIT`'s exit gate. A real
/// recorded-vs-candidate comparison needs a Phase 1 candidate physics
/// engine plus `RB-VERIFY-003-FR-002`/`FR-003` (car-state scoring,
/// timestamp-tolerant alignment), neither of which exist yet.
pub fn score_replay_against_capture(
    replay_path: impl AsRef<Path>,
    capture_path: impl AsRef<Path>,
) -> Result<DivergenceScore, IngestError> {
    let recorded = ReplayFileSource::new(replay_path.as_ref()).frames()?;
    let candidate = CaptureFileSource::new(capture_path.as_ref()).frames()?;
    Ok(rb_domain::divergence::score(&recorded, &candidate))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn replay_fixture() -> &'static str {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../rb_replay_ingest/fixtures/subtr-actor-sample.replay"
        )
    }

    fn capture_fixture() -> &'static str {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../rb_capture_ingest/fixtures/example.capture.jsonl"
        )
    }

    #[test]
    fn scores_a_real_replay_against_the_synthetic_capture_fixture() {
        let score = score_replay_against_capture(replay_fixture(), capture_fixture()).unwrap();
        assert!(score.frames_compared > 0);
    }

    #[test]
    fn missing_replay_file_reports_io_error() {
        let result = score_replay_against_capture("does-not-exist.replay", capture_fixture());
        assert!(matches!(result, Err(IngestError::Io(_))));
    }

    #[test]
    fn missing_capture_file_reports_io_error() {
        let result = score_replay_against_capture(replay_fixture(), "does-not-exist.capture.jsonl");
        assert!(matches!(result, Err(IngestError::Io(_))));
    }
}
