//! Scenario runs (`RB-VERIFY-003-FR-010`, `FR-011`): what the port does with
//! a scripted scenario, and how that compares with its recording.

use rb_domain::{CarState, IngestError, PhysicsFrame, Vec3};
use rb_physics_bullet::PhysicsWorld;
use rb_scenario::Scenario;

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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

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
}
