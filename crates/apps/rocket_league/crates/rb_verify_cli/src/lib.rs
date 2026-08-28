//! `rb-verify`'s composition-root logic, factored out of `main.rs` so it's
//! unit-testable without spawning a process — see `RB-VERIFY-003`'s
//! Architecture and interfaces.

use rb_capture_ingest::CaptureFileSource;
use rb_domain::divergence::DivergenceScore;
use rb_domain::{IngestError, PhysicsStateSource};
use rb_replay_ingest::ReplayFileSource;
use std::path::Path;

/// Default timestamp tolerance (seconds) `rb-verify` uses when the caller
/// doesn't supply one explicitly (see `main.rs`'s optional third
/// argument). Chosen relative to `rb_replay_ingest`'s vendored fixture's
/// own average sampling interval (~0.036s, from 12,029 frames over
/// ~428s): permissive enough to bridge a typical replay/capture tick-rate
/// mismatch without matching frames that are meaningfully different
/// instants. Not empirically tuned against real divergence data — no
/// candidate physics engine exists yet to generate any — revisit once
/// `RB-VERIFY-003`'s "what threshold is good enough" open question is
/// answered.
pub const DEFAULT_MAX_TIMESTAMP_DELTA_SECS: f32 = 0.02;

/// Ingests a replay file as the recorded trajectory and a capture file as
/// the candidate trajectory, then scores their divergence — aligning
/// frames by nearest timestamp within `max_timestamp_delta_secs` rather
/// than by list index (`RB-VERIFY-003-FR-003`).
///
/// This is a mechanical pipeline check, not a meaningful fidelity
/// comparison: a replay and a capture recorded from different matches have
/// no physical reason to resemble each other. What this proves is that
/// ingestion → `rb_domain::divergence::score` runs end-to-end across both
/// real adapters without erroring — `PHASE-0-EXIT`'s exit gate. A real
/// recorded-vs-candidate comparison still needs a Phase 1 candidate
/// physics engine, which doesn't exist yet.
pub fn score_replay_against_capture(
    replay_path: impl AsRef<Path>,
    capture_path: impl AsRef<Path>,
    max_timestamp_delta_secs: f32,
) -> Result<DivergenceScore, IngestError> {
    let recorded = ReplayFileSource::new(replay_path.as_ref()).frames()?;
    let candidate = CaptureFileSource::new(capture_path.as_ref()).frames()?;
    Ok(rb_domain::divergence::score(
        &recorded,
        &candidate,
        max_timestamp_delta_secs,
    ))
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
        let score = score_replay_against_capture(
            replay_fixture(),
            capture_fixture(),
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        )
        .unwrap();
        assert!(score.frames_compared > 0);
    }

    #[test]
    fn missing_replay_file_reports_io_error() {
        let result = score_replay_against_capture(
            "does-not-exist.replay",
            capture_fixture(),
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        );
        assert!(matches!(result, Err(IngestError::Io(_))));
    }

    #[test]
    fn missing_capture_file_reports_io_error() {
        let result = score_replay_against_capture(
            replay_fixture(),
            "does-not-exist.capture.jsonl",
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        );
        assert!(matches!(result, Err(IngestError::Io(_))));
    }
}
