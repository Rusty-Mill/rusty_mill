//! Scenario files for the tape-player bot: an initial game state plus a
//! run-length input timeline, replayed one input per game packet.
//!
//! Pure data and lookup, no RLBot types, so it is testable without the game.

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

#[cfg(test)]
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
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios");
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
}
