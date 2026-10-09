//! The divergence metric: how far a candidate physics engine's output
//! drifts from a recorded ground-truth trajectory when fed the same
//! initial state and inputs. This number is what Phase 1+ physics work
//! tunes against — see `RB-VERIFY-003`.

use crate::state::{CarState, PhysicsFrame};
use std::collections::HashMap;

/// Divergence between a recorded trajectory and a candidate simulation's
/// trajectory, expressed in the same units as `Vec3` positions (unreal
/// units, matching replay/BakkesMod conventions).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DivergenceScore {
    /// Mean ball-position distance across all compared frames.
    pub mean_ball_distance: f32,
    /// Largest single-frame ball-position distance observed.
    pub max_ball_distance: f32,
    /// Number of recorded frames matched to a candidate frame within
    /// `max_timestamp_delta_secs` (see `score`) — not necessarily
    /// `min(recorded.len(), candidate.len())` now that matching is by
    /// nearest timestamp rather than list index.
    pub frames_compared: usize,
    /// Car position/rotation/velocity divergence (`RB-VERIFY-003-FR-002`).
    pub cars: CarDivergence,
}

/// Divergence between matched cars across a frame pair sequence.
///
/// Cars are matched by `player_id` within each frame pair, not by list
/// position — `CarState.player_id` is the stable per-sequence identity
/// (see `rb_domain::state::CarState`'s doc comment), and a replay/capture's
/// car ordering isn't guaranteed to agree between two independently
/// produced sequences. A car present on only one side of a frame pair
/// (not yet spawned, already destroyed, or simply absent from that
/// frame — see `PhysicsFrame`'s contract) is skipped for that frame, not
/// treated as an error, the same convention `rb_replay_ingest`/
/// `rb_capture_ingest` use for a car missing from one frame's `cars`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarDivergence {
    /// Mean position distance across all matched car pairs.
    pub mean_position_distance: f32,
    /// Largest single-pair position distance observed.
    pub max_position_distance: f32,
    /// Mean rotation angle (radians, see `Quat::angle_to`) across all
    /// matched car pairs.
    pub mean_rotation_distance: f32,
    /// Largest single-pair rotation angle observed.
    pub max_rotation_distance: f32,
    /// Mean velocity-vector distance across all matched car pairs.
    pub mean_velocity_distance: f32,
    /// Largest single-pair velocity distance observed.
    pub max_velocity_distance: f32,
    /// Number of car pairs (recorded car matched to a candidate car by
    /// `player_id`, within one matched frame pair) the score was computed
    /// over.
    pub pairs_compared: usize,
}

/// Finds the candidate frame nearest in time to `target`, advancing
/// `candidate_idx` forward only — valid because both `recorded` and
/// `candidate` are chronologically ordered (`PhysicsFrame`'s contract), so
/// successive `target`s (walking `recorded` in order) never need to look
/// backward in `candidate`. This makes the whole `score` scan `O(recorded.len()
/// + candidate.len())` instead of a binary search per frame.
///
/// Ties (`candidate[idx]` and `candidate[idx + 1]` exactly equidistant from
/// `target`) resolve to the later frame — an arbitrary but deterministic
/// choice; nothing in `RB-VERIFY-003` depends on which side a tie breaks
/// toward.
fn advance_to_nearest(candidate: &[PhysicsFrame], candidate_idx: &mut usize, target: f32) {
    while *candidate_idx + 1 < candidate.len()
        && (candidate[*candidate_idx + 1].timestamp_secs - target).abs()
            <= (candidate[*candidate_idx].timestamp_secs - target).abs()
    {
        *candidate_idx += 1;
    }
}

/// A recorded frame matched to its nearest-in-time candidate frame (see
/// `matched_pairs`) — the shared unit both `score` and `score_windows`
/// (`RB-VERIFY-003-FR-004`) aggregate over.
type MatchedPair<'a> = (&'a PhysicsFrame, &'a PhysicsFrame);

/// Matches `recorded` frames to their nearest `candidate` frame by
/// timestamp (`RB-VERIFY-003-FR-003`) — tolerant of the two sequences
/// being sampled at different tick rates, or one running slightly
/// ahead/behind the other, which the original index-pairwise comparison
/// assumed away.
///
/// For each `recorded` frame (walked in chronological order), the nearest
/// `candidate` frame by `timestamp_secs` is found in amortized-constant
/// time per frame (`advance_to_nearest`). A match is only kept if the two
/// frames' timestamps are within `max_timestamp_delta_secs` of each other;
/// a `recorded` frame with no `candidate` frame that close is skipped
/// entirely, not force-matched to the nearest-but-still-distant one — the
/// same "absent, not an error" convention `CarDivergence`'s matching
/// already uses. What counts as "close enough" depends on both sequences'
/// actual sampling rates, so this is a required parameter, not a baked-in
/// default — see `rb_verify_cli::DEFAULT_MAX_TIMESTAMP_DELTA_SECS` for the
/// value the CLI actually uses and why.
fn matched_pairs<'a>(
    recorded: &'a [PhysicsFrame],
    candidate: &'a [PhysicsFrame],
    max_timestamp_delta_secs: f32,
) -> Vec<MatchedPair<'a>> {
    let mut pairs = Vec::new();
    if candidate.is_empty() {
        return pairs;
    }

    let mut candidate_idx = 0usize;
    for r in recorded {
        advance_to_nearest(candidate, &mut candidate_idx, r.timestamp_secs);
        let c = &candidate[candidate_idx];
        if (c.timestamp_secs - r.timestamp_secs).abs() > max_timestamp_delta_secs {
            continue;
        }
        pairs.push((r, c));
    }
    pairs
}

/// Aggregates an already-matched sequence of recorded/candidate frame
/// pairs into a single `DivergenceScore` — the same aggregation `score`
/// performs over the whole run's own matched pairs, factored out so
/// `score_windows` (`RB-VERIFY-003-FR-004`) can apply it per time window
/// instead of once over the entire run.
fn score_pairs(pairs: &[MatchedPair<'_>]) -> DivergenceScore {
    let mut ball_sum = 0.0;
    let mut ball_max = 0.0f32;

    let mut position_sum = 0.0;
    let mut position_max = 0.0f32;
    let mut rotation_sum = 0.0;
    let mut rotation_max = 0.0f32;
    let mut velocity_sum = 0.0;
    let mut velocity_max = 0.0f32;
    let mut pairs_compared = 0usize;

    for (r, c) in pairs {
        let d = r.ball.position.distance(&c.ball.position);
        ball_sum += d;
        ball_max = ball_max.max(d);

        let candidate_cars: HashMap<u32, &CarState> =
            c.cars.iter().map(|car| (car.player_id, car)).collect();

        for recorded_car in &r.cars {
            let Some(candidate_car) = candidate_cars.get(&recorded_car.player_id) else {
                continue;
            };

            let pd = recorded_car.position.distance(&candidate_car.position);
            position_sum += pd;
            position_max = position_max.max(pd);

            let rd = recorded_car.rotation.angle_to(&candidate_car.rotation);
            rotation_sum += rd;
            rotation_max = rotation_max.max(rd);

            let vd = recorded_car.velocity.distance(&candidate_car.velocity);
            velocity_sum += vd;
            velocity_max = velocity_max.max(vd);

            pairs_compared += 1;
        }
    }

    let frames_compared = pairs.len();
    let mean_ball_distance = if frames_compared == 0 {
        0.0
    } else {
        ball_sum / frames_compared as f32
    };

    let mean_of = |sum: f32| {
        if pairs_compared == 0 {
            0.0
        } else {
            sum / pairs_compared as f32
        }
    };

    DivergenceScore {
        mean_ball_distance,
        max_ball_distance: ball_max,
        frames_compared,
        cars: CarDivergence {
            mean_position_distance: mean_of(position_sum),
            max_position_distance: position_max,
            mean_rotation_distance: mean_of(rotation_sum),
            max_rotation_distance: rotation_max,
            mean_velocity_distance: mean_of(velocity_sum),
            max_velocity_distance: velocity_max,
            pairs_compared,
        },
    }
}

/// Compares two frame sequences by nearest timestamp, not list index
/// (`RB-VERIFY-003-FR-003`) — tolerant of the two sequences being sampled
/// at different tick rates, or one running slightly ahead/behind the
/// other, which the original index-pairwise comparison assumed away. See
/// `matched_pairs` for the matching rule and `score_pairs` for the
/// aggregation; `score` is just that pipeline run once over the whole run.
pub fn score(
    recorded: &[PhysicsFrame],
    candidate: &[PhysicsFrame],
    max_timestamp_delta_secs: f32,
) -> DivergenceScore {
    score_pairs(&matched_pairs(
        recorded,
        candidate,
        max_timestamp_delta_secs,
    ))
}

/// A divergence-growth diagnostic (`RB-VERIFY-003-FR-004`): partitions the
/// same nearest-timestamp-matched pairs `score` computes (see
/// `matched_pairs`) into consecutive, non-overlapping `window_secs`-wide
/// buckets keyed by each pair's own recorded timestamp — the first window
/// starts at the first matched pair's own timestamp, not a fixed `0.0`, so
/// a mid-file seed frame doesn't produce a mostly-empty leading window —
/// and scores each bucket independently (`score_pairs`), returning each
/// window's start time paired with its own score.
///
/// This exposes how divergence changes *within* a run instead of
/// collapsing it to one whole-run mean/max pair: a run whose matched pairs
/// all fall within a single window reproduces exactly the same numbers
/// `score` computes on the same input, since both build on the identical
/// matched pairs and the identical aggregation.
///
/// Windows with no matched pairs are omitted, not zero-filled — consistent
/// with `score`'s own "empty means `0.0`, not an error" contract, but
/// avoiding a wall of meaningless zero rows for a run with a long gap.
/// `window_secs` must be positive; choosing a sensible width is the
/// caller's responsibility (see `rb_verify_cli`'s own CLI default).
pub fn score_windows(
    recorded: &[PhysicsFrame],
    candidate: &[PhysicsFrame],
    max_timestamp_delta_secs: f32,
    window_secs: f32,
) -> Vec<(f32, DivergenceScore)> {
    let pairs = matched_pairs(recorded, candidate, max_timestamp_delta_secs);
    let Some((first, _)) = pairs.first() else {
        return Vec::new();
    };
    let origin = first.timestamp_secs;

    let mut windows: Vec<(i64, Vec<MatchedPair<'_>>)> = Vec::new();
    for pair in pairs {
        let index = ((pair.0.timestamp_secs - origin) / window_secs).floor() as i64;
        match windows.last_mut() {
            Some((window_index, bucket)) if *window_index == index => bucket.push(pair),
            _ => windows.push((index, vec![pair])),
        }
    }

    windows
        .into_iter()
        .map(|(index, bucket)| (origin + index as f32 * window_secs, score_pairs(&bucket)))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::state::{BallState, Quat, Vec3};

    /// Generous enough that every test not specifically exercising
    /// tolerance behavior can treat "same timestamp" or "close enough
    /// timestamp" matches as unconditional.
    const GENEROUS_TOLERANCE: f32 = 1.0;

    fn frame_with_ball_x(timestamp_secs: f32, x: f32) -> PhysicsFrame {
        PhysicsFrame {
            timestamp_secs,
            ball: BallState {
                position: Vec3 { x, y: 0.0, z: 0.0 },
                rotation: Quat::IDENTITY,
                velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
            },
            cars: Vec::<CarState>::new(),
        }
    }

    fn car(player_id: u32, x: f32, rotation: Quat, vx: f32) -> CarState {
        CarState {
            player_id,
            position: Vec3::new(x, 0.0, 0.0),
            rotation,
            velocity: Vec3::new(vx, 0.0, 0.0),
            angular_velocity: Vec3::ZERO,
            boost_amount: 0.0,
            input: None,
        }
    }

    fn frame_with_cars(timestamp_secs: f32, cars: Vec<CarState>) -> PhysicsFrame {
        PhysicsFrame {
            timestamp_secs,
            ball: BallState {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
            },
            cars,
        }
    }

    #[test]
    fn identical_trajectories_score_zero() {
        let frames = vec![frame_with_ball_x(0.0, 0.0), frame_with_ball_x(1.0, 10.0)];
        let result = score(&frames, &frames, GENEROUS_TOLERANCE);
        assert_eq!(result.mean_ball_distance, 0.0);
        assert_eq!(result.max_ball_distance, 0.0);
        assert_eq!(result.frames_compared, 2);
        assert_eq!(result.cars.pairs_compared, 0);
    }

    #[test]
    fn constant_offset_scores_that_offset() {
        let recorded = vec![frame_with_ball_x(0.0, 0.0), frame_with_ball_x(1.0, 0.0)];
        let candidate = vec![frame_with_ball_x(0.0, 5.0), frame_with_ball_x(1.0, 5.0)];
        let result = score(&recorded, &candidate, GENEROUS_TOLERANCE);
        assert_eq!(result.mean_ball_distance, 5.0);
        assert_eq!(result.max_ball_distance, 5.0);
    }

    #[test]
    fn frames_beyond_the_timestamp_tolerance_are_skipped_not_force_matched() {
        let recorded = vec![
            frame_with_ball_x(0.0, 0.0),
            frame_with_ball_x(1.0, 0.0),
            frame_with_ball_x(2.0, 0.0),
        ];
        // Only one candidate frame exists, at t=0 — recorded's t=1 and t=2
        // frames are outside a 0.5s tolerance of it.
        let candidate = vec![frame_with_ball_x(0.0, 2.0)];
        let result = score(&recorded, &candidate, 0.5);
        assert_eq!(result.frames_compared, 1);
        assert_eq!(result.mean_ball_distance, 2.0);
    }

    #[test]
    fn different_tick_rates_align_by_nearest_timestamp_not_index() {
        // Recorded at a coarse, regular tick; candidate at irregular
        // timestamps that don't line up with recorded's — the scenario
        // RB-VERIFY-003-FR-003 exists for.
        let recorded = vec![
            frame_with_ball_x(0.0, 0.0),
            frame_with_ball_x(0.1, 0.0),
            frame_with_ball_x(0.2, 0.0),
        ];
        let candidate = vec![
            frame_with_ball_x(0.0, 100.0),
            frame_with_ball_x(0.09, 200.0),
            frame_with_ball_x(0.22, 300.0),
        ];
        // Nearest matches: recorded 0.0 -> candidate 0.0 (delta 0.0);
        // recorded 0.1 -> candidate 0.09 (delta 0.01, closer than 0.22's
        // 0.12); recorded 0.2 -> candidate 0.22 (delta 0.02).
        let result = score(&recorded, &candidate, 0.05);
        assert_eq!(result.frames_compared, 3);
        assert_eq!(result.mean_ball_distance, (100.0 + 200.0 + 300.0) / 3.0);
        assert_eq!(result.max_ball_distance, 300.0);
    }

    #[test]
    fn a_shorter_sequence_can_still_match_every_recorded_frame() {
        // Three recorded frames close enough together that the single
        // candidate frame is within tolerance of all of them — unlike the
        // old index-pairwise comparison, sequence length alone no longer
        // caps how many frames can be compared.
        let recorded = vec![
            frame_with_ball_x(0.0, 0.0),
            frame_with_ball_x(0.01, 0.0),
            frame_with_ball_x(0.02, 0.0),
        ];
        let candidate = vec![frame_with_ball_x(0.01, 5.0)];
        let result = score(&recorded, &candidate, 0.05);
        assert_eq!(result.frames_compared, 3);
        assert_eq!(result.mean_ball_distance, 5.0);
    }

    #[test]
    fn empty_inputs_score_zero_not_nan() {
        let result = score(&[], &[], GENEROUS_TOLERANCE);
        assert_eq!(result.mean_ball_distance, 0.0);
        assert_eq!(result.frames_compared, 0);
        assert_eq!(result.cars.mean_position_distance, 0.0);
        assert_eq!(result.cars.pairs_compared, 0);
    }

    #[test]
    fn identical_car_states_score_zero_car_divergence() {
        let frame = frame_with_cars(0.0, vec![car(0, 100.0, Quat::IDENTITY, 500.0)]);
        let frames = vec![frame];
        let result = score(&frames, &frames, GENEROUS_TOLERANCE);
        assert_eq!(result.cars.pairs_compared, 1);
        assert_eq!(result.cars.mean_position_distance, 0.0);
        assert_eq!(result.cars.mean_rotation_distance, 0.0);
        assert_eq!(result.cars.mean_velocity_distance, 0.0);
    }

    #[test]
    fn car_position_and_velocity_offsets_score_that_distance() {
        let recorded = vec![frame_with_cars(0.0, vec![car(0, 0.0, Quat::IDENTITY, 0.0)])];
        let candidate = vec![frame_with_cars(0.0, vec![car(0, 3.0, Quat::IDENTITY, 4.0)])];
        let result = score(&recorded, &candidate, GENEROUS_TOLERANCE);
        assert_eq!(result.cars.pairs_compared, 1);
        assert_eq!(result.cars.mean_position_distance, 3.0);
        assert_eq!(result.cars.mean_velocity_distance, 4.0);
    }

    #[test]
    fn car_rotation_difference_scores_the_angle_between_them() {
        let half = std::f32::consts::FRAC_PI_4;
        let quarter_turn_z = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let recorded = vec![frame_with_cars(0.0, vec![car(0, 0.0, Quat::IDENTITY, 0.0)])];
        let candidate = vec![frame_with_cars(0.0, vec![car(0, 0.0, quarter_turn_z, 0.0)])];
        let result = score(&recorded, &candidate, GENEROUS_TOLERANCE);
        assert!((result.cars.mean_rotation_distance - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    }

    #[test]
    fn a_car_missing_from_the_candidate_frame_is_skipped_not_an_error() {
        let recorded = vec![frame_with_cars(
            0.0,
            vec![
                car(0, 0.0, Quat::IDENTITY, 0.0),
                car(1, 0.0, Quat::IDENTITY, 0.0),
            ],
        )];
        // Candidate only has player 1 — player 0 has no match this frame.
        let candidate = vec![frame_with_cars(0.0, vec![car(1, 0.0, Quat::IDENTITY, 0.0)])];
        let result = score(&recorded, &candidate, GENEROUS_TOLERANCE);
        assert_eq!(result.cars.pairs_compared, 1);
    }

    #[test]
    fn score_windows_on_a_single_window_run_reproduces_scores_own_numbers() {
        let recorded = vec![
            frame_with_ball_x(0.0, 0.0),
            frame_with_ball_x(0.2, 0.0),
            frame_with_ball_x(0.4, 0.0),
        ];
        let candidate = vec![
            frame_with_ball_x(0.0, 3.0),
            frame_with_ball_x(0.2, 5.0),
            frame_with_ball_x(0.4, 7.0),
        ];
        let whole_run = score(&recorded, &candidate, GENEROUS_TOLERANCE);

        // A single window wide enough to hold the whole ~0.4s run.
        let windows = score_windows(&recorded, &candidate, GENEROUS_TOLERANCE, 10.0);

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].0, 0.0);
        assert_eq!(windows[0].1, whole_run);
    }

    #[test]
    fn score_windows_splits_a_run_into_separate_windows_by_time() {
        // First window (t in [0.0, 1.0)): identical trajectories, zero
        // divergence. Second window (t in [1.0, 2.0)): a known constant
        // offset of 5.0.
        let recorded = vec![
            frame_with_ball_x(0.0, 0.0),
            frame_with_ball_x(0.5, 0.0),
            frame_with_ball_x(1.0, 0.0),
            frame_with_ball_x(1.5, 0.0),
        ];
        let candidate = vec![
            frame_with_ball_x(0.0, 0.0),
            frame_with_ball_x(0.5, 0.0),
            frame_with_ball_x(1.0, 5.0),
            frame_with_ball_x(1.5, 5.0),
        ];
        let windows = score_windows(&recorded, &candidate, GENEROUS_TOLERANCE, 1.0);

        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].0, 0.0);
        assert_eq!(windows[0].1.frames_compared, 2);
        assert_eq!(windows[0].1.mean_ball_distance, 0.0);
        assert_eq!(windows[1].0, 1.0);
        assert_eq!(windows[1].1.frames_compared, 2);
        assert_eq!(windows[1].1.mean_ball_distance, 5.0);
    }

    #[test]
    fn score_windows_starts_the_first_window_at_the_first_matched_pairs_own_timestamp() {
        // The recorded sequence starts well before the candidate does, so
        // the first several recorded frames have no match within
        // tolerance — the first window must start at the first frame that
        // actually matched (t=10.0), not at t=0.0.
        let recorded = vec![
            frame_with_ball_x(0.0, 0.0),
            frame_with_ball_x(5.0, 0.0),
            frame_with_ball_x(10.0, 0.0),
            frame_with_ball_x(10.5, 0.0),
        ];
        let candidate = vec![frame_with_ball_x(10.0, 2.0), frame_with_ball_x(10.5, 2.0)];
        let windows = score_windows(&recorded, &candidate, 0.1, 1.0);

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].0, 10.0);
        assert_eq!(windows[0].1.frames_compared, 2);
    }

    #[test]
    fn score_windows_on_no_matched_pairs_returns_no_windows() {
        let recorded = vec![frame_with_ball_x(0.0, 0.0)];
        let candidate = vec![frame_with_ball_x(100.0, 0.0)];
        let windows = score_windows(&recorded, &candidate, 0.1, 1.0);
        assert!(windows.is_empty());
    }
}
