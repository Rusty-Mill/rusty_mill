//! Scenario runs (`RB-VERIFY-003-FR-010`, `FR-011`, `FR-012`): what the
//! port does with a scripted scenario, how that compares with its recording,
//! and a scenario cut from a window of a recording.

use rb_domain::{CarState, ControllerInput, IngestError, PhysicsFrame, Vec3};
use rb_env::Env;
use rb_scenario::{quat_to_rotator, BallStart, CarStart, Input, Scenario, Step};

/// Seconds per tick of a scenario run: the game's 120 Hz.
pub const SCENARIO_TICK_SECS: f32 = 1.0 / 120.0;

/// What the port does with a scenario (`docs/research/BOT-CAPTURE-PLAN.md`):
/// the start state, then one frame after each tick of the scenario's input
/// tape (settling and a final neutral tick included). The first frame is the
/// start state and carries no input; later frames carry the input that
/// produced them.
pub fn simulate_scenario(scenario: &Scenario) -> Vec<PhysicsFrame> {
    let mut env = Env::new();
    env.set_teams(&scenario.teams());
    let start = scenario.initial_frame();
    let mut frames = vec![start.clone()];
    env.reset(&start);
    for tick in 0..=scenario.total_ticks() {
        let inputs: Vec<ControllerInput> = (0..scenario.car_count())
            .map(|car| scenario.input_at_car(car, tick).to_controller_input())
            .collect();
        let mut frame = env.step(&inputs);
        frame.timestamp_secs = (tick + 1) as f32 * SCENARIO_TICK_SECS;
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

/// The cars after the first of a hivemind scenario (`rb_tape_hive`, one process playing every
/// car's tape) get their input in the game one tick before the first car's: with the second
/// car's recorded input read one tick later the port matches a boosted straight run to 0.3 uu
/// (`duel_870`: 54.3 against 54.0, where the same input a tick earlier led by 6.6 uu).
const HIVE_INPUT_LEAD: usize = 1;

/// Start speed (uu/s) above which the recording is not lagged against the port.
const FAST_START_SPEED: f32 = 300.0;

/// A car that moved less than this (uu) between two frames has not moved.
const STILL_FRAME_UU: f32 = 0.5;

/// Ticks used to choose the lag: the early trajectory, before chaos.
const LAG_WINDOW_TICKS: usize = 60;

/// One tick of a recorded car against the port's prediction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScenarioRow {
    /// Ticks since the scenario's start frame.
    pub tick: usize,
    pub recorded: CarState,
    pub predicted: CarState,
    /// The ball's recorded and predicted positions on the same tick
    /// (`RB-VERIFY-003-FR-015`).
    pub recorded_ball: Vec3,
    pub predicted_ball: Vec3,
    /// The same ball's recorded and predicted velocities (uu/s).
    pub recorded_ball_velocity: Vec3,
    pub predicted_ball_velocity: Vec3,
}

impl ScenarioRow {
    /// Distance (uu) between recorded and predicted car positions.
    pub fn position_error(&self) -> f32 {
        self.recorded.position.distance(&self.predicted.position)
    }

    /// Distance (uu) between recorded and predicted ball positions.
    pub fn ball_error(&self) -> f32 {
        self.recorded_ball.distance(&self.predicted_ball)
    }

    /// Distance (uu/s) between recorded and predicted car velocities.
    pub fn velocity_error(&self) -> f32 {
        self.recorded.velocity.distance(&self.predicted.velocity)
    }

    /// Difference (boost units, 0 to 100) between recorded and predicted tanks.
    pub fn boost_error(&self) -> f32 {
        (self.recorded.boost_amount - self.predicted.boost_amount).abs()
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
    /// One row per tick both sides have; ticks missing from the capture
    /// (holes) have no row.
    pub rows: Vec<ScenarioRow>,
    /// Ticks where the recorded car's input differs from the tape's, past
    /// the first (state-setting) tick: nonzero means the bot did not play
    /// the tape as written.
    pub input_mismatches: usize,
    /// Rows of every car after the first, one list per car (a bump or
    /// demolition scenario, `RB-VERIFY-003-FR-018`): the tick and the
    /// recorded and predicted state.
    pub other_rows: Vec<Vec<(usize, CarState, CarState)>>,
    /// For a recorded-input replay (`compare_scenario_recorded`): which
    /// recorded input fed each step, 0 for the previous tick's, 1 for the
    /// same tick's. `None` for a tape replay.
    pub input_offset: Option<usize>,
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

    /// First tick whose ball position error exceeds `threshold` uu.
    pub fn first_ball_error_over(&self, threshold: f32) -> Option<usize> {
        self.rows
            .iter()
            .find(|row| row.ball_error() > threshold)
            .map(|row| row.tick)
    }

    /// Largest ball position error (uu) over the run.
    pub fn max_ball_error(&self) -> f32 {
        self.rows
            .iter()
            .map(ScenarioRow::ball_error)
            .fold(0.0, f32::max)
    }

    /// Mean ball position error (uu) over the run.
    pub fn mean_ball_error(&self) -> f32 {
        if self.rows.is_empty() {
            return 0.0;
        }
        self.rows.iter().map(ScenarioRow::ball_error).sum::<f32>() / self.rows.len() as f32
    }

    /// Largest boost error over the run (boost units).
    pub fn max_boost_error(&self) -> f32 {
        self.rows
            .iter()
            .map(ScenarioRow::boost_error)
            .fold(0.0, f32::max)
    }

    /// Mean boost error over the run (boost units).
    pub fn mean_boost_error(&self) -> f32 {
        if self.rows.is_empty() {
            return 0.0;
        }
        self.rows.iter().map(ScenarioRow::boost_error).sum::<f32>() / self.rows.len() as f32
    }

    /// Position error (uu) of every row of car `index` after the first
    /// (`index` 0 is the second car).
    fn other_errors(&self, index: usize) -> impl Iterator<Item = f32> + '_ {
        self.other_rows
            .get(index)
            .into_iter()
            .flatten()
            .map(|(_, recorded, predicted)| recorded.position.distance(&predicted.position))
    }

    /// Mean position error (uu) of the car after the first at `index`.
    pub fn mean_other_error(&self, index: usize) -> f32 {
        let (sum, count) = self
            .other_errors(index)
            .fold((0.0, 0usize), |(sum, count), e| (sum + e, count + 1));
        sum / count.max(1) as f32
    }

    /// Largest position error (uu) of the car after the first at `index`.
    pub fn max_other_error(&self, index: usize) -> f32 {
        self.other_errors(index).fold(0.0, f32::max)
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

/// Whole 120 Hz ticks from the start frame's timestamp to `secs`.
fn tick_after(start_secs: f32, secs: f32) -> usize {
    let ticks = ((secs - start_secs) / SCENARIO_TICK_SECS).round();
    if ticks.is_sign_negative() {
        0
    } else {
        ticks as usize
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
/// location. Every later recorded frame is placed by its timestamp, in
/// 120 Hz ticks after that frame, so a hole in the capture (the plugin
/// drops a few ticks per run, `RB-RESEARCH-O009`) skips those ticks instead
/// of shifting the rest of the comparison; the lag (0 to `MAX_LAG_TICKS`) is
/// the one that best matches the first `LAG_WINDOW_TICKS` ticks. A scenario
/// with no start location cannot be aligned.
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

    // Each recorded frame after the start, with its tick count since the
    // start frame read from its timestamp (not its row number: captures
    // have holes).
    let start_secs = recorded[start_index].timestamp_secs;
    let recorded_ticks: Vec<(usize, &PhysicsFrame)> = recorded[start_index..]
        .iter()
        .map(|frame| (tick_after(start_secs, frame.timestamp_secs), frame))
        .take_while(|(tick, _)| *tick < predicted.len() + MAX_LAG_TICKS)
        .collect();

    // Mean, not sum: a hole in the window must not favour a lag.
    let error_at_lag = |lag: usize| -> f32 {
        let errors: Vec<f32> = recorded_ticks
            .iter()
            .filter(|(tick, _)| *tick >= lag && *tick - lag < LAG_WINDOW_TICKS)
            .filter_map(|(tick, r)| {
                let frame = predicted.get(*tick - lag)?;
                let p = frame.cars.first()?;
                // Car and ball both: a scenario whose car stands still (a
                // ball-only probe) would otherwise pick its lag at random.
                // Every other car too (a bump scenario).
                let others: f32 = frame
                    .cars
                    .iter()
                    .zip(r.cars.iter())
                    .skip(1)
                    .map(|(p, r)| p.position.distance(&r.position))
                    .sum();
                Some(
                    p.position.distance(&r.cars.first()?.position)
                        + frame.ball.position.distance(&r.ball.position)
                        + others,
                )
            })
            .collect();
        if errors.is_empty() {
            return f32::INFINITY;
        }
        errors.iter().sum::<f32>() / errors.len() as f32
    };
    let lag_ticks = (0..=MAX_LAG_TICKS)
        .min_by(|a, b| error_at_lag(*a).total_cmp(&error_at_lag(*b)))
        .unwrap_or(0);

    let mut rows = Vec::new();
    let mut other_rows: Vec<Vec<(usize, CarState, CarState)>> =
        vec![Vec::new(); scenario.car_count().saturating_sub(1)];
    let mut input_mismatches = 0;
    for (recorded_tick, r) in recorded_ticks {
        let Some(tick) = recorded_tick.checked_sub(lag_ticks) else {
            continue;
        };
        let Some(predicted_frame) = predicted.get(tick) else {
            continue;
        };
        let (Some(p), Some(car)) = (predicted_frame.cars.first(), r.cars.first()) else {
            continue;
        };
        let (recorded_ball, predicted_ball) = (r.ball.position, predicted_frame.ball.position);
        for (index, rows) in other_rows.iter_mut().enumerate() {
            if let (Some(p), Some(r)) = (predicted_frame.cars.get(index + 1), r.cars.get(index + 1))
            {
                rows.push((tick, *r, *p));
            }
        }
        let r = car;
        // The capture's input may belong to this tick or the previous one;
        // either counts as playing the tape.
        let next = predicted.get(tick + 1).and_then(|f| f.cars.first());
        // The last frame is one past the tape, so it holds no input to play.
        let on_tape = (tick as u64) <= scenario.total_ticks();
        if on_tape && tick > 1 && !inputs_match(r, p) && !next.is_some_and(|n| inputs_match(r, n)) {
            input_mismatches += 1;
        }
        rows.push(ScenarioRow {
            tick,
            recorded: *r,
            predicted: *p,
            recorded_ball,
            predicted_ball,
            recorded_ball_velocity: Vec3::ZERO,
            predicted_ball_velocity: Vec3::ZERO,
        });
    }
    Ok(ScenarioComparison {
        start_index,
        lag_ticks,
        rows,
        other_rows,
        input_mismatches,
        input_offset: None,
    })
}

/// What the port does from `start` when fed `inputs`, one per tick: the
/// start frame, then one frame after each input.
fn simulate_with_inputs(
    start: &PhysicsFrame,
    teams: &[u32],
    inputs: &[Vec<ControllerInput>],
    respawn_points: &[Option<usize>],
) -> Vec<PhysicsFrame> {
    let mut env = Env::new();
    env.set_teams(teams);
    let mut frames = vec![start.clone()];
    env.reset(start);
    for (car, point) in respawn_points.iter().enumerate() {
        env.set_respawn_point(car, *point);
    }
    for (tick, input) in inputs.iter().enumerate() {
        let mut frame = env.step(input);
        frame.timestamp_secs = (tick + 1) as f32 * SCENARIO_TICK_SECS;
        frames.push(frame);
    }
    frames
}

/// How many frames at the start of `frames` show the first car where the frame before did:
/// a state set the recording shows repeated, which a lag can absorb.
fn leading_still_frames(frames: &[(usize, &PhysicsFrame)]) -> usize {
    frames
        .windows(2)
        .take_while(
            |pair| match (pair[0].1.cars.first(), pair[1].1.cars.first()) {
                (Some(a), Some(b)) => a.position.distance(&b.position) < STILL_FRAME_UU,
                _ => false,
            },
        )
        .count()
}

/// The spawn point each car came back at in `frames`, if it was demolished: the point
/// nearest to where it reappears after being out of the capture. The game's pick is
/// random, so a replay is given the recording's (`PhysicsWorld::set_respawn_point`).
fn recorded_respawn_points(frames: &[(usize, &PhysicsFrame)], cars: usize) -> Vec<Option<usize>> {
    (0..cars)
        .map(|car| {
            let mut seen = false;
            let mut gone = false;
            for (_, frame) in frames {
                match frame.cars.iter().find(|c| c.player_id as usize == car) {
                    Some(state) if gone => {
                        return Some(rb_physics_bullet::respawn::nearest_spawn_point(
                            state.position.x,
                            state.position.y,
                        ));
                    }
                    Some(_) => seen = true,
                    None if seen => gone = true,
                    None => {}
                }
            }
            None
        })
        .collect()
}

/// `compare_scenario`, but the port is fed the input the recording shows
/// instead of the tape (`RB-VERIFY-003-FR-016`). The tape bot's input
/// reaches the game a variable one to four ticks after the state set (the
/// capture has a hole there, `RB-RESEARCH-O009`), so against the tape every
/// scenario carries that timing error; against the recorded input what is
/// left is physics. A tick the capture lacks takes the last recorded input.
/// The recorded input may belong to this tick or the previous one, so both
/// are tried and the one with the smaller error is used.
pub fn compare_scenario_recorded(
    scenario: &Scenario,
    recorded: &[PhysicsFrame],
) -> Result<ScenarioComparison, IngestError> {
    let Some(location) = scenario.car.location else {
        return Err(IngestError::Malformed(
            "the scenario sets no car location, so the recording cannot be aligned".to_string(),
        ));
    };
    let location = Vec3::new(location[0], location[1], location[2]);
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
    let start_secs = recorded[start_index].timestamp_secs;
    let limit = scenario.total_ticks() as usize + 2;
    let recorded_ticks: Vec<(usize, &PhysicsFrame)> = recorded[start_index..]
        .iter()
        .map(|frame| (tick_after(start_secs, frame.timestamp_secs), frame))
        .take_while(|(tick, _)| *tick <= limit)
        .collect();

    // Every car's recorded input of each tick, holes filled with the last one
    // seen.
    let cars = scenario.car_count();
    let filled: Vec<Vec<ControllerInput>> = (0..cars)
        .map(|car| {
            let mut by_tick: Vec<Option<ControllerInput>> = vec![None; limit + 1];
            for (tick, frame) in &recorded_ticks {
                if let Some(input) = frame.cars.get(car).and_then(|c| c.input) {
                    by_tick[*tick] = Some(input);
                }
            }
            let mut last = ControllerInput::default();
            by_tick
                .iter()
                .map(|input| {
                    last = input.unwrap_or(last);
                    last
                })
                .collect()
        })
        .collect();

    // The tank the recording shows at the start, not the scenario's: the game's
    // state set can leave pads taken on its way (`padp_*`), and a recorded-input
    // replay scores the tank (`boost error`) from the recording's own start.
    let mut start = scenario.initial_frame();
    for (car, seen) in start.cars.iter_mut().zip(&recorded[start_index].cars) {
        car.boost_amount = seen.boost_amount;
    }
    let respawn_points = recorded_respawn_points(&recorded_ticks, cars);
    let teams = scenario.teams();
    type Rows = (Vec<ScenarioRow>, Vec<Vec<(usize, CarState, CarState)>>);
    // `lag`: recorded tick t is the port's tick t - lag (the state set can
    // land a tick or two after the frame the recording first shows it);
    // `offset`: which recorded input fed the step; `ticks` bounds the run.
    let run = |lag: usize, offset: usize, ticks: usize| -> (Rows, f32) {
        let inputs: Vec<Vec<ControllerInput>> = (0..ticks.min(limit))
            .map(|tick| {
                filled
                    .iter()
                    .enumerate()
                    .map(|(index, car)| {
                        let lead = if index > 0 { HIVE_INPUT_LEAD } else { 0 };
                        car[(tick + lag + offset + lead).min(limit)]
                    })
                    .collect()
            })
            .collect();
        let predicted = simulate_with_inputs(&start, &teams, &inputs, &respawn_points);
        let mut rows = Vec::new();
        let mut other_rows: Vec<Vec<(usize, CarState, CarState)>> =
            vec![Vec::new(); cars.saturating_sub(1)];
        let mut total = 0.0;
        for (recorded_tick, r) in &recorded_ticks {
            let Some(tick) = recorded_tick.checked_sub(lag) else {
                continue;
            };
            let (Some(p), Some(car)) = (predicted.get(tick), r.cars.first()) else {
                continue;
            };
            let Some(pc) = p.cars.first() else {
                continue;
            };
            total +=
                pc.position.distance(&car.position) + p.ball.position.distance(&r.ball.position);
            for (index, rows) in other_rows.iter_mut().enumerate() {
                if let (Some(pc), Some(rc)) = (p.cars.get(index + 1), r.cars.get(index + 1)) {
                    total += pc.position.distance(&rc.position);
                    rows.push((tick, *rc, *pc));
                }
            }
            rows.push(ScenarioRow {
                tick,
                recorded: *car,
                predicted: *pc,
                recorded_ball: r.ball.position,
                predicted_ball: p.ball.position,
                recorded_ball_velocity: r.ball.velocity,
                predicted_ball_velocity: p.ball.velocity,
            });
        }
        let mean = total / rows.len().max(1) as f32;
        ((rows, other_rows), mean)
    };
    // Lag first (offset 0), then offset at that lag, each on the whole run:
    // the early trajectory alone mis-picks a falling start (`pogo`).
    let mut best = (run(0, 0, limit), 0, 0);
    // A car already moving at the start sits at the state-set frame in the recording's
    // first matching frame and moves on every frame after it, so a lag would only add its
    // speed times the lag as a constant offset (19 uu per tick at 2300 uu/s). A fast start
    // is lagged only by the leading frames in which the recorded car has not moved (a
    // state set that shows up repeated).
    let start_speed = scenario
        .car
        .velocity
        .map_or(0.0, |v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt());
    let max_lag = if start_speed > FAST_START_SPEED {
        leading_still_frames(&recorded_ticks).min(MAX_LAG_TICKS)
    } else {
        MAX_LAG_TICKS
    };
    for lag in 1..=max_lag {
        let trial = run(lag, 0, limit);
        if trial.1 < (best.0).1 {
            best = (trial, lag, 0);
        }
    }
    let lag_ticks = best.1;
    let shifted = run(lag_ticks, 1, limit);
    if shifted.1 < (best.0).1 {
        best = (shifted, lag_ticks, 1);
    }
    // `RB_FORCE_ALIGN=lag,offset` overrides the whole-run pick (a diagnostic: the pick can
    // land a tick early on a tape whose late events dominate the mean).
    if let Ok(force) = std::env::var("RB_FORCE_ALIGN") {
        let parts: Vec<usize> = force.split(',').filter_map(|p| p.parse().ok()).collect();
        if let [lag, offset] = parts[..] {
            best = (run(lag, offset, limit), lag, offset);
        }
    }
    let (((rows, other_rows), _), _, input_offset) = best;
    Ok(ScenarioComparison {
        start_index,
        lag_ticks,
        rows,
        other_rows,
        input_mismatches: 0,
        input_offset: Some(input_offset),
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
        others: Vec::new(),
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
        // A capture's rows are 1/120 s apart whatever they show.
        for (i, frame) in frames.iter_mut().enumerate() {
            frame.timestamp_secs = i as f32 * SCENARIO_TICK_SECS;
        }
        frames
    }

    /// `RB-VERIFY-003-FR-016`: fed the recorded input, a recording whose bot
    /// started the tape late compares at zero error where the tape replay
    /// carries the whole delay.
    #[test]
    fn a_recorded_input_replay_absorbs_a_late_tape_start() {
        let sc = Scenario::from_json(
            r#"{ "name": "boosting", "settle_ticks": 0,
                 "car": { "location": [0, -4500, 17], "rotation": [0, 1.5708, 0],
                          "velocity": [0, 1400, 0], "boost": 100 },
                 "steps": [ { "ticks": 90, "throttle": 1, "boost": true } ] }"#,
        )
        .unwrap();
        let late = {
            let mut delayed = sc.clone();
            delayed.settle_ticks += 3;
            delayed
        };
        // The "game" ran the tape three ticks late; the recording shows it.
        let capture = fake_capture(&late, 4, 0);
        let against_tape = compare_scenario(&sc, &capture).unwrap();
        let against_recorded = compare_scenario_recorded(&sc, &capture).unwrap();
        assert!(
            against_tape.max_position_error() > 3.0,
            "the tape replay carries the delay: {}",
            against_tape.max_position_error()
        );
        assert!(
            against_recorded.max_position_error() < 0.5,
            "{}",
            against_recorded.max_position_error()
        );
        assert!(against_recorded.input_offset.is_some());
    }

    /// A recording that shows the start state a tick or two late is aligned by
    /// the recorded-input replay as it is by the tape replay.
    #[test]
    fn a_recorded_input_replay_finds_the_start_lag() {
        let sc = scenario("wavedash_mid");
        let capture = fake_capture(&sc, 4, 2);
        let comparison = compare_scenario_recorded(&sc, &capture).unwrap();
        assert_eq!(comparison.lag_ticks, 2);
        assert!(
            comparison.max_position_error() < 0.5,
            "{}",
            comparison.max_position_error()
        );
    }

    #[test]
    fn a_hole_in_the_recording_skips_those_ticks_instead_of_shifting_the_rest() {
        let sc = scenario("prompt_dodge");
        let mut capture = fake_capture(&sc, 4, 0);
        // The plugin dropped ticks 2, 3 and 4 after the start frame.
        capture.drain(6..9);
        let comparison = compare_scenario(&sc, &capture).unwrap();
        assert_eq!(comparison.start_index, 4);
        assert_eq!(comparison.lag_ticks, 0);
        assert_eq!(comparison.max_position_error(), 0.0);
        assert_eq!(comparison.rows.len() as u64, sc.total_ticks() + 2 - 3);
        let ticks: Vec<usize> = comparison.rows.iter().map(|r| r.tick).take(4).collect();
        assert_eq!(ticks, vec![0, 1, 5, 6]);
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

    const TWO_CAR_SCENARIO: &str = r#"{
        "name": "head-on",
        "settle_ticks": 0,
        "car": { "location": [0, -600, 17], "rotation": [0, 1.5708, 0],
                 "velocity": [0, 1000, 0], "boost": 0 },
        "steps": [ { "ticks": 90, "throttle": 1 } ],
        "others": [ { "car": { "location": [0, 600, 17], "rotation": [0, -1.5708, 0],
                               "velocity": [0, -1000, 0], "boost": 0 },
                      "steps": [ { "ticks": 90, "throttle": 1 } ] } ]
    }"#;

    /// `RB-VERIFY-003-FR-018`: every car is simulated and scored. Two cars
    /// meeting head-on bounce off each other; a recording of that matches its
    /// own prediction on both cars, and one whose second car is 40 uu off
    /// shows it on the second car and not on the first.
    #[test]
    fn a_second_car_is_simulated_and_scored_beside_the_first() {
        let sc = Scenario::from_json(TWO_CAR_SCENARIO).unwrap();
        let frames = simulate_scenario(&sc);
        assert_eq!(frames[0].cars.len(), 2);
        assert!(
            frames.last().unwrap().cars[1].velocity.y > -500.0,
            "the second car was slowed by the collision"
        );
        let capture = fake_capture(&sc, 3, 0);
        let comparison = compare_scenario(&sc, &capture).unwrap();
        assert_eq!(comparison.other_rows.len(), 1);
        assert_eq!(comparison.max_other_error(0), 0.0);
        let mut off = capture.clone();
        for frame in off.iter_mut().skip(10) {
            frame.cars[1].position.x += 40.0;
        }
        let comparison = compare_scenario(&sc, &off).unwrap();
        assert!((comparison.max_other_error(0) - 40.0).abs() < 1e-3);
        assert_eq!(comparison.max_position_error(), 0.0);
        // The recorded-input replay reads each car's own input.
        let replay = compare_scenario_recorded(&sc, &capture).unwrap();
        assert_eq!(replay.max_other_error(0), 0.0);
    }

    /// `RB-VERIFY-003-FR-015`: a ball-only scenario (the car stands still) is
    /// lined up by its ball, not by an arbitrary lag.
    #[test]
    fn a_ball_only_scenario_is_aligned_by_the_ball() {
        let sc = Scenario::from_json(
            r#"{ "name": "ball only", "settle_ticks": 0,
                 "car": { "location": [-3500, -4000, 17], "rotation": [0, 1.5708, 0] },
                 "ball": { "location": [-2000, 0, 500], "velocity": [3500, 500, 200] },
                 "steps": [ { "ticks": 90 } ] }"#,
        )
        .unwrap();
        for lag in 0..=2 {
            let capture = fake_capture(&sc, 3, lag);
            let comparison = compare_scenario(&sc, &capture).unwrap();
            assert_eq!(comparison.lag_ticks, lag, "lag {lag}");
            assert!(comparison.max_ball_error() < 1.0, "lag {lag}");
        }
    }

    /// `RB-VERIFY-003-FR-015`: the ball is scored beside the car.
    #[test]
    fn a_wrong_ball_shows_up_as_ball_error_and_not_car_error() {
        let sc = scenario("prompt_dodge");
        let mut capture = fake_capture(&sc, 3, 0);
        assert_eq!(
            compare_scenario(&sc, &capture).unwrap().max_ball_error(),
            0.0
        );
        for frame in capture.iter_mut().skip(23) {
            frame.ball.position.x += 30.0;
        }
        let comparison = compare_scenario(&sc, &capture).unwrap();
        assert_eq!(comparison.max_position_error(), 0.0);
        assert!((comparison.max_ball_error() - 30.0).abs() < 1e-3);
        assert_eq!(comparison.first_ball_error_over(10.0), Some(20));
        assert_eq!(comparison.first_ball_error_over(100.0), None);
        assert!(comparison.mean_ball_error() > 0.0);
    }

    #[test]
    fn the_frame_past_the_tape_is_not_an_input_mismatch() {
        let sc = scenario("prompt_dodge");
        let mut capture = fake_capture(&sc, 10, 0);
        // A held steer on the last frame, where the tape has run out.
        if let Some(car) = capture.last_mut().and_then(|f| f.cars.first_mut()) {
            if let Some(input) = car.input.as_mut() {
                input.steer = -1.0;
            }
        }
        assert_eq!(compare_scenario(&sc, &capture).unwrap().input_mismatches, 0);
    }

    /// A car already moving at the start moves on every recorded frame, so the recorded-input
    /// replay does not lag it: each lag tick would be a constant offset of its speed over 120
    /// (19 uu a tick at 2300 uu/s) in every row. A start the recording shows repeated is
    /// still lagged (`a_recorded_input_replay_finds_the_start_lag`).
    #[test]
    fn only_a_start_shown_repeated_has_leading_still_frames() {
        let path = format!(
            "{}/../../tools/rb_tape_bot/experiments/bumpf_flip.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let fast = Scenario::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let ticks = |frames: &[PhysicsFrame]| -> usize {
            let numbered: Vec<(usize, &PhysicsFrame)> = frames.iter().enumerate().collect();
            leading_still_frames(&numbered)
        };
        // The fast car moves on from the first frame it is shown.
        assert_eq!(ticks(&fake_capture(&fast, 0, 0)), 0);
        // A start shown twice more before the car moves is two still frames.
        assert_eq!(ticks(&fake_capture(&fast, 0, 2)), 2);
        // And the replay of the repeated start still finds its lag.
        let capture = fake_capture(&fast, 5, 2);
        assert_eq!(
            compare_scenario_recorded(&fast, &capture)
                .unwrap()
                .lag_ticks,
            2
        );
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
