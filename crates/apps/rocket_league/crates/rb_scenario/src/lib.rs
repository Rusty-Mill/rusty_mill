//! Scenario files for scripted mechanic captures: an initial game state plus
//! a run-length input timeline, replayed one input per game packet by
//! `tools/rb_tape_bot` and simulated by `rb-verify --scenario`
//! (`docs/research/BOT-CAPTURE-PLAN.md`).
//!
//! Pure data and lookup, no RLBot types, so it is testable without the game.

use rb_domain::{BallState, CarState, ControllerInput, PhysicsFrame, Quat, Vec3};
use serde::Deserialize;

/// One tick of controller input, the same fields as the capture format's
/// `input` object (ADR-0005).
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Step {
    pub ticks: u64,
    #[serde(flatten)]
    pub input: Input,
}

/// A car's initial state; absent fields are left as the game has them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct CarStart {
    pub location: Option<[f32; 3]>,
    /// `[pitch, yaw, roll]` in radians, RLBot's convention.
    pub rotation: Option<[f32; 3]>,
    pub velocity: Option<[f32; 3]>,
    pub angular_velocity: Option<[f32; 3]>,
    /// 0 to 100.
    pub boost: Option<f32>,
}

/// A ball's initial state; absent fields are left as the game has them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct BallStart {
    pub location: Option<[f32; 3]>,
    pub velocity: Option<[f32; 3]>,
    pub angular_velocity: Option<[f32; 3]>,
}

/// A scenario: where the car (and ball) start and what the car presses.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Scenario {
    pub name: String,
    /// Neutral packets after the start state is set and before the first
    /// step, so the car can settle (a car set on the floor is still
    /// dropping or springing for a few ticks).
    #[serde(default)]
    pub settle_ticks: u64,
    #[serde(default)]
    pub car: CarStart,
    #[serde(default)]
    pub ball: Option<BallStart>,
    pub steps: Vec<Step>,
}

/// Why a scenario could not be read.
#[derive(Debug)]
pub enum ScenarioError {
    /// Not valid JSON for the schema.
    Parse(serde_json::Error),
    /// No steps: nothing to play.
    Empty,
}

impl std::fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScenarioError::Parse(err) => write!(f, "invalid scenario JSON: {err}"),
            ScenarioError::Empty => write!(f, "scenario has no steps"),
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

    /// Packets of input the scenario plays, settling included.
    pub fn total_ticks(&self) -> u64 {
        self.settle_ticks + self.steps.iter().map(|step| step.ticks).sum::<u64>()
    }

    /// The input for packet `tick`, counted from the packet after the start
    /// state was set: neutral while settling and after the last step.
    pub fn input_at(&self, tick: u64) -> Input {
        let Some(mut remaining) = tick.checked_sub(self.settle_ticks) else {
            return Input::default();
        };
        for step in &self.steps {
            if remaining < step.ticks {
                return step.input;
            }
            remaining -= step.ticks;
        }
        Input::default()
    }
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

impl Scenario {
    /// The scenario's start as a physics frame (one car, player 0, and the
    /// ball), with the car's input unset.
    pub fn initial_frame(&self) -> PhysicsFrame {
        let car = CarState {
            player_id: 0,
            position: vec3(self.car.location.unwrap_or(DEFAULT_CAR_LOCATION)),
            rotation: rotator_to_quat(self.car.rotation.unwrap_or([0.0; 3])),
            velocity: vec3(self.car.velocity.unwrap_or([0.0; 3])),
            angular_velocity: vec3(self.car.angular_velocity.unwrap_or([0.0; 3])),
            boost_amount: self.car.boost.unwrap_or(100.0),
            input: None,
        };
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
            cars: vec![car],
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
