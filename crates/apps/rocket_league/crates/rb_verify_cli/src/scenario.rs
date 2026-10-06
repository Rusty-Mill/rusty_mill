//! Scenario runs (`RB-VERIFY-003-FR-010`, `FR-011`, `FR-012`): what the
//! port does with a scripted scenario, how that compares with its recording,
//! and a scenario cut from a window of a recording.

use rb_domain::{CarState, IngestError, PhysicsFrame, Vec3};
use rb_physics_bullet::PhysicsWorld;
use rb_scenario::{quat_to_rotator, BallStart, CarStart, Input, Scenario, Step};

/// Seconds per tick of a scenario run: the game's 120 Hz.
pub const SCENARIO_TICK_SECS: f32 = 1.0 / 120.0;

/// What the port does with a scenario (`docs/research/BOT-CAPTURE-PLAN.md`):
/// the start state, then one frame after each tick of the scenario's input
/// tape (settling and a final neutral tick included). The first frame is the
/// start state and carries no input; later frames carry the input that
/// produced them.
pub fn simulate_scenario(scenario: &Scenario) -> Vec<PhysicsFrame> {
    let start = scenario.initial_frame();
    let mut world = PhysicsWorld::from_frame(&start);
    let mut frames = vec![start];
    for tick in 0..=scenario.total_ticks() {
        let input = scenario.input_at(tick).to_controller_input();
        world.set_car_input(0, input);
        world.step(SCENARIO_TICK_SECS);
        let mut frame = world.frame();
        frame.timestamp_secs = (tick + 1) as f32 * SCENARIO_TICK_SECS;
        if let Some(car) = frame.cars.first_mut() {
            car.input = Some(input);
        }
        frames.push(frame);
    }
    frames
}

/// How far (uu) a recorded car may be from a scenario's start location and
/// still count as the frame where the start state was set.
const START_MATCH_RADIUS: f32 = 5.0;

/// Ticks of recording lag tried when aligning (the start state may take a
/// tick or two to appear in the capture).
const MAX_LAG_TICKS: usize = 3;

/// Ticks used to choose the lag: the early trajectory, before chaos.
const LAG_WINDOW_TICKS: usize = 60;

/// One tick of a recorded car against the port's prediction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScenarioRow {
    /// Ticks since the scenario's start frame.
    pub tick: usize,
    pub recorded: CarState,
    pub predicted: CarState,
}

impl ScenarioRow {
    /// Distance (uu) between recorded and predicted car positions.
    pub fn position_error(&self) -> f32 {
        self.recorded.position.distance(&self.predicted.position)
    }

    /// Distance (uu/s) between recorded and predicted car velocities.
    pub fn velocity_error(&self) -> f32 {
        self.recorded.velocity.distance(&self.predicted.velocity)
    }

    /// Distance (rad/s) between recorded and predicted spins.
    pub fn spin_error(&self) -> f32 {
        self.recorded
            .angular_velocity
            .distance(&self.predicted.angular_velocity)
    }
}

/// A scenario's recording lined up with the port's free-run prediction.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioComparison {
    /// Index of the capture frame matched to the scenario's start frame
    /// (before lag).
    pub start_index: usize,
    /// Ticks the recording lagged the predicted start (0 to `MAX_LAG_TICKS`).
    pub lag_ticks: usize,
    /// One row per tick both sides have.
    pub rows: Vec<ScenarioRow>,
    /// Ticks where the recorded car's input differs from the tape's, past
    /// the first (state-setting) tick: nonzero means the bot did not play
    /// the tape as written.
    pub input_mismatches: usize,
}

impl ScenarioComparison {
    /// First tick whose position error exceeds `threshold` uu.
    pub fn first_position_error_over(&self, threshold: f32) -> Option<usize> {
        self.rows
            .iter()
            .find(|row| row.position_error() > threshold)
            .map(|row| row.tick)
    }

    /// Largest position error (uu) over the run.
    pub fn max_position_error(&self) -> f32 {
        self.rows
            .iter()
            .map(ScenarioRow::position_error)
            .fold(0.0, f32::max)
    }

    /// Mean position error (uu) over the run.
    pub fn mean_position_error(&self) -> f32 {
        if self.rows.is_empty() {
            return 0.0;
        }
        self.rows
            .iter()
            .map(ScenarioRow::position_error)
            .sum::<f32>()
            / self.rows.len() as f32
    }
}

fn near(a: &Vec3, b: &Vec3, radius: f32) -> bool {
    a.distance(b) <= radius
}

fn inputs_match(recorded: &CarState, predicted: &CarState) -> bool {
    match (recorded.input, predicted.input) {
        (Some(r), Some(p)) => {
            let close =
                |a: Option<f32>, b: Option<f32>| (a.unwrap_or(0.0) - b.unwrap_or(0.0)).abs() < 0.01;
            (r.throttle - p.throttle).abs() < 0.01
                && (r.steer - p.steer).abs() < 0.01
                && close(r.pitch, p.pitch)
                && close(r.yaw, p.yaw)
                && close(r.roll, p.roll)
                && r.jump == p.jump
                && r.boost == p.boost
                && r.handbrake == p.handbrake
        }
        _ => true,
    }
}

/// Lines a scenario's recording (`recorded`, the whole capture) up with the
/// port's free-run prediction of it. The start is the first recorded frame
/// whose car is within `START_MATCH_RADIUS` of the scenario's start
/// location; the lag (0 to `MAX_LAG_TICKS`) is the one that best matches the
/// first `LAG_WINDOW_TICKS` ticks. A scenario with no start location cannot
/// be aligned.
pub fn compare_scenario(
    scenario: &Scenario,
    recorded: &[PhysicsFrame],
) -> Result<ScenarioComparison, IngestError> {
    let Some(location) = scenario.car.location else {
        return Err(IngestError::Malformed(
            "the scenario sets no car location, so the recording cannot be aligned".to_string(),
        ));
    };
    let location = Vec3::new(location[0], location[1], location[2]);
    let predicted = simulate_scenario(scenario);
    let start_index = recorded
        .iter()
        .position(|frame| {
            frame
                .cars
                .first()
                .is_some_and(|car| near(&car.position, &location, START_MATCH_RADIUS))
        })
        .ok_or_else(|| {
            IngestError::Malformed(format!(
                "no recorded frame has the car within {START_MATCH_RADIUS} uu of the scenario start"
            ))
        })?;

    let error_at_lag = |lag: usize| -> f32 {
        predicted
            .iter()
            .zip(recorded.iter().skip(start_index + lag))
            .take(LAG_WINDOW_TICKS)
            .filter_map(|(p, r)| Some(p.cars.first()?.position.distance(&r.cars.first()?.position)))
            .sum()
    };
    let lag_ticks = (0..=MAX_LAG_TICKS)
        .min_by(|a, b| error_at_lag(*a).total_cmp(&error_at_lag(*b)))
        .unwrap_or(0);

    let mut rows = Vec::new();
    let mut input_mismatches = 0;
    for (tick, (p, r)) in predicted
        .iter()
        .zip(recorded.iter().skip(start_index + lag_ticks))
        .enumerate()
    {
        let (Some(p), Some(r)) = (p.cars.first(), r.cars.first()) else {
            continue;
        };
        // The capture's input may belong to this tick or the previous one;
        // either counts as playing the tape.
        let next = predicted.get(tick + 1).and_then(|f| f.cars.first());
        if tick > 1 && !inputs_match(r, p) && !next.is_some_and(|n| inputs_match(r, n)) {
            input_mismatches += 1;
        }
        rows.push(ScenarioRow {
            tick,
            recorded: *r,
            predicted: *p,
        });
    }
    Ok(ScenarioComparison {
        start_index,
        lag_ticks,
        rows,
        input_mismatches,
    })
}

/// How far (fraction) a capture's mean frame interval may sit from the
/// game's 120 Hz tick and still be cut into a scenario, whose steps are
/// counted in 120 Hz ticks.
const SCENARIO_FRAME_RATE_TOLERANCE: f32 = 0.05;

/// Analog inputs closer than this (per axis) count as the same input when
/// run-length encoding a recording, the same tolerance `compare_scenario`
/// uses to say a recorded input played the tape.
const INPUT_MERGE_TOLERANCE: f32 = 0.01;

/// A scenario cut from the recorded frames whose timestamp lies in
/// `[from_secs, to_secs]` (`RB-VERIFY-003-FR-012`), so a human performance
/// can be replayed exactly by the tape bot: the first car's state and the
/// ball's state at the window's first frame as the start, no settling, and
/// the input recorded on each frame of the window as the tape, run-length
/// encoded (consecutive frames whose inputs agree within
/// `INPUT_MERGE_TOLERANCE` merge into one step).
///
/// The capture's mean frame interval must be within
/// `SCENARIO_FRAME_RATE_TOLERANCE` of `SCENARIO_TICK_SECS`, since the tape
/// counts 120 Hz ticks. A window with no frames, a first frame with no car,
/// or a frame without a recorded input (a replay-derived capture) is an
/// error.
pub fn scenario_from_capture(
    frames: &[PhysicsFrame],
    from_secs: f32,
    to_secs: f32,
    name: &str,
) -> Result<Scenario, IngestError> {
    check_frame_rate(frames)?;
    let window: Vec<&PhysicsFrame> = frames
        .iter()
        .filter(|frame| (from_secs..=to_secs).contains(&frame.timestamp_secs))
        .collect();
    let Some(first) = window.first() else {
        return Err(IngestError::Malformed(format!(
            "no recorded frame between {from_secs} s and {to_secs} s"
        )));
    };
    let Some(car) = first.cars.first() else {
        return Err(IngestError::Malformed(format!(
            "the frame at {} s has no car to start the scenario from",
            first.timestamp_secs
        )));
    };
    let mut steps: Vec<Step> = Vec::new();
    for frame in &window {
        let input = recorded_input(frame, car.player_id)?;
        match steps.last_mut() {
            Some(step) if inputs_agree(&step.input, &input) => step.ticks += 1,
            _ => steps.push(Step { ticks: 1, input }),
        }
    }
    Ok(Scenario {
        name: name.to_string(),
        settle_ticks: 0,
        car: CarStart {
            location: Some(array(&car.position)),
            rotation: Some(quat_to_rotator(car.rotation)),
            velocity: Some(array(&car.velocity)),
            angular_velocity: Some(array(&car.angular_velocity)),
            boost: Some(car.boost_amount),
        },
        ball: Some(BallStart {
            location: Some(array(&first.ball.position)),
            velocity: Some(array(&first.ball.velocity)),
            angular_velocity: Some(array(&first.ball.angular_velocity)),
        }),
        steps,
    })
}

fn array(v: &Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

/// Fails unless the capture's mean frame interval is the game's 120 Hz tick
/// within `SCENARIO_FRAME_RATE_TOLERANCE`.
fn check_frame_rate(frames: &[PhysicsFrame]) -> Result<(), IngestError> {
    let (Some(first), Some(last)) = (frames.first(), frames.last()) else {
        return Err(IngestError::Malformed(
            "the capture has no frames".to_string(),
        ));
    };
    if frames.len() < 2 {
        return Err(IngestError::Malformed(
            "the capture has one frame, so its frame rate is unknown".to_string(),
        ));
    }
    let mean = (last.timestamp_secs - first.timestamp_secs) / (frames.len() - 1) as f32;
    let off_by = (mean - SCENARIO_TICK_SECS).abs() / SCENARIO_TICK_SECS;
    // A NaN interval (equal first and last timestamps) is refused too.
    if off_by.is_nan() || off_by > SCENARIO_FRAME_RATE_TOLERANCE {
        return Err(IngestError::Malformed(format!(
            "the capture's mean frame interval is {:.3} ms ({:.1} Hz); a scenario needs 120 Hz frames (within {:.0}%)",
            mean * 1000.0,
            1.0 / mean,
            SCENARIO_FRAME_RATE_TOLERANCE * 100.0
        )));
    }
    Ok(())
}

/// The input recorded for car `player_id` on `frame`, as a scenario input.
fn recorded_input(frame: &PhysicsFrame, player_id: u32) -> Result<Input, IngestError> {
    let at = frame.timestamp_secs;
    let car = frame
        .cars
        .iter()
        .find(|car| car.player_id == player_id)
        .ok_or_else(|| {
            IngestError::Malformed(format!("the frame at {at} s has no car {player_id}"))
        })?;
    let Some(input) = car.input else {
        return Err(IngestError::Malformed(format!(
            "the frame at {at} s has no recorded input (a replay-derived capture cannot be cut into a scenario)"
        )));
    };
    let axis = |name: &str, value: Option<f32>| {
        value.ok_or_else(|| {
            IngestError::Malformed(format!("the frame at {at} s has no recorded {name} input"))
        })
    };
    Ok(Input {
        throttle: input.throttle,
        steer: input.steer,
        pitch: axis("pitch", input.pitch)?,
        yaw: axis("yaw", input.yaw)?,
        roll: axis("roll", input.roll)?,
        jump: input.jump,
        boost: input.boost,
        handbrake: input.handbrake,
    })
}

/// Whether two scenario inputs are the same press: analog axes within
/// `INPUT_MERGE_TOLERANCE`, buttons equal.
fn inputs_agree(a: &Input, b: &Input) -> bool {
    let close = |x: f32, y: f32| (x - y).abs() < INPUT_MERGE_TOLERANCE;
    close(a.throttle, b.throttle)
        && close(a.steer, b.steer)
        && close(a.pitch, b.pitch)
        && close(a.yaw, b.yaw)
        && close(a.roll, b.roll)
        && a.jump == b.jump
        && a.boost == b.boost
        && a.handbrake == b.handbrake
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use rb_domain::ControllerInput;

    fn scenario(name: &str) -> Scenario {
        let path = format!(
            "{}/../../tools/rb_tape_bot/scenarios/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        Scenario::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// A stand-in capture: some frames before the scenario starts, then the
    /// prediction itself, delayed by `lag` repeated start frames.
    fn fake_capture(scenario: &Scenario, before: usize, lag: usize) -> Vec<PhysicsFrame> {
        let predicted = simulate_scenario(scenario);
        let mut frames = Vec::new();
        for _ in 0..before {
            let mut frame = predicted[0].clone();
            frame.cars[0].position = Vec3::new(-3000.0, 3000.0, 17.0);
            frames.push(frame);
        }
        for _ in 0..lag {
            frames.push(predicted[0].clone());
        }
        frames.extend(predicted);
        frames
    }

    #[test]
    fn a_recording_equal_to_the_prediction_has_no_error() {
        let sc = scenario("prompt_dodge");
        let capture = fake_capture(&sc, 10, 0);
        let comparison = compare_scenario(&sc, &capture).unwrap();
        assert_eq!(comparison.start_index, 10);
        assert_eq!(comparison.lag_ticks, 0);
        assert_eq!(comparison.max_position_error(), 0.0);
        assert_eq!(comparison.input_mismatches, 0);
        assert_eq!(comparison.first_position_error_over(1.0), None);
        assert_eq!(comparison.rows.len() as u64, sc.total_ticks() + 2);
    }

    #[test]
    fn a_recording_that_lags_the_start_by_a_tick_is_aligned() {
        let sc = scenario("prompt_dodge");
        let capture = fake_capture(&sc, 5, 2);
        let comparison = compare_scenario(&sc, &capture).unwrap();
        // The two repeated start frames match from the first one, so the
        // start index is the first of them and the lag is 2.
        assert_eq!(comparison.start_index, 5);
        assert_eq!(comparison.lag_ticks, 2);
        assert_eq!(comparison.max_position_error(), 0.0);
    }

    #[test]
    fn a_wrong_velocity_shows_up_as_error_and_a_wrong_input_as_a_mismatch() {
        let sc = scenario("wavedash_mid");
        let mut capture = fake_capture(&sc, 3, 0);
        for frame in capture.iter_mut().skip(40) {
            frame.cars[0].position.y += 50.0;
            if let Some(input) = frame.cars[0].input.as_mut() {
                input.throttle = 1.0;
            }
        }
        let comparison = compare_scenario(&sc, &capture).unwrap();
        assert!(comparison.max_position_error() > 40.0);
        let first = comparison.first_position_error_over(10.0).unwrap();
        assert!((36..=40).contains(&first), "first error at tick {first}");
        assert!(comparison.input_mismatches > 100);
    }

    #[test]
    fn a_recording_without_the_start_or_a_scenario_without_a_location_is_an_error() {
        let sc = scenario("prompt_dodge");
        let mut capture = fake_capture(&sc, 4, 0);
        for frame in &mut capture {
            frame.cars[0].position = Vec3::new(500.0, 500.0, 17.0);
        }
        assert!(matches!(
            compare_scenario(&sc, &capture),
            Err(IngestError::Malformed(_))
        ));
        let bare = Scenario::from_json(r#"{ "name": "x", "steps": [ { "ticks": 1 } ] }"#).unwrap();
        assert!(matches!(
            compare_scenario(&bare, &capture),
            Err(IngestError::Malformed(_))
        ));
    }

    /// A 120 Hz capture of `ticks` frames: the car yawed a quarter turn at
    /// (100, 200, 17) moving +y with a boost tank of 42, the ball at
    /// (1, 2, 93.15) rising, and the input returned by `input_at(tick)`.
    fn synthetic_capture(
        ticks: usize,
        tick_secs: f32,
        input_at: impl Fn(usize) -> Option<ControllerInput>,
    ) -> Vec<PhysicsFrame> {
        let base = Scenario::from_json(
            r#"{ "name": "base",
                 "car": { "location": [100, 200, 17], "rotation": [0, 1.5707964, 0],
                          "velocity": [0, 500, 0], "angular_velocity": [0, 0, 0.3], "boost": 42 },
                 "ball": { "location": [1, 2, 93.15], "velocity": [0, 0, 10] },
                 "steps": [ { "ticks": 1 } ] }"#,
        )
        .unwrap()
        .initial_frame();
        (0..ticks)
            .map(|tick| {
                let mut frame = base.clone();
                frame.timestamp_secs = 10.0 + tick as f32 * tick_secs;
                frame.cars[0].position.y += tick as f32 * 500.0 * tick_secs;
                frame.cars[0].input = input_at(tick);
                frame
            })
            .collect()
    }

    fn pressing(throttle: f32, jump: bool, boost: bool) -> Option<ControllerInput> {
        Some(ControllerInput {
            throttle,
            steer: 0.0,
            pitch: Some(0.0),
            yaw: Some(0.0),
            roll: Some(0.0),
            jump,
            boost,
            handbrake: false,
        })
    }

    #[test]
    fn a_window_of_a_capture_becomes_a_scenario_with_a_run_length_tape() {
        // Ticks 0-9 throttle, 10-12 jump, 13-19 boost; throttle wobbles
        // within the merge tolerance on odd ticks.
        let capture = synthetic_capture(40, SCENARIO_TICK_SECS, |tick| match tick {
            0..=9 => pressing(if tick % 2 == 0 { 1.0 } else { 0.995 }, false, false),
            10..=12 => pressing(1.0, true, false),
            _ => pressing(1.0, false, true),
        });
        // Frames 5 to 19 inclusive (timestamps 10 + tick/120).
        let from = 10.0 + 4.5 * SCENARIO_TICK_SECS;
        let to = 10.0 + 19.5 * SCENARIO_TICK_SECS;
        let scenario = scenario_from_capture(&capture, from, to, "cut").unwrap();
        assert_eq!(scenario.name, "cut");
        assert_eq!(scenario.settle_ticks, 0);
        assert_eq!(scenario.total_ticks(), 15);
        assert_eq!(scenario.steps.len(), 3, "{:?}", scenario.steps);
        assert_eq!(scenario.steps[0].ticks, 5);
        assert!(!scenario.steps[0].input.jump);
        assert_eq!(scenario.steps[1].ticks, 3);
        assert!(scenario.steps[1].input.jump);
        assert_eq!(scenario.steps[2].ticks, 7);
        assert!(scenario.steps[2].input.boost);
        // The start is frame 5's state.
        let start = capture[5].cars[0];
        assert_eq!(scenario.car.location, Some(array(&start.position)));
        assert_eq!(scenario.car.velocity, Some([0.0, 500.0, 0.0]));
        assert_eq!(scenario.car.angular_velocity, Some([0.0, 0.0, 0.3]));
        assert_eq!(scenario.car.boost, Some(42.0));
        let rotation = scenario.car.rotation.unwrap();
        assert!(
            (rotation[1] - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "{rotation:?}"
        );
        assert!(rotation[0].abs() < 1e-3 && rotation[2].abs() < 1e-3);
        let ball = scenario.ball.unwrap();
        assert_eq!(ball.location, Some([1.0, 2.0, 93.15]));
        assert_eq!(ball.velocity, Some([0.0, 0.0, 10.0]));
        // Re-simulated, the scenario starts where the recording did, and
        // the JSON reads back equal.
        let frame = scenario.initial_frame();
        assert!(frame.cars[0].position.distance(&start.position) < 1e-3);
        assert!(frame.cars[0].rotation.angle_to(&start.rotation) < 1e-3);
        let text = scenario.to_json().unwrap();
        assert_eq!(Scenario::from_json(&text).unwrap(), scenario);
    }

    #[test]
    fn an_input_change_beyond_the_tolerance_starts_a_new_step() {
        let capture = synthetic_capture(6, SCENARIO_TICK_SECS, |tick| {
            pressing(if tick < 3 { 1.0 } else { 0.98 }, false, false)
        });
        let scenario = scenario_from_capture(&capture, 0.0, 100.0, "x").unwrap();
        assert_eq!(scenario.steps.len(), 2);
        assert_eq!(scenario.steps[0].ticks, 3);
        assert_eq!(scenario.steps[1].ticks, 3);
    }

    #[test]
    fn a_window_without_frames_or_input_is_an_error() {
        let capture = synthetic_capture(10, SCENARIO_TICK_SECS, |_| pressing(1.0, false, false));
        assert!(matches!(
            scenario_from_capture(&capture, 50.0, 60.0, "x"),
            Err(IngestError::Malformed(_))
        ));
        let no_input = synthetic_capture(10, SCENARIO_TICK_SECS, |tick| {
            if tick == 4 {
                None
            } else {
                pressing(1.0, false, false)
            }
        });
        let err = scenario_from_capture(&no_input, 0.0, 100.0, "x").unwrap_err();
        assert!(err.to_string().contains("no recorded input"), "{err}");
        let empty: Vec<PhysicsFrame> = Vec::new();
        assert!(matches!(
            scenario_from_capture(&empty, 0.0, 1.0, "x"),
            Err(IngestError::Malformed(_))
        ));
    }

    #[test]
    fn a_capture_that_is_not_120_hz_is_refused() {
        let slow = synthetic_capture(10, 1.0 / 60.0, |_| pressing(1.0, false, false));
        let err = scenario_from_capture(&slow, 0.0, 100.0, "x").unwrap_err();
        assert!(err.to_string().contains("120 Hz"), "{err}");
        // 4% off is accepted; 6% off is not.
        let near = synthetic_capture(10, SCENARIO_TICK_SECS * 1.04, |_| {
            pressing(1.0, false, false)
        });
        assert!(scenario_from_capture(&near, 0.0, 100.0, "x").is_ok());
        let far = synthetic_capture(10, SCENARIO_TICK_SECS * 1.06, |_| {
            pressing(1.0, false, false)
        });
        assert!(scenario_from_capture(&far, 0.0, 100.0, "x").is_err());
    }
}
