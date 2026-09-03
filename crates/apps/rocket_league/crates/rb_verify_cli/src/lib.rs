//! `rb-verify`'s composition-root logic, factored out of `main.rs` so it's
//! unit-testable without spawning a process — see `RB-VERIFY-003`'s
//! Architecture and interfaces.

use rb_capture_ingest::CaptureFileSource;
use rb_domain::divergence::DivergenceScore;
use rb_domain::{IngestError, PhysicsFrame, PhysicsStateSource};
use rb_physics_bullet::body::CAR_HALF_EXTENTS;
use rb_physics_bullet::world::simulate_recorded;
use rb_physics_bullet::PhysicsWorld;
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

/// Margin (world units) added to `CAR_HALF_EXTENTS.z` when deciding whether
/// a recorded car frame is "grounded" — a car resting on flat ground sits
/// with its center roughly at that half-height, but a capture's own
/// sampling jitter and the ground's own collision margin mean an exact
/// equality check would almost always miss. Not derived from any real
/// source; a generous, deliberately-documented tolerance rather than a
/// calibrated constant.
const SEED_FRAME_GROUND_TOLERANCE: f32 = 10.0;

/// Threshold (uu/s) below which a car's vertical velocity counts as "not
/// airborne" for seed-frame selection. Rocket League cars bounce slightly
/// even at rest on the ground; this only needs to rule out a car that's
/// mid-jump or mid-fall, not demand exact zero.
const SEED_FRAME_VERTICAL_VELOCITY_TOLERANCE: f32 = 50.0;

/// `RB-PHYSICS-001-FR-076`'s `PhysicsWorld::from_frame` only seeds the
/// per-car state a `PhysicsFrame` actually carries — it can't set the
/// hidden jump/double-jump/dodge state `PhysicsWorld` tracks internally
/// (see `from_frame`'s own doc comment), so seeding from an arbitrary frame
/// is only accurate if that frame is already a neutral, grounded moment
/// where those hidden fields' fixed defaults (not held, double-jump
/// available, no dodge in progress) happen to be true.
///
/// This is a heuristic proxy for that, not a guarantee: every car in the
/// frame must be at rest on the ground (position and vertical velocity
/// within the tolerances above) with no jump, boost, or handbrake held. It
/// cannot detect "mid-dodge but not holding jump" or "just landed from a
/// wall jump" — those still slip through if they happen to look grounded
/// and neutral by this check alone. A frame with no cars at all never
/// qualifies (there would be nothing to seed).
fn is_grounded_and_neutral(frame: &PhysicsFrame) -> bool {
    if frame.cars.is_empty() {
        return false;
    }
    frame.cars.iter().all(|car| {
        let Some(input) = car.input else {
            return false;
        };
        let grounded = car.position.z <= CAR_HALF_EXTENTS.z + SEED_FRAME_GROUND_TOLERANCE
            && car.velocity.z.abs() <= SEED_FRAME_VERTICAL_VELOCITY_TOLERANCE;
        grounded && !input.jump && !input.boost && !input.handbrake
    })
}

/// Ingests a capture file as both the recorded ground truth *and* the
/// source of a candidate trajectory: seeds an `rb_physics_bullet`
/// `PhysicsWorld` from the first grounded, neutral frame the capture
/// contains (`is_grounded_and_neutral`), simulates it forward using that
/// same capture's own recorded per-tick controller input
/// (`rb_physics_bullet::world::simulate_recorded`), then scores the
/// resulting candidate trajectory against the capture's own recorded
/// outcome from that seed frame onward.
///
/// Unlike [`score_replay_against_capture`], this comparison has a genuine
/// physical reason to be small if the physics core is accurate: the
/// candidate was actually simulated from the same starting state and the
/// same input the real capture recorded, not sourced from an unrelated
/// match (`RB-PHYSICS-001-FR-077`).
///
/// Returns `IngestError::Malformed` if the capture contains no frame
/// `is_grounded_and_neutral` accepts — there would be no valid seed to
/// simulate from.
pub fn score_capture_against_candidate(
    capture_path: impl AsRef<Path>,
    max_timestamp_delta_secs: f32,
) -> Result<DivergenceScore, IngestError> {
    let captured = CaptureFileSource::new(capture_path.as_ref()).frames()?;

    let seed_index = captured
        .iter()
        .position(is_grounded_and_neutral)
        .ok_or_else(|| {
            IngestError::Malformed(
                "no grounded, neutral frame found to seed a candidate simulation from".to_string(),
            )
        })?;

    let recorded = &captured[seed_index..];
    let world = PhysicsWorld::from_frame(&recorded[0]);
    let candidate = simulate_recorded(world, recorded);

    Ok(rb_domain::divergence::score(
        recorded,
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

    #[test]
    fn scores_a_real_capture_against_a_candidate_simulated_from_its_own_input() {
        let score =
            score_capture_against_candidate(capture_fixture(), DEFAULT_MAX_TIMESTAMP_DELTA_SECS)
                .unwrap();
        assert!(score.frames_compared > 0);
        assert!(score.cars.pairs_compared > 0);
    }

    #[test]
    fn capture_against_candidate_missing_file_reports_io_error() {
        let result = score_capture_against_candidate(
            "does-not-exist.capture.jsonl",
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        );
        assert!(matches!(result, Err(IngestError::Io(_))));
    }

    #[test]
    fn capture_with_no_grounded_neutral_frame_reports_malformed() {
        // Every frame holds jump the whole time, so no frame ever satisfies
        // `is_grounded_and_neutral` (which also requires !input.jump).
        let line = r#"{"timestamp_secs":0.0,"ball":{"position":{"x":0.0,"y":0.0,"z":93.0},"rotation":{"x":0.0,"y":0.0,"z":0.0,"w":1.0},"velocity":{"x":0.0,"y":0.0,"z":0.0},"angular_velocity":{"x":0.0,"y":0.0,"z":0.0}},"cars":[{"player_id":0,"position":{"x":0.0,"y":0.0,"z":17.0},"rotation":{"x":0.0,"y":0.0,"z":0.0,"w":1.0},"velocity":{"x":0.0,"y":0.0,"z":0.0},"angular_velocity":{"x":0.0,"y":0.0,"z":0.0},"boost_amount":33.0,"input":{"throttle":0.0,"steer":0.0,"pitch":0.0,"yaw":0.0,"roll":0.0,"jump":true,"boost":false,"handbrake":false}}]}"#;
        let path = std::env::temp_dir().join("rb_verify_cli_test_no_grounded_neutral_frame.jsonl");
        std::fs::write(&path, format!("{line}\n{line}\n")).unwrap();

        let result = score_capture_against_candidate(&path, DEFAULT_MAX_TIMESTAMP_DELTA_SECS);

        std::fs::remove_file(&path).ok();
        assert!(matches!(result, Err(IngestError::Malformed(_))));
    }
}
