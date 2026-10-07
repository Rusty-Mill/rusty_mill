//! Scenario files for scripted mechanic captures: an initial game state plus
//! a run-length input timeline, replayed one input per game packet by
//! `tools/rb_tape_bot` and simulated by `rb-verify --scenario`
//! (`docs/research/BOT-CAPTURE-PLAN.md`).
//!
//! Pure data and lookup, no RLBot types, so it is testable without the game.

use rb_domain::{BallState, CarState, ControllerInput, PhysicsFrame, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// One tick of controller input, the same fields as the capture format's
/// `input` object (ADR-0005).
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Input {
    pub throttle: f32,
    pub steer: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub jump: bool,
    pub boost: bool,
    pub handbrake: bool,
}

/// `ticks` consecutive packets of the same input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Step {
    pub ticks: u64,
    #[serde(flatten)]
    pub input: Input,
}

/// A car's initial state; absent fields are left as the game has them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct CarStart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<[f32; 3]>,
    /// `[pitch, yaw, roll]` in radians, RLBot's convention.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub velocity: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angular_velocity: Option<[f32; 3]>,
    /// 0 to 100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boost: Option<f32>,
}

/// A ball's initial state; absent fields are left as the game has them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct BallStart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub velocity: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angular_velocity: Option<[f32; 3]>,
}

/// A further car in a scenario (a bump or demolition test): where it starts
/// and what it presses, on the same clock as the first car's tape (the first
/// car's `settle_ticks` apply to it too). No steps leaves it neutral.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct OtherCar {
    pub car: CarStart,
    pub steps: Vec<Step>,
}

/// A scenario: where the car (and ball) start and what the car presses.
/// More cars are listed under `others`; the tape bot drives them all from one
/// hivemind process so their tapes share a clock.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Scenario {
    pub name: String,
    /// Neutral packets after the start state is set and before the first
    /// step, so the car can settle (a car set on the floor is still
    /// dropping or springing for a few ticks).
    #[serde(default)]
    pub settle_ticks: u64,
    #[serde(default)]
    pub car: CarStart,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ball: Option<BallStart>,
    pub steps: Vec<Step>,
    /// Cars after the first, in car-index order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<OtherCar>,
}

/// Why a scenario could not be read.
#[derive(Debug)]
pub enum ScenarioError {
    /// Not valid JSON for the schema.
    Parse(serde_json::Error),
    /// No steps: nothing to play.
    Empty,
    /// The scenario could not be written as JSON.
    Write(serde_json::Error),
}

impl std::fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScenarioError::Parse(err) => write!(f, "invalid scenario JSON: {err}"),
            ScenarioError::Empty => write!(f, "scenario has no steps"),
            ScenarioError::Write(err) => write!(f, "cannot write scenario JSON: {err}"),
        }
    }
}

impl std::error::Error for ScenarioError {}

impl Scenario {
    /// Reads a scenario from JSON text.
    pub fn from_json(text: &str) -> Result<Scenario, ScenarioError> {
        let scenario: Scenario = serde_json::from_str(text).map_err(ScenarioError::Parse)?;
        if scenario.steps.iter().all(|step| step.ticks == 0) {
            return Err(ScenarioError::Empty);
        }
        Ok(scenario)
    }

    /// Writes the scenario as pretty JSON that [`Scenario::from_json`]
    /// reads back equal. Absent start fields are left out, as a hand-written
    /// file would.
    pub fn to_json(&self) -> Result<String, ScenarioError> {
        serde_json::to_string_pretty(self).map_err(ScenarioError::Write)
    }

    /// Cars in the scenario: the first and every other.
    pub fn car_count(&self) -> usize {
        1 + self.others.len()
    }

    /// Packets of input the scenario plays, settling included: until the
    /// longest car's last step.
    pub fn total_ticks(&self) -> u64 {
        let longest = self
            .others
            .iter()
            .map(|other| ticks_of(&other.steps))
            .chain(std::iter::once(ticks_of(&self.steps)))
            .max()
            .unwrap_or(0);
        self.settle_ticks + longest
    }

    /// The first car's input for packet `tick`, counted from the packet after
    /// the start state was set: neutral while settling and after the last
    /// step.
    pub fn input_at(&self, tick: u64) -> Input {
        self.input_at_car(0, tick)
    }

    /// Car `car`'s input for packet `tick` (see [`Scenario::input_at`]);
    /// neutral for a car the scenario does not have.
    pub fn input_at_car(&self, car: usize, tick: u64) -> Input {
        let steps = match car {
            0 => &self.steps,
            n => match self.others.get(n - 1) {
                Some(other) => &other.steps,
                None => return Input::default(),
            },
        };
        let Some(mut remaining) = tick.checked_sub(self.settle_ticks) else {
            return Input::default();
        };
        for step in steps {
            if remaining < step.ticks {
                return step.input;
            }
            remaining -= step.ticks;
        }
        Input::default()
    }
}

fn ticks_of(steps: &[Step]) -> u64 {
    steps.iter().map(|step| step.ticks).sum()
}

impl Input {
    /// The domain's controller input for this tick.
    pub fn to_controller_input(self) -> ControllerInput {
        ControllerInput {
            throttle: self.throttle,
            steer: self.steer,
            pitch: Some(self.pitch),
            yaw: Some(self.yaw),
            roll: Some(self.roll),
            jump: self.jump,
            boost: self.boost,
            handbrake: self.handbrake,
        }
    }
}

/// Where a scenario puts a car that gives no location, and a ball that is
/// absent: the car on the floor at the origin, the ball far away and still.
const DEFAULT_CAR_LOCATION: [f32; 3] = [0.0, 0.0, 17.0];
const DEFAULT_BALL_LOCATION: [f32; 3] = [3000.0, 3000.0, 93.15];

fn vec3(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}

/// A rotation from RLBot's `[pitch, yaw, roll]` (radians) as a quaternion.
///
/// Pitch is nose up, yaw 0 faces +x, and the car's local +y axis is the
/// "left" of RLUtilities' Euler convention. Checked by round trip against a
/// recorded quaternion (`test2.jsonl` 18.308 s, see the tests).
pub fn rotator_to_quat(rotation: [f32; 3]) -> Quat {
    let (sp, cp) = rotation[0].sin_cos();
    let (sy, cy) = rotation[1].sin_cos();
    let (sr, cr) = rotation[2].sin_cos();
    // Matrix columns: forward, local y, up.
    let forward = [cp * cy, cp * sy, sp];
    let side = [cy * sp * sr - cr * sy, sy * sp * sr + cr * cy, -cp * sr];
    let up = [-cr * cy * sp - sr * sy, -cr * sy * sp + sr * cy, cp * cr];
    let m = [
        [forward[0], side[0], up[0]],
        [forward[1], side[1], up[1]],
        [forward[2], side[2], up[2]],
    ];
    let trace = m[0][0] + m[1][1] + m[2][2];
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        Quat {
            w: 0.25 * s,
            x: (m[2][1] - m[1][2]) / s,
            y: (m[0][2] - m[2][0]) / s,
            z: (m[1][0] - m[0][1]) / s,
        }
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        Quat {
            w: (m[2][1] - m[1][2]) / s,
            x: 0.25 * s,
            y: (m[0][1] + m[1][0]) / s,
            z: (m[0][2] + m[2][0]) / s,
        }
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        Quat {
            w: (m[0][2] - m[2][0]) / s,
            x: (m[0][1] + m[1][0]) / s,
            y: 0.25 * s,
            z: (m[1][2] + m[2][1]) / s,
        }
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        Quat {
            w: (m[1][0] - m[0][1]) / s,
            x: (m[0][2] + m[2][0]) / s,
            y: (m[1][2] + m[2][1]) / s,
            z: 0.25 * s,
        }
    }
}

/// The inverse of [`rotator_to_quat`]: a quaternion as RLBot's
/// `[pitch, yaw, roll]` (radians), same axes and signs, so the two round
/// trip (up to the quaternion's sign). With the nose straight up or down
/// (pitch at ±90°) yaw and roll are not separable; roll is reported as 0
/// and yaw carries the whole turn, which still reproduces the quaternion.
pub fn quat_to_rotator(q: Quat) -> [f32; 3] {
    let (x, y, z, w) = (q.x, q.y, q.z, q.w);
    // The same matrix `rotator_to_quat` builds: columns forward, side, up.
    let forward_x = 1.0 - 2.0 * (y * y + z * z);
    let forward_y = 2.0 * (x * y + w * z);
    let forward_z = 2.0 * (x * z - w * y);
    let side_x = 2.0 * (x * y - w * z);
    let side_y = 1.0 - 2.0 * (x * x + z * z);
    let side_z = 2.0 * (y * z + w * x);
    let up_z = 1.0 - 2.0 * (x * x + y * y);
    let pitch = forward_z.clamp(-1.0, 1.0).asin();
    if pitch.cos() < 1e-6 {
        // Gimbal lock: with roll 0, side = (-sin yaw, cos yaw, 0).
        return [pitch, (-side_x).atan2(side_y), 0.0];
    }
    let yaw = forward_y.atan2(forward_x);
    let roll = (-side_z).atan2(up_z);
    [pitch, yaw, roll]
}

impl Scenario {
    /// The scenario's start as a physics frame (every car, player ids in car
    /// order, and the ball), with the cars' inputs unset.
    pub fn initial_frame(&self) -> PhysicsFrame {
        let car_state = |index: usize, start: &CarStart| CarState {
            player_id: index as u32,
            position: vec3(start.location.unwrap_or(DEFAULT_CAR_LOCATION)),
            rotation: rotator_to_quat(start.rotation.unwrap_or([0.0; 3])),
            velocity: vec3(start.velocity.unwrap_or([0.0; 3])),
            angular_velocity: vec3(start.angular_velocity.unwrap_or([0.0; 3])),
            boost_amount: start.boost.unwrap_or(100.0),
            input: None,
        };
        let cars: Vec<CarState> = std::iter::once(car_state(0, &self.car))
            .chain(
                self.others
                    .iter()
                    .enumerate()
                    .map(|(i, other)| car_state(i + 1, &other.car)),
            )
            .collect();
        let ball = self.ball.unwrap_or_default();
        PhysicsFrame {
            timestamp_secs: 0.0,
            ball: BallState {
                position: vec3(ball.location.unwrap_or(DEFAULT_BALL_LOCATION)),
                rotation: Quat {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                velocity: vec3(ball.velocity.unwrap_or([0.0; 3])),
                angular_velocity: vec3(ball.angular_velocity.unwrap_or([0.0; 3])),
            },
            cars,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const JUMP_THEN_DODGE: &str = r#"{
        "name": "jump then dodge",
        "settle_ticks": 2,
        "car": { "location": [0, 0, 17], "rotation": [0, 1.5, 0], "boost": 0 },
        "steps": [
            { "ticks": 3, "jump": true },
            { "ticks": 2 },
            { "ticks": 1, "jump": true, "pitch": -1 }
        ]
    }"#;

    #[test]
    fn a_scenario_reads_its_state_and_steps() {
        let scenario = Scenario::from_json(JUMP_THEN_DODGE).expect("valid");
        assert_eq!(scenario.name, "jump then dodge");
        assert_eq!(scenario.car.location, Some([0.0, 0.0, 17.0]));
        assert_eq!(scenario.car.boost, Some(0.0));
        assert_eq!(scenario.car.velocity, None);
        assert_eq!(scenario.ball, None);
        assert_eq!(scenario.total_ticks(), 8);
    }

    const TWO_CARS: &str = r#"{
        "name": "bump",
        "settle_ticks": 2,
        "car": { "location": [0, -500, 17], "rotation": [0, 1.5708, 0] },
        "steps": [ { "ticks": 4, "throttle": 1 } ],
        "others": [
            { "car": { "location": [0, 500, 17], "rotation": [0, -1.5708, 0] },
              "steps": [ { "ticks": 2 }, { "ticks": 6, "boost": true } ] }
        ]
    }"#;

    #[test]
    fn a_scenario_without_others_is_one_car_as_before() {
        let scenario = Scenario::from_json(JUMP_THEN_DODGE).expect("valid");
        assert_eq!(scenario.car_count(), 1);
        assert!(scenario.others.is_empty());
        assert_eq!(scenario.initial_frame().cars.len(), 1);
        // And an unset `others` is not written back.
        assert!(!scenario.to_json().expect("writes").contains("others"));
    }

    #[test]
    fn other_cars_have_their_own_start_and_tape_on_one_clock() {
        let scenario = Scenario::from_json(TWO_CARS).expect("valid");
        assert_eq!(scenario.car_count(), 2);
        let frame = scenario.initial_frame();
        assert_eq!(frame.cars.len(), 2);
        assert_eq!(frame.cars[0].player_id, 0);
        assert_eq!(frame.cars[1].player_id, 1);
        assert_eq!(frame.cars[1].position, Vec3::new(0.0, 500.0, 17.0));
        // Settling applies to every car; then each car follows its own steps.
        assert!(!scenario.input_at_car(0, 1).boost && !scenario.input_at_car(1, 1).boost);
        assert_eq!(scenario.input_at_car(0, 2).throttle, 1.0);
        assert!(
            !scenario.input_at_car(1, 3).boost,
            "its two neutral ticks first"
        );
        assert!(scenario.input_at_car(1, 4).boost);
        // Neutral past its own tape and for a car the scenario lacks.
        assert_eq!(scenario.input_at_car(0, 6), Input::default());
        assert!(scenario.input_at_car(1, 9).boost);
        assert_eq!(scenario.input_at_car(5, 4), Input::default());
        // The tape runs until the longest car's last step: 2 + 8.
        assert_eq!(scenario.total_ticks(), 10);
    }

    #[test]
    fn a_two_car_scenario_round_trips_through_json() {
        let scenario = Scenario::from_json(TWO_CARS).expect("valid");
        let again = Scenario::from_json(&scenario.to_json().expect("writes")).expect("reads");
        assert_eq!(scenario, again);
    }

    #[test]
    fn input_is_neutral_while_settling_then_follows_the_steps() {
        let scenario = Scenario::from_json(JUMP_THEN_DODGE).expect("valid");
        let jump = |tick| scenario.input_at(tick).jump;
        assert!(!jump(0) && !jump(1), "settling");
        assert!(jump(2) && jump(3) && jump(4), "first step");
        assert!(!jump(5) && !jump(6), "gap");
        let dodge = scenario.input_at(7);
        assert!(dodge.jump);
        assert_eq!(dodge.pitch, -1.0);
        assert_eq!(scenario.input_at(8), Input::default(), "after the tape");
        assert_eq!(scenario.input_at(10_000), Input::default());
    }

    #[test]
    fn a_missing_or_empty_tape_is_an_error() {
        let none = r#"{ "name": "x", "steps": [] }"#;
        assert!(matches!(
            Scenario::from_json(none),
            Err(ScenarioError::Empty)
        ));
        let zero = r#"{ "name": "x", "steps": [ { "ticks": 0, "jump": true } ] }"#;
        assert!(matches!(
            Scenario::from_json(zero),
            Err(ScenarioError::Empty)
        ));
        assert!(matches!(
            Scenario::from_json("{ not json"),
            Err(ScenarioError::Parse(_))
        ));
        let no_name = r#"{ "steps": [ { "ticks": 1 } ] }"#;
        assert!(matches!(
            Scenario::from_json(no_name),
            Err(ScenarioError::Parse(_))
        ));
    }

    #[test]
    fn every_shipped_scenario_parses() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/rb_tape_bot/scenarios");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).expect("scenarios dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let text = std::fs::read_to_string(&path).expect("read scenario");
                let scenario = Scenario::from_json(&text)
                    .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
                assert!(scenario.total_ticks() > 0, "{}", path.display());
                count += 1;
            }
        }
        assert!(count >= 10, "only {count} scenarios found");
    }

    #[test]
    fn a_rotator_round_trips_a_recorded_quaternion() {
        // test2.jsonl 18.308 s: rotator [-0.4315, -1.0776, 0.4492] against
        // the recorded quaternion (-0.0796464, 0.290751, -0.447697, 0.841836).
        let q = rotator_to_quat([-0.4315, -1.0776, 0.4492]);
        for (got, want) in [
            (q.x, -0.0796464),
            (q.y, 0.290751),
            (q.z, -0.447697),
            (q.w, 0.841836),
        ] {
            assert!((got - want).abs() < 1e-3, "{q:?}");
        }
    }

    #[test]
    fn a_rotator_round_trips_through_a_quaternion_and_back() {
        let half_pi = std::f32::consts::FRAC_PI_2;
        let cases: [[f32; 3]; 7] = [
            [0.0, 0.0, 0.0],
            [0.3, -1.2, 0.7],
            [-0.4315, -1.0776, 0.4492],
            [-1.4, 2.9, -2.5],
            [1.5, 0.4, -0.9],
            [half_pi - 1e-4, 1.0, 0.0],
            [-half_pi + 1e-4, -2.0, 0.0],
        ];
        for rotation in cases {
            let q = rotator_to_quat(rotation);
            let back = quat_to_rotator(q);
            let again = rotator_to_quat(back);
            // The same orientation, whichever sign the quaternion took.
            let sign = if again.w * q.w + again.x * q.x + again.y * q.y + again.z * q.z < 0.0 {
                -1.0
            } else {
                1.0
            };
            for (got, want) in [
                (sign * again.x, q.x),
                (sign * again.y, q.y),
                (sign * again.z, q.z),
                (sign * again.w, q.w),
            ] {
                assert!((got - want).abs() < 2e-3, "{rotation:?} -> {back:?}");
            }
            if rotation[0].abs() < 1.45 {
                // Away from the nose-up singularity the angles themselves
                // come back.
                for (got, want) in back.iter().zip(rotation.iter()) {
                    assert!((got - want).abs() < 1e-3, "{rotation:?} -> {back:?}");
                }
            }
        }
    }

    #[test]
    fn a_recorded_quaternion_becomes_the_rotator_that_made_it() {
        // The quaternion from `a_rotator_round_trips_a_recorded_quaternion`
        // (test2.jsonl 18.308 s), roll sign included.
        let q = Quat {
            x: -0.0796464,
            y: 0.290751,
            z: -0.447697,
            w: 0.841836,
        };
        let rotation = quat_to_rotator(q);
        for (got, want) in rotation.iter().zip([-0.4315, -1.0776, 0.4492]) {
            assert!((got - want).abs() < 1e-3, "{rotation:?}");
        }
    }

    #[test]
    fn a_scenario_writes_json_that_reads_back_equal() {
        let scenario = Scenario::from_json(JUMP_THEN_DODGE).expect("valid");
        let text = scenario.to_json().expect("serialises");
        assert!(
            !text.contains("velocity"),
            "absent fields are left out:
{text}"
        );
        let back = Scenario::from_json(&text).expect("reads back");
        assert_eq!(back, scenario);
        let with_ball = Scenario {
            ball: Some(BallStart {
                location: Some([1.0, 2.0, 93.15]),
                velocity: None,
                angular_velocity: Some([0.0, 0.5, 0.0]),
            }),
            ..scenario
        };
        let back = Scenario::from_json(&with_ball.to_json().expect("serialises")).expect("reads");
        assert_eq!(back, with_ball);
    }

    #[test]
    fn a_level_rotator_yawed_a_quarter_turn_faces_plus_y() {
        let q = rotator_to_quat([0.0, std::f32::consts::FRAC_PI_2, 0.0]);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!(q.x.abs() < 1e-5 && q.y.abs() < 1e-5);
        assert!((q.z - half).abs() < 1e-5 && (q.w - half).abs() < 1e-5);
    }

    #[test]
    fn the_initial_frame_uses_the_scenario_state_and_defaults() {
        let scenario = Scenario::from_json(JUMP_THEN_DODGE).expect("valid");
        let frame = scenario.initial_frame();
        assert_eq!(frame.cars.len(), 1);
        assert_eq!(frame.cars[0].position, Vec3::new(0.0, 0.0, 17.0));
        assert_eq!(frame.cars[0].boost_amount, 0.0);
        assert_eq!(frame.ball.position, Vec3::new(3000.0, 3000.0, 93.15));
        let jump = scenario.input_at(2).to_controller_input();
        assert!(jump.jump);
        assert_eq!(jump.pitch, Some(0.0));
    }
}
