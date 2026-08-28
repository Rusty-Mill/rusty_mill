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
    /// Number of frame pairs the score was computed over.
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
    /// `player_id`, within one frame pair) the score was computed over.
    pub pairs_compared: usize,
}

/// Compares two frame sequences pairwise by index.
///
/// Pairwise-by-index is a deliberate simplification for the bootstrap
/// baseline: it assumes both sequences were sampled at the same fixed tick
/// rate and start at the same instant, which holds for a
/// candidate-engine-fed-recorded-inputs run (Phase 0's actual use case).
/// Timestamp-tolerant resampling for sequences with independent tick rates
/// is tracked as an open item in `RB-VERIFY-003`'s acceptance criteria, not
/// solved here.
pub fn score(recorded: &[PhysicsFrame], candidate: &[PhysicsFrame]) -> DivergenceScore {
    let frames_compared = recorded.len().min(candidate.len());

    let mut ball_sum = 0.0;
    let mut ball_max = 0.0f32;

    let mut position_sum = 0.0;
    let mut position_max = 0.0f32;
    let mut rotation_sum = 0.0;
    let mut rotation_max = 0.0f32;
    let mut velocity_sum = 0.0;
    let mut velocity_max = 0.0f32;
    let mut pairs_compared = 0usize;

    for (r, c) in recorded.iter().zip(candidate.iter()).take(frames_compared) {
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::state::{BallState, Quat, Vec3};

    fn frame_with_ball_x(x: f32) -> PhysicsFrame {
        PhysicsFrame {
            timestamp_secs: 0.0,
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

    fn frame_with_cars(cars: Vec<CarState>) -> PhysicsFrame {
        PhysicsFrame {
            timestamp_secs: 0.0,
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
        let frames = vec![frame_with_ball_x(0.0), frame_with_ball_x(10.0)];
        let result = score(&frames, &frames);
        assert_eq!(result.mean_ball_distance, 0.0);
        assert_eq!(result.max_ball_distance, 0.0);
        assert_eq!(result.frames_compared, 2);
        assert_eq!(result.cars.pairs_compared, 0);
    }

    #[test]
    fn constant_offset_scores_that_offset() {
        let recorded = vec![frame_with_ball_x(0.0), frame_with_ball_x(0.0)];
        let candidate = vec![frame_with_ball_x(5.0), frame_with_ball_x(5.0)];
        let result = score(&recorded, &candidate);
        assert_eq!(result.mean_ball_distance, 5.0);
        assert_eq!(result.max_ball_distance, 5.0);
    }

    #[test]
    fn mismatched_lengths_compare_only_the_overlap() {
        let recorded = vec![
            frame_with_ball_x(0.0),
            frame_with_ball_x(0.0),
            frame_with_ball_x(0.0),
        ];
        let candidate = vec![frame_with_ball_x(2.0)];
        let result = score(&recorded, &candidate);
        assert_eq!(result.frames_compared, 1);
        assert_eq!(result.mean_ball_distance, 2.0);
    }

    #[test]
    fn empty_inputs_score_zero_not_nan() {
        let result = score(&[], &[]);
        assert_eq!(result.mean_ball_distance, 0.0);
        assert_eq!(result.frames_compared, 0);
        assert_eq!(result.cars.mean_position_distance, 0.0);
        assert_eq!(result.cars.pairs_compared, 0);
    }

    #[test]
    fn identical_car_states_score_zero_car_divergence() {
        let frame = frame_with_cars(vec![car(0, 100.0, Quat::IDENTITY, 500.0)]);
        let frames = vec![frame];
        let result = score(&frames, &frames);
        assert_eq!(result.cars.pairs_compared, 1);
        assert_eq!(result.cars.mean_position_distance, 0.0);
        assert_eq!(result.cars.mean_rotation_distance, 0.0);
        assert_eq!(result.cars.mean_velocity_distance, 0.0);
    }

    #[test]
    fn car_position_and_velocity_offsets_score_that_distance() {
        let recorded = vec![frame_with_cars(vec![car(0, 0.0, Quat::IDENTITY, 0.0)])];
        let candidate = vec![frame_with_cars(vec![car(0, 3.0, Quat::IDENTITY, 4.0)])];
        let result = score(&recorded, &candidate);
        assert_eq!(result.cars.pairs_compared, 1);
        assert_eq!(result.cars.mean_position_distance, 3.0);
        assert_eq!(result.cars.mean_velocity_distance, 4.0);
    }

    #[test]
    fn car_rotation_difference_scores_the_angle_between_them() {
        let half = std::f32::consts::FRAC_PI_4;
        let quarter_turn_z = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let recorded = vec![frame_with_cars(vec![car(0, 0.0, Quat::IDENTITY, 0.0)])];
        let candidate = vec![frame_with_cars(vec![car(0, 0.0, quarter_turn_z, 0.0)])];
        let result = score(&recorded, &candidate);
        assert!((result.cars.mean_rotation_distance - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    }

    #[test]
    fn a_car_missing_from_the_candidate_frame_is_skipped_not_an_error() {
        let recorded = vec![frame_with_cars(vec![
            car(0, 0.0, Quat::IDENTITY, 0.0),
            car(1, 0.0, Quat::IDENTITY, 0.0),
        ])];
        // Candidate only has player 1 — player 0 has no match this frame.
        let candidate = vec![frame_with_cars(vec![car(1, 0.0, Quat::IDENTITY, 0.0)])];
        let result = score(&recorded, &candidate);
        assert_eq!(result.cars.pairs_compared, 1);
    }
}
