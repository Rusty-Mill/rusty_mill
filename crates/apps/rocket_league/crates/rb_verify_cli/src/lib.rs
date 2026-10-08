//! `rb-verify`'s composition-root logic, factored out of `main.rs` so it's
//! unit-testable without spawning a process — see `RB-VERIFY-003`'s
//! Architecture and interfaces.

use rb_capture_ingest::CaptureFileSource;
use rb_domain::divergence::DivergenceScore;
use rb_domain::{
    BallState, CarState, ControllerInput, IngestError, PhysicsFrame, PhysicsStateSource, Quat, Vec3,
};
use rb_physics_bullet::body::CAR_HALF_EXTENTS;
use rb_physics_bullet::drive::WheelContacts;
use rb_physics_bullet::world::{
    simulate_recorded, simulate_recorded_k_step, simulate_recorded_one_step, wheel_contacts_along,
};
use rb_physics_bullet::PhysicsWorld;
use rb_replay_ingest::ReplayFileSource;
use std::path::Path;

mod scenario;
mod sweep;
pub use scenario::{
    compare_scenario, compare_scenario_recorded, scenario_from_capture, simulate_scenario,
    ScenarioComparison, ScenarioRow, SCENARIO_TICK_SECS,
};
pub use sweep::{default_grid, sweep, window_error, Candidate, Window, WindowError};

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

/// Which recorded frame a capture's candidate simulation starts from
/// (`RB-VERIFY-003-FR-013`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SeedFrame {
    /// The first grounded, neutral frame (`is_grounded_and_neutral`), where
    /// the hidden jump state `PhysicsWorld::from_frame` cannot read is known
    /// to be at its defaults. The default.
    #[default]
    FirstGroundedNeutral,
    /// The capture's first frame, whatever its state: for a recording whose
    /// start was set by the tape bot (often airborne) and so has no
    /// grounded, neutral frame to wait for. `rb-verify --seed-first-frame`.
    First,
}

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
    seed_frame: SeedFrame,
    max_timestamp_delta_secs: f32,
) -> Result<DivergenceScore, IngestError> {
    let (recorded, candidate) = seed_and_simulate(capture_path, seed_frame)?;
    Ok(rb_domain::divergence::score(
        &recorded,
        &candidate,
        max_timestamp_delta_secs,
    ))
}

/// Default window width (seconds) `rb-verify --self-growth` uses when the
/// caller doesn't supply one explicitly. Chosen so the one real capture run
/// recorded so far (~23 seconds, 2,818 frames) prints as roughly 23 rows —
/// small enough to read on one screen, fine enough to localize an abrupt
/// derailment to roughly which second it started. Not empirically tuned;
/// revisit if a real run makes it too coarse or too noisy to read.
pub const DEFAULT_GROWTH_WINDOW_SECS: f32 = 1.0;

/// A divergence-growth diagnostic (`RB-VERIFY-003-FR-004`): the same
/// seed-frame selection and `simulate_recorded` call
/// [`score_capture_against_candidate`] performs, but scored with
/// [`rb_domain::divergence::score_windows`] instead of the whole-run
/// [`rb_domain::divergence::score`] — reporting how divergence changes
/// *within* the run instead of collapsing it to one mean/max pair. See
/// `score_windows`'s own doc comment for the windowing rule.
pub fn score_capture_growth(
    capture_path: impl AsRef<Path>,
    seed_frame: SeedFrame,
    max_timestamp_delta_secs: f32,
    window_secs: f32,
) -> Result<Vec<(f32, DivergenceScore)>, IngestError> {
    let (recorded, candidate) = seed_and_simulate(capture_path, seed_frame)?;
    Ok(rb_domain::divergence::score_windows(
        &recorded,
        &candidate,
        max_timestamp_delta_secs,
        window_secs,
    ))
}

/// One car at one recorded instant of a `rb-verify --self-trace` run
/// (`RB-VERIFY-003-FR-005`): the input the capture recorded for that car,
/// and the car's recorded and simulated state at that same instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceRow {
    /// Seconds since the seed frame — the same time axis
    /// `rb-verify --self-growth` prints, so a window it flags can be traced
    /// directly.
    pub t_secs: f32,
    /// The input the capture recorded for this car at this frame, which
    /// `simulate_recorded` applies for the step that follows it.
    pub input: Option<ControllerInput>,
    pub recorded: CarState,
    pub candidate: CarState,
    /// The ball in the same recorded and candidate frames
    /// (`RB-VERIFY-003-FR-007`), so a car-ball hit's effect on the ball is
    /// visible next to the car's.
    pub recorded_ball: BallState,
    pub candidate_ball: BallState,
}

impl TraceRow {
    /// Distance (uu) between recorded and simulated car positions.
    pub fn position_error(&self) -> f32 {
        self.recorded.position.distance(&self.candidate.position)
    }

    /// Distance (uu/s) between recorded and simulated car velocities.
    pub fn velocity_error(&self) -> f32 {
        self.recorded.velocity.distance(&self.candidate.velocity)
    }

    /// Distance (uu/s) between recorded and simulated ball velocities.
    pub fn ball_velocity_error(&self) -> f32 {
        self.recorded_ball
            .velocity
            .distance(&self.candidate_ball.velocity)
    }

    /// Angle (rad) between recorded and simulated car orientations.
    pub fn rotation_error(&self) -> f32 {
        self.recorded.rotation.angle_to(&self.candidate.rotation)
    }

    /// Distance (rad/s) between recorded and simulated angular velocities.
    pub fn spin_error(&self) -> f32 {
        self.recorded
            .angular_velocity
            .distance(&self.candidate.angular_velocity)
    }
}

/// `spin` (world frame) expressed in the car's own frame for orientation
/// `rotation`: `(forward, side, up)` components, i.e. roll, pitch and yaw
/// rates. Lets `--self-trace` tell which car axis a spin change is about,
/// whatever the car's orientation (`RB-VERIFY-003` 0.14.0).
pub fn car_frame_spin(rotation: &Quat, spin: &Vec3) -> Vec3 {
    rotation.conjugate().rotate(spin)
}

/// World-frame angular velocity (rad/s) that turns `from` into `to` over
/// `dt`: the rotation `to * from^-1` as axis times angle, divided by `dt`
/// (the same left-multiplied, world-frame convention `integrate_transform`
/// uses). `Vec3::ZERO` for no rotation or a non-positive `dt`. Lets
/// `--self-trace` check a capture's recorded orientations against its own
/// recorded angular velocity.
pub fn rotation_rate(from: &Quat, to: &Quat, dt: f32) -> Vec3 {
    let mut delta = to.mul(&from.conjugate());
    if delta.w < 0.0 {
        delta = Quat::new(-delta.x, -delta.y, -delta.z, -delta.w);
    }
    let axis = Vec3::new(delta.x, delta.y, delta.z);
    let sin_half = axis.length();
    if dt <= 0.0 || sin_half < 1e-9 {
        return Vec3::ZERO;
    }
    let angle = 2.0 * sin_half.atan2(delta.w);
    axis * (angle / (sin_half * dt))
}

/// A per-frame trace of a capture against the candidate simulated from it
/// (`RB-VERIFY-003-FR-005`): the same seed frame and `simulate_recorded`
/// run [`score_capture_growth`] uses, but returning every car at every
/// frame whose time since the seed falls in `[from_secs, to_secs]`
/// instead of a score. Meant for reading exactly which input preceded the
/// frame where a run `--self-growth` flagged starts to derail.
///
/// Recorded and candidate frames are paired by index, which is exact here:
/// `simulate_recorded` returns one candidate frame per recorded frame,
/// stepped by the recorded timestamps' own deltas. Cars are paired by
/// `player_id`; a recorded car with no simulated counterpart is skipped.
/// An empty or inverted window yields an empty trace, not an error.
pub fn trace_capture(
    capture_path: impl AsRef<Path>,
    seed_frame: SeedFrame,
    from_secs: f32,
    to_secs: f32,
) -> Result<Vec<TraceRow>, IngestError> {
    let (recorded, candidate) = seed_and_simulate(capture_path, seed_frame)?;
    Ok(trace_rows(&recorded, &candidate, from_secs, to_secs))
}

/// Every car at every frame of a capture against the candidate's one-step
/// prediction of that frame (`RB-VERIFY-003-FR-006`,
/// `simulate_recorded_one_step`): each candidate frame is stepped once
/// from the recorded frame before it, so a row's error is that one step's
/// model error alone, not divergence carried from earlier frames. Same
/// seed frame and time axis as [`trace_capture`].
pub fn one_step_capture(
    capture_path: impl AsRef<Path>,
    seed_frame: SeedFrame,
) -> Result<Vec<TraceRow>, IngestError> {
    let (recorded, world) = seed(capture_path, seed_frame)?;
    let candidate = simulate_recorded_one_step(world, &recorded);
    Ok(trace_rows(&recorded, &candidate, 0.0, f32::INFINITY))
}

/// One tick of a wall-and-ramp instrument row (`rb-verify --wheel-trace`, PARITY-PLAN
/// workstream A): the one-step velocity error into this tick and what the four
/// wheel rays hit in the state the step started from.
#[derive(Debug, Clone, Copy)]
pub struct WheelTraceRow {
    /// Time of the tick's end (the frame whose velocity is predicted).
    pub t_secs: f32,
    /// The input applied over the tick.
    pub input: Option<ControllerInput>,
    /// Recorded minus predicted velocity at the tick's end (uu/s).
    pub velocity_error: Vec3,
    /// Recorded minus predicted spin at the tick's end (rad/s).
    pub spin_error: Vec3,
    /// The first car's wheel hits at the start of the tick, in wheel order.
    pub wheels: WheelContacts,
}

/// The first car's one-step error at each tick of a capture (frames from the first on,
/// the tape bot's captures start at the state set) beside its wheel-ray hits, for
/// lining a velocity error up with the facet a wheel crossed.
pub fn wheel_trace_capture(
    capture_path: impl AsRef<Path>,
    from_secs: f32,
    to_secs: f32,
) -> Result<Vec<WheelTraceRow>, IngestError> {
    let recorded = CaptureFileSource::new(capture_path.as_ref()).frames()?;
    if recorded.first().is_none_or(|frame| frame.cars.is_empty()) {
        return Err(IngestError::Malformed(
            "the capture's first frame has no car to trace".to_string(),
        ));
    }
    let world = PhysicsWorld::from_frame(&recorded[0]);
    let candidate = simulate_recorded_one_step(world.clone(), &recorded);
    let hits = wheel_contacts_along(world, &recorded);
    let mut rows = Vec::new();
    for (index, wheels) in hits.iter().enumerate() {
        let (prev, next) = (&recorded[index], &recorded[index + 1]);
        let (Some(rec), Some(pred)) = (next.cars.first(), candidate[index + 1].cars.first()) else {
            continue;
        };
        if next.timestamp_secs < from_secs || next.timestamp_secs > to_secs {
            continue;
        }
        rows.push(WheelTraceRow {
            t_secs: next.timestamp_secs,
            input: prev.cars.first().and_then(|car| car.input),
            velocity_error: rec.velocity - pred.velocity,
            spin_error: rec.angular_velocity - pred.angular_velocity,
            wheels: *wheels,
        });
    }
    Ok(rows)
}

/// Default horizon (ticks) for `rb-verify --self-kstep`: a quarter second
/// at 120 Hz, long enough for position-level effects (contact correction,
/// suspension) to show, short enough that the chaos of a free run doesn't.
pub const DEFAULT_K_STEP: usize = 30;

/// A capture scored against the candidate's k-step predictions
/// (`RB-VERIFY-003-FR-008`, `simulate_recorded_k_step`): each frame is
/// predicted `k` ticks ahead from the recorded frame `k` before it, then
/// scored like `--self`. Steadier than one free run for comparing model
/// changes, and it sees positions, which one-step barely moves.
pub fn k_step_score(
    capture_path: impl AsRef<Path>,
    seed_frame: SeedFrame,
    k: usize,
    max_timestamp_delta_secs: f32,
) -> Result<DivergenceScore, IngestError> {
    let (recorded, world) = seed(capture_path, seed_frame)?;
    let candidate = simulate_recorded_k_step(world, &recorded, k);
    Ok(rb_domain::divergence::score(
        &recorded,
        &candidate,
        max_timestamp_delta_secs,
    ))
}

/// Every car at every frame of a capture against the candidate's k-step
/// prediction of that frame (`RB-VERIFY-003-FR-008`): the rows behind
/// [`k_step_score`], for finding where the quarter-second error peaks.
pub fn k_step_capture(
    capture_path: impl AsRef<Path>,
    seed_frame: SeedFrame,
    k: usize,
) -> Result<Vec<TraceRow>, IngestError> {
    let (recorded, world) = seed(capture_path, seed_frame)?;
    let candidate = simulate_recorded_k_step(world, &recorded, k);
    Ok(trace_rows(&recorded, &candidate, 0.0, f32::INFINITY))
}

/// Pairs recorded and candidate frames by index and cars by `player_id`,
/// keeping frames whose time since the first recorded frame falls in
/// `[from_secs, to_secs]`.
fn trace_rows(
    recorded: &[PhysicsFrame],
    candidate: &[PhysicsFrame],
    from_secs: f32,
    to_secs: f32,
) -> Vec<TraceRow> {
    let Some(origin) = recorded.first().map(|frame| frame.timestamp_secs) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (rec, cand) in recorded.iter().zip(candidate.iter()) {
        let t_secs = rec.timestamp_secs - origin;
        if t_secs < from_secs || t_secs > to_secs {
            continue;
        }
        for rec_car in &rec.cars {
            let Some(cand_car) = cand
                .cars
                .iter()
                .find(|car| car.player_id == rec_car.player_id)
            else {
                continue;
            };
            rows.push(TraceRow {
                t_secs,
                input: rec_car.input,
                recorded: *rec_car,
                candidate: *cand_car,
                recorded_ball: rec.ball,
                candidate_ball: cand.ball,
            });
        }
    }
    rows
}

/// Shared plumbing behind [`score_capture_against_candidate`] and
/// [`score_capture_growth`]: ingest the capture, find its first grounded,
/// neutral frame (`is_grounded_and_neutral`), and simulate a candidate
/// trajectory forward from there using that capture's own recorded input.
/// Returns the recorded ground truth from the seed frame onward alongside
/// the simulated candidate, both ready to hand to either scoring function.
fn seed_and_simulate(
    capture_path: impl AsRef<Path>,
    seed_frame: SeedFrame,
) -> Result<(Vec<PhysicsFrame>, Vec<PhysicsFrame>), IngestError> {
    let (recorded, world) = seed(capture_path, seed_frame)?;
    let candidate = simulate_recorded(world, &recorded);
    Ok((recorded, candidate))
}

/// The capture from its seed frame on (`seed_frame`: its first grounded,
/// neutral frame, or simply its first), and a world seeded from that frame.
fn seed(
    capture_path: impl AsRef<Path>,
    seed_frame: SeedFrame,
) -> Result<(Vec<PhysicsFrame>, PhysicsWorld), IngestError> {
    let captured = CaptureFileSource::new(capture_path.as_ref()).frames()?;

    let seed_index = match seed_frame {
        SeedFrame::FirstGroundedNeutral => captured
            .iter()
            .position(is_grounded_and_neutral)
            .ok_or_else(|| {
                IngestError::Malformed(
                    "no grounded, neutral frame found to seed a candidate simulation from (--seed-first-frame starts from the first frame regardless)".to_string(),
                )
            })?,
        SeedFrame::First => {
            if captured.first().is_none_or(|frame| frame.cars.is_empty()) {
                return Err(IngestError::Malformed(
                    "the capture's first frame has no car to seed a candidate simulation from"
                        .to_string(),
                ));
            }
            0
        }
    };

    let recorded = captured[seed_index..].to_vec();
    let mut world = PhysicsWorld::from_frame(&recorded[0]);
    // The captures scored here are the owner's keyboard recordings.
    world.set_dodge_forward_from_throttle(true);
    if boost_is_unlimited(&recorded) {
        world.set_boost_used_per_second(0.0);
    }
    Ok((recorded, world))
}

/// Frames of held boost (with fuel in the tank) a capture needs before an
/// untouched tank counts as unlimited: a tenth of a second at 120 Hz.
const UNLIMITED_BOOST_MIN_HELD_FRAMES: usize = 12;

/// Whether a capture was recorded with unlimited boost
/// (`RB-PHYSICS-001-FR-111`): some car holds boost with fuel in the tank
/// for at least `UNLIMITED_BOOST_MIN_HELD_FRAMES` frames, yet no car's fuel
/// ever drops from one frame to the next. A capture that never boosts
/// can't tell, and keeps the default drain.
pub fn boost_is_unlimited(frames: &[PhysicsFrame]) -> bool {
    let held = frames
        .iter()
        .flat_map(|frame| &frame.cars)
        .filter(|car| car.boost_amount > 0.0 && car.input.is_some_and(|input| input.boost))
        .count();
    let drained = frames.windows(2).any(|pair| {
        pair[1].cars.iter().any(|after| {
            pair[0]
                .cars
                .iter()
                .find(|before| before.player_id == after.player_id)
                .is_some_and(|before| after.boost_amount < before.boost_amount)
        })
    });
    held >= UNLIMITED_BOOST_MIN_HELD_FRAMES && !drained
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rb_scenario::Scenario;

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
        let score = score_capture_against_candidate(
            capture_fixture(),
            SeedFrame::default(),
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        )
        .unwrap();
        assert!(score.frames_compared > 0);
        assert!(score.cars.pairs_compared > 0);
    }

    #[test]
    fn capture_against_candidate_missing_file_reports_io_error() {
        let result = score_capture_against_candidate(
            "does-not-exist.capture.jsonl",
            SeedFrame::default(),
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

        let result = score_capture_against_candidate(
            &path,
            SeedFrame::default(),
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        );

        std::fs::remove_file(&path).ok();
        assert!(matches!(result, Err(IngestError::Malformed(_))));
    }

    #[test]
    fn growth_diagnostic_runs_against_the_synthetic_capture_fixture_without_erroring() {
        let windows = score_capture_growth(
            capture_fixture(),
            SeedFrame::default(),
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
            DEFAULT_GROWTH_WINDOW_SECS,
        )
        .unwrap();
        assert!(!windows.is_empty());
        let total_frames_compared: usize =
            windows.iter().map(|(_, score)| score.frames_compared).sum();
        assert!(total_frames_compared > 0);
    }

    #[test]
    fn growth_diagnostic_missing_file_reports_io_error() {
        let result = score_capture_growth(
            "does-not-exist.capture.jsonl",
            SeedFrame::default(),
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
            DEFAULT_GROWTH_WINDOW_SECS,
        );
        assert!(matches!(result, Err(IngestError::Io(_))));
    }

    #[test]
    fn trace_starts_at_the_seed_frame_with_zero_error() {
        let rows =
            trace_capture(capture_fixture(), SeedFrame::default(), 0.0, f32::INFINITY).unwrap();
        let first = rows.first().unwrap();
        assert_eq!(first.t_secs, 0.0);
        // The candidate world is seeded from this exact recorded frame.
        assert_eq!(first.position_error(), 0.0);
        assert_eq!(first.velocity_error(), 0.0);
        assert_eq!(first.spin_error(), 0.0);
        assert!(first.input.is_some());
    }

    #[test]
    fn car_frame_spin_reads_a_world_spin_in_the_cars_axes() {
        // A car yawed 90 deg left: its forward is world +Y, so a world spin
        // about +Y is a roll about its own forward axis.
        let half = std::f32::consts::FRAC_PI_4;
        let yawed = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let local = car_frame_spin(&yawed, &Vec3::new(0.0, 2.0, 0.0));
        assert!(
            (local - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5,
            "{local:?}"
        );
        let level = car_frame_spin(&Quat::IDENTITY, &Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(level, Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn rotation_rate_recovers_a_known_spin() {
        // 0.1 rad about +Z over 0.1 s is 1 rad/s about +Z.
        let half = 0.05_f32;
        let to = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let rate = rotation_rate(&Quat::IDENTITY, &to, 0.1);
        assert!(
            (rate - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-4,
            "{rate:?}"
        );
        assert_eq!(rotation_rate(&to, &to, 0.1), Vec3::ZERO);
        assert_eq!(rotation_rate(&Quat::IDENTITY, &to, 0.0), Vec3::ZERO);
    }

    #[test]
    fn trace_only_returns_frames_inside_the_window() {
        let all =
            trace_capture(capture_fixture(), SeedFrame::default(), 0.0, f32::INFINITY).unwrap();
        let last_t = all.last().unwrap().t_secs;
        assert!(last_t > 0.0, "fixture needs more than one traced frame");

        let later = trace_capture(
            capture_fixture(),
            SeedFrame::default(),
            last_t,
            f32::INFINITY,
        )
        .unwrap();
        assert!(!later.is_empty());
        assert!(later.iter().all(|row| row.t_secs >= last_t));
        assert!(later.len() < all.len());
    }

    #[test]
    fn one_step_rows_cover_the_whole_capture_and_start_exact() {
        let continuous =
            trace_capture(capture_fixture(), SeedFrame::default(), 0.0, f32::INFINITY).unwrap();
        let one_step = one_step_capture(capture_fixture(), SeedFrame::default()).unwrap();
        assert_eq!(one_step.len(), continuous.len());
        assert_eq!(one_step[0].velocity_error(), 0.0);
        // RB-VERIFY-003-FR-007: each row carries the ball too.
        assert_eq!(one_step[0].ball_velocity_error(), 0.0);
        assert_eq!(one_step[0].recorded_ball, one_step[0].candidate_ball);
    }

    #[test]
    fn one_step_missing_file_reports_io_error() {
        let result = one_step_capture("does-not-exist.capture.jsonl", SeedFrame::default());
        assert!(matches!(result, Err(IngestError::Io(_))));
    }

    /// RB-VERIFY-003-FR-008: every frame is scored, and predicting from
    /// further back can only let more error build up on this fixture.
    #[test]
    fn k_step_scores_every_frame_and_grows_with_k() {
        let frames = one_step_capture(capture_fixture(), SeedFrame::default())
            .unwrap()
            .len();
        let one = k_step_score(
            capture_fixture(),
            SeedFrame::default(),
            1,
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        )
        .unwrap();
        let thirty = k_step_score(
            capture_fixture(),
            SeedFrame::default(),
            30,
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        )
        .unwrap();
        assert_eq!(one.frames_compared, frames);
        assert_eq!(thirty.frames_compared, frames);
        assert!(thirty.cars.mean_position_distance >= one.cars.mean_position_distance);
    }

    #[test]
    fn k_step_missing_file_reports_io_error() {
        let result = k_step_score(
            "does-not-exist.capture.jsonl",
            SeedFrame::default(),
            30,
            0.02,
        );
        assert!(matches!(result, Err(IngestError::Io(_))));
    }

    /// `frames` frames of one car holding `boost`, its fuel `fuel(i)`.
    fn boost_frames(frames: usize, boost: bool, fuel: impl Fn(usize) -> f32) -> Vec<PhysicsFrame> {
        (0..frames)
            .map(|i| PhysicsFrame {
                timestamp_secs: i as f32 / 120.0,
                ball: BallState {
                    position: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    velocity: Vec3::ZERO,
                    angular_velocity: Vec3::ZERO,
                },
                cars: vec![CarState {
                    player_id: 0,
                    position: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    velocity: Vec3::ZERO,
                    angular_velocity: Vec3::ZERO,
                    boost_amount: fuel(i),
                    input: Some(ControllerInput {
                        boost,
                        ..ControllerInput::default()
                    }),
                }],
            })
            .collect()
    }

    /// RB-PHYSICS-001-FR-111: a tank that never drops while boost is held
    /// is unlimited; one that drains, or one never boosted, is not.
    #[test]
    fn unlimited_boost_is_detected_only_from_a_held_boost_that_never_drains() {
        assert!(boost_is_unlimited(&boost_frames(12, true, |_| 100.0)));
        assert!(!boost_is_unlimited(&boost_frames(11, true, |_| 100.0)));
        assert!(!boost_is_unlimited(&boost_frames(60, false, |_| 100.0)));
        assert!(!boost_is_unlimited(&boost_frames(60, true, |i| {
            100.0 - i as f32 * 0.28
        })));
        assert!(!boost_is_unlimited(&boost_frames(60, true, |_| 0.0)));
    }

    #[test]
    fn trace_with_an_inverted_window_is_empty() {
        let rows = trace_capture(capture_fixture(), SeedFrame::default(), 1.0, 0.0).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn trace_missing_file_reports_io_error() {
        let result = trace_capture(
            "does-not-exist.capture.jsonl",
            SeedFrame::default(),
            0.0,
            1.0,
        );
        assert!(matches!(result, Err(IngestError::Io(_))));
    }

    #[test]
    fn growth_diagnostic_with_no_grounded_neutral_frame_reports_malformed() {
        let line = r#"{"timestamp_secs":0.0,"ball":{"position":{"x":0.0,"y":0.0,"z":93.0},"rotation":{"x":0.0,"y":0.0,"z":0.0,"w":1.0},"velocity":{"x":0.0,"y":0.0,"z":0.0},"angular_velocity":{"x":0.0,"y":0.0,"z":0.0}},"cars":[{"player_id":0,"position":{"x":0.0,"y":0.0,"z":17.0},"rotation":{"x":0.0,"y":0.0,"z":0.0,"w":1.0},"velocity":{"x":0.0,"y":0.0,"z":0.0},"angular_velocity":{"x":0.0,"y":0.0,"z":0.0},"boost_amount":33.0,"input":{"throttle":0.0,"steer":0.0,"pitch":0.0,"yaw":0.0,"roll":0.0,"jump":true,"boost":false,"handbrake":false}}]}"#;
        let path =
            std::env::temp_dir().join("rb_verify_cli_test_growth_no_grounded_neutral_frame.jsonl");
        std::fs::write(&path, format!("{line}\n{line}\n")).unwrap();

        let result = score_capture_growth(
            &path,
            SeedFrame::default(),
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
            DEFAULT_GROWTH_WINDOW_SECS,
        );

        std::fs::remove_file(&path).ok();
        assert!(matches!(result, Err(IngestError::Malformed(_))));
    }

    fn scenario(name: &str) -> Scenario {
        let path = format!(
            "{}/../../tools/rb_tape_bot/scenarios/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        Scenario::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn a_scenario_run_has_one_frame_per_tick_and_the_start_state_first() {
        let sc = scenario("prompt_dodge");
        let frames = simulate_scenario(&sc);
        assert_eq!(frames.len() as u64, sc.total_ticks() + 2);
        assert_eq!(frames[0].cars[0].input, None);
        assert!(frames[1].cars[0].input.is_some());
        assert!(frames[1].timestamp_secs > 0.0);
    }

    #[test]
    fn the_prompt_dodge_fires_and_the_late_dodge_does_not() {
        // The scenarios face +y, so a forward dodge adds speed along +y.
        let fastest_y = |name: &str| {
            simulate_scenario(&scenario(name))
                .iter()
                .map(|f| f.cars[0].velocity.y)
                .fold(0.0_f32, f32::max)
        };
        assert!(fastest_y("prompt_dodge") > 400.0, "dodge must fire");
        assert!(fastest_y("late_dodge") < 50.0, "past the 1.25 s window");
    }

    #[test]
    fn a_late_enough_wavedash_keeps_its_speed_and_an_early_one_hops() {
        let mid = simulate_scenario(&scenario("wavedash_mid"));
        // Down to the floor and moving about 1300 uu/s along +y (from 900).
        let landed = &mid[60];
        assert!(landed.cars[0].position.z < 20.0, "{:?}", landed.cars[0]);
        assert!(landed.cars[0].velocity.y > 1100.0, "{:?}", landed.cars[0]);
        let early = simulate_scenario(&scenario("wavedash_early"));
        assert!(early[60].cars[0].position.z > 40.0, "the early flip hops");
    }

    /// A capture whose car starts 500 uu up and falls, never touching the
    /// floor: no frame is grounded, so the default seed has nothing to use.
    fn airborne_capture(name: &str) -> std::path::PathBuf {
        let lines: Vec<String> = (0..12)
            .map(|tick| {
                let t = tick as f32 / 120.0;
                let z = 500.0 - 300.0 * t;
                let x = 100.0 * t;
                format!(
                    r#"{{"timestamp_secs":{t},"ball":{{"position":{{"x":2000.0,"y":0.0,"z":93.15}},"rotation":{{"x":0.0,"y":0.0,"z":0.0,"w":1.0}},"velocity":{{"x":0.0,"y":0.0,"z":0.0}},"angular_velocity":{{"x":0.0,"y":0.0,"z":0.0}}}},"cars":[{{"player_id":0,"position":{{"x":{x},"y":0.0,"z":{z}}},"rotation":{{"x":0.0,"y":0.0,"z":0.0,"w":1.0}},"velocity":{{"x":100.0,"y":0.0,"z":-300.0}},"angular_velocity":{{"x":0.0,"y":0.0,"z":0.0}},"boost_amount":50.0,"input":{{"throttle":0.0,"steer":0.0,"pitch":0.0,"yaw":0.0,"roll":0.0,"jump":false,"boost":false,"handbrake":false}}}}]}}"#
                )
            })
            .collect();
        let path = std::env::temp_dir().join(format!("rb_verify_cli_test_airborne_{name}.jsonl"));
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    #[test]
    fn an_airborne_start_needs_seed_first_frame() {
        let path = airborne_capture("self");
        let default = score_capture_against_candidate(
            &path,
            SeedFrame::FirstGroundedNeutral,
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        );
        assert!(
            matches!(default, Err(IngestError::Malformed(ref msg)) if msg.contains("no grounded, neutral frame")),
            "{default:?}"
        );
        let first = score_capture_against_candidate(
            &path,
            SeedFrame::First,
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        )
        .unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(first.frames_compared, 12);
        assert_eq!(first.cars.pairs_compared, 12);
    }

    #[test]
    fn seed_first_frame_starts_every_self_mode_at_the_first_frame() {
        let path = airborne_capture("modes");
        assert!(one_step_capture(&path, SeedFrame::FirstGroundedNeutral).is_err());
        let one_step = one_step_capture(&path, SeedFrame::First).unwrap();
        assert_eq!(one_step.len(), 12);
        assert_eq!(one_step[0].t_secs, 0.0);
        assert_eq!(
            one_step[0].position_error(),
            0.0,
            "seeded from the first frame"
        );
        let k_step =
            k_step_score(&path, SeedFrame::First, 3, DEFAULT_MAX_TIMESTAMP_DELTA_SECS).unwrap();
        assert_eq!(k_step.frames_compared, 12);
        let growth = score_capture_growth(
            &path,
            SeedFrame::First,
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
            1.0,
        )
        .unwrap();
        assert_eq!(growth.len(), 1);
        let trace = trace_capture(&path, SeedFrame::First, 0.0, f32::INFINITY).unwrap();
        assert_eq!(trace.len(), 12);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn seed_first_frame_still_refuses_a_capture_whose_first_frame_has_no_car() {
        let line = r#"{"timestamp_secs":0.0,"ball":{"position":{"x":0.0,"y":0.0,"z":93.0},"rotation":{"x":0.0,"y":0.0,"z":0.0,"w":1.0},"velocity":{"x":0.0,"y":0.0,"z":0.0},"angular_velocity":{"x":0.0,"y":0.0,"z":0.0}},"cars":[]}"#;
        let path = std::env::temp_dir().join("rb_verify_cli_test_first_frame_no_car.jsonl");
        std::fs::write(&path, format!("{line}\n{line}\n")).unwrap();
        let result = k_step_score(&path, SeedFrame::First, 3, DEFAULT_MAX_TIMESTAMP_DELTA_SECS);
        std::fs::remove_file(&path).ok();
        assert!(
            matches!(result, Err(IngestError::Malformed(_))),
            "{result:?}"
        );
    }
}
