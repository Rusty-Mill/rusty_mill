//! Car-ball hit sweeps (`RB-VERIFY-003-FR-014`, ADR-0060's second caller of
//! `rb_env::Env`): try many `CarBallTuning`s against short windows of a
//! recording, k ticks ahead from the recorded frame k before each scored
//! frame, and rank them.
//!
//! The environment is tracked along the recording (`Env::snap`), because a
//! prediction restarted cold from a recorded frame (`Env::reset`) loses the
//! car's suspension and contact state: on `hitjump` that alone cost 290 uu/s
//! of car velocity error, swamping every effect swept for.
//!
//! A window objective finds candidates; it does not decide. The one sweep
//! run so far (`RB-RESEARCH-O010`) found a setting that cut a hit's car
//! velocity error 51 -> 8 uu/s and then doubled `test2`'s k = 30 ball
//! distance. Whatever ranks first here must still pass the full-capture
//! `--self-kstep` table before it goes anywhere near the physics crate.

use rb_domain::{ControllerInput, IngestError, PhysicsFrame, PhysicsStateSource};
use rb_env::{Env, TICK_SECS};
use rb_physics_bullet::CarBallTuning;

/// A recording and the span of it to score.
pub struct Window {
    pub frames: Vec<PhysicsFrame>,
    pub from_secs: f32,
    pub to_secs: f32,
}

impl Window {
    /// Reads `capture` (a BakkesMod capture file) and scores `[from, to]`.
    pub fn from_capture(
        capture: &str,
        from_secs: f32,
        to_secs: f32,
    ) -> Result<Window, IngestError> {
        let frames = rb_capture_ingest::CaptureFileSource::new(capture).frames()?;
        Ok(Window {
            frames,
            from_secs,
            to_secs,
        })
    }
}

/// Mean error of a window's k-step predictions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowError {
    /// Frame 0's car, uu/s.
    pub car_velocity: f32,
    /// The ball, uu.
    pub ball_position: f32,
    /// Frames scored; zero means the window held none.
    pub frames: usize,
}

impl WindowError {
    /// What a sweep ranks by: both errors, equally weighted.
    pub fn total(&self) -> f32 {
        self.car_velocity + self.ball_position
    }
}

/// Whether the k frames before `index` were evenly 120 Hz (a capture hole
/// would make `Env`'s fixed tick the wrong clock).
fn evenly_spaced(frames: &[PhysicsFrame], start: usize, index: usize) -> bool {
    let span = frames[index].timestamp_secs - frames[start].timestamp_secs;
    let expected = (index - start) as f32 * TICK_SECS;
    (span - expected).abs() < 0.5 * TICK_SECS
}

fn recorded_inputs(frame: &PhysicsFrame) -> Vec<ControllerInput> {
    frame
        .cars
        .iter()
        .map(|car| car.input.unwrap_or_default())
        .collect()
}

/// Ticks tracked along the recording before a window's first prediction,
/// so contact and suspension state settle before it is asked anything.
const PRE_ROLL_TICKS: usize = 60;

/// `window`'s k-step error under `tuning`: each scored frame is predicted
/// from the recorded frame `k` before it, stepping the recorded inputs.
/// The environment is tracked along the recording (`Env::snap` each tick,
/// as `simulate_recorded_k_step` does) so predictions start from the state
/// a real run would have, not a cold start.
pub fn window_error(
    env: &mut Env,
    window: &Window,
    k: usize,
    tuning: CarBallTuning,
) -> WindowError {
    env.set_car_ball(tuning);
    let k = k.max(1);
    let frames = &window.frames;
    let scored: Vec<usize> = (k..frames.len())
        .filter(|&i| {
            let t = frames[i].timestamp_secs;
            t >= window.from_secs && t <= window.to_secs && evenly_spaced(frames, i - k, i)
        })
        .collect();
    let (Some(&first), Some(&last)) = (scored.first(), scored.last()) else {
        return WindowError {
            car_velocity: 0.0,
            ball_position: 0.0,
            frames: 0,
        };
    };
    let begin = (first - k).saturating_sub(PRE_ROLL_TICKS);
    env.reset(&frames[begin]);
    let inputs: Vec<Vec<ControllerInput>> = frames.iter().map(recorded_inputs).collect();
    let (mut car_velocity, mut ball_position, mut counted) = (0.0, 0.0, 0usize);
    for start in begin..last {
        env.snap(&frames[start]);
        let target = start + k;
        if scored.binary_search(&target).is_ok() {
            let ticks: Vec<&[ControllerInput]> =
                inputs[start..target].iter().map(Vec::as_slice).collect();
            let predicted = env.peek(&ticks);
            if let (Some(recorded_car), Some(predicted_car)) =
                (frames[target].cars.first(), predicted.cars.first())
            {
                car_velocity += (recorded_car.velocity - predicted_car.velocity).length();
                ball_position += (frames[target].ball.position - predicted.ball.position).length();
                counted += 1;
            }
        }
        env.step(&inputs[start]);
    }
    let mean = |sum: f32| {
        if counted == 0 {
            0.0
        } else {
            sum / counted as f32
        }
    };
    WindowError {
        car_velocity: mean(car_velocity),
        ball_position: mean(ball_position),
        frames: counted,
    }
}

/// One tuning's per-window errors.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub tuning: CarBallTuning,
    pub windows: Vec<WindowError>,
}

impl Candidate {
    pub fn total(&self) -> f32 {
        self.windows.iter().map(WindowError::total).sum()
    }
}

/// Every tuning in `tunings` scored over every window, best (lowest total)
/// first. Ties keep the order given, so put the default first.
pub fn sweep(windows: &[Window], k: usize, tunings: &[CarBallTuning]) -> Vec<Candidate> {
    let mut env = Env::new();
    let mut candidates: Vec<Candidate> = tunings
        .iter()
        .map(|&tuning| Candidate {
            tuning,
            windows: windows
                .iter()
                .map(|window| window_error(&mut env, window, k, tuning))
                .collect(),
        })
        .collect();
    candidates.sort_by(|a, b| a.total().total_cmp(&b.total()));
    candidates
}

/// The default grid: RocketSim's own first, then restitution, friction and
/// the extra impulse's scale, z and forward factors around it.
pub fn default_grid() -> Vec<CarBallTuning> {
    let base = CarBallTuning::default();
    let mut grid = vec![base];
    for restitution in [0.0, 0.1, 0.2] {
        for friction in [0.5, 1.0, 2.0] {
            for hit_scale in [0.5, 0.6, 0.7, 0.85, 1.0] {
                for hit_z_scale in [0.2, 0.35, 0.5] {
                    for hit_forward_scale in [0.4, 0.65, 0.9] {
                        let mut tuning = base;
                        tuning.material.restitution = restitution;
                        tuning.material.friction = friction;
                        tuning.hit_scale = hit_scale;
                        tuning.hit_z_scale = hit_z_scale;
                        tuning.hit_forward_scale = hit_forward_scale;
                        if tuning != base {
                            grid.push(tuning);
                        }
                    }
                }
            }
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What re-seeding costs even with the right tuning (uu/s plus uu).
    const WINDOW_FLOOR: f32 = 1.0;

    use rb_domain::{BallState, CarState, Quat, Vec3};

    /// A car at 1500 uu/s reaching a ball 160 uu ahead, recorded from `Env`
    /// itself under `tuning`.
    fn recording(tuning: CarBallTuning, ticks: usize) -> Vec<PhysicsFrame> {
        let start = PhysicsFrame {
            timestamp_secs: 0.0,
            ball: BallState {
                position: Vec3::new(160.0, 0.0, 93.0),
                rotation: Quat::IDENTITY,
                velocity: Vec3::new(0.0, 0.0, 0.0),
                angular_velocity: Vec3::new(0.0, 0.0, 0.0),
            },
            cars: vec![CarState {
                player_id: 0,
                position: Vec3::new(0.0, 0.0, 17.0),
                rotation: Quat::IDENTITY,
                velocity: Vec3::new(1500.0, 0.0, 0.0),
                angular_velocity: Vec3::new(0.0, 0.0, 0.0),
                boost_amount: 100.0,
                input: None,
            }],
        };
        let mut env = Env::new();
        env.set_car_ball(tuning);
        let mut frames = vec![env.reset(&start)];
        let drive = ControllerInput {
            throttle: 1.0,
            ..ControllerInput::default()
        };
        for tick in 1..=ticks {
            let mut frame = env.step(&[drive]);
            frame.timestamp_secs = tick as f32 * TICK_SECS;
            frames.push(frame);
        }
        frames[0].timestamp_secs = 0.0;
        frames
    }

    fn window(tuning: CarBallTuning) -> Window {
        Window {
            frames: recording(tuning, 40),
            from_secs: 0.0,
            to_secs: 1.0,
        }
    }

    fn weak() -> CarBallTuning {
        CarBallTuning {
            hit_scale: 0.0,
            ..CarBallTuning::default()
        }
    }

    #[test]
    fn a_recording_made_under_a_tuning_scores_near_zero_under_it() {
        // Not exactly zero: the tracked environment re-snaps each tick.
        let w = window(weak());
        let error = window_error(&mut Env::new(), &w, 8, weak());
        assert!(error.frames > 0);
        assert!(error.total() < WINDOW_FLOOR, "{error:?}");
    }

    #[test]
    fn the_wrong_tuning_scores_worse() {
        let w = window(weak());
        let mut env = Env::new();
        let right = window_error(&mut env, &w, 8, weak());
        let wrong = window_error(&mut env, &w, 8, CarBallTuning::default());
        assert!(
            wrong.total() > right.total() + 1.0,
            "{wrong:?} vs {right:?}"
        );
    }

    #[test]
    fn a_sweep_ranks_the_generating_tuning_first() {
        let w = window(weak());
        let grid = [CarBallTuning::default(), weak()];
        let ranked = sweep(&[w], 8, &grid);
        assert_eq!(ranked[0].tuning, weak());
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn a_window_outside_the_recording_scores_no_frames() {
        let mut w = window(weak());
        w.from_secs = 50.0;
        w.to_secs = 60.0;
        let error = window_error(&mut Env::new(), &w, 8, weak());
        assert_eq!(error.frames, 0);
        assert_eq!(error.total(), 0.0);
    }

    #[test]
    fn a_capture_hole_skips_the_frames_it_spans() {
        let mut w = window(weak());
        for frame in w.frames.iter_mut().skip(20) {
            frame.timestamp_secs += 5.0 * TICK_SECS;
        }
        let with_hole = window_error(&mut Env::new(), &w, 8, weak());
        let without = window_error(&mut Env::new(), &window(weak()), 8, weak());
        assert!(with_hole.frames < without.frames);
    }

    #[test]
    fn the_default_grid_starts_with_the_default_and_has_no_repeats() {
        let grid = default_grid();
        assert_eq!(grid[0], CarBallTuning::default());
        assert_eq!(grid.iter().filter(|t| **t == grid[0]).count(), 1);
        assert_eq!(grid.len(), 3 * 3 * 5 * 3 * 3);
    }
}
