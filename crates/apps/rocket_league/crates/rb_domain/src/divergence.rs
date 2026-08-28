//! The divergence metric: how far a candidate physics engine's output
//! drifts from a recorded ground-truth trajectory when fed the same
//! initial state and inputs. This number is what Phase 1+ physics work
//! tunes against — see `RB-VERIFY-003`.

use crate::state::PhysicsFrame;

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

    let mut sum = 0.0;
    let mut max = 0.0f32;
    for (r, c) in recorded.iter().zip(candidate.iter()).take(frames_compared) {
        let d = r.ball.position.distance(&c.ball.position);
        sum += d;
        max = max.max(d);
    }

    let mean_ball_distance = if frames_compared == 0 {
        0.0
    } else {
        sum / frames_compared as f32
    };

    DivergenceScore {
        mean_ball_distance,
        max_ball_distance: max,
        frames_compared,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{BallState, CarState, Quat, Vec3};

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

    #[test]
    fn identical_trajectories_score_zero() {
        let frames = vec![frame_with_ball_x(0.0), frame_with_ball_x(10.0)];
        let result = score(&frames, &frames);
        assert_eq!(result.mean_ball_distance, 0.0);
        assert_eq!(result.max_ball_distance, 0.0);
        assert_eq!(result.frames_compared, 2);
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
    }
}
