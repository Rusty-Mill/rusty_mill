//! The JSON-Lines capture file wire format (see ADR-0005) and pure
//! conversion into `rb_domain::state` types. Kept separate from `lib.rs`'s
//! `PhysicsStateSource` implementation, mirroring `rb_replay_ingest::convert`'s
//! own split, so these functions are unit-testable with hand-built values
//! instead of needing a real capture file on disk.
//!
//! One JSON object per line: `{"timestamp_secs", "ball", "cars"}`, matching
//! `rb_domain::PhysicsFrame`'s shape. `serde`'s field names are the wire
//! format itself — changing a field name here is a capture-format change,
//! not a refactor.

use rb_domain::{BallState, CarState, ControllerInput, PhysicsFrame, Quat, Vec3};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireVec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl From<WireVec3> for Vec3 {
    fn from(v: WireVec3) -> Vec3 {
        Vec3::new(v.x, v.y, v.z)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireQuat {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

impl From<WireQuat> for Quat {
    fn from(q: WireQuat) -> Quat {
        Quat::new(q.x, q.y, q.z, q.w)
    }
}

/// Mirrors `rb_domain::ControllerInput` field-for-field (see ADR-0005) — a
/// BakkesMod capture is expected to always have all eight fields, unlike
/// replay-sourced input where `pitch`/`yaw`/`roll` are structurally absent.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireInput {
    throttle: f32,
    steer: f32,
    #[serde(default)]
    pitch: Option<f32>,
    #[serde(default)]
    yaw: Option<f32>,
    #[serde(default)]
    roll: Option<f32>,
    jump: bool,
    boost: bool,
    handbrake: bool,
}

impl From<WireInput> for ControllerInput {
    fn from(i: WireInput) -> ControllerInput {
        ControllerInput {
            throttle: i.throttle,
            steer: i.steer,
            pitch: i.pitch,
            yaw: i.yaw,
            roll: i.roll,
            jump: i.jump,
            boost: i.boost,
            handbrake: i.handbrake,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireCar {
    player_id: u32,
    position: WireVec3,
    rotation: WireQuat,
    velocity: WireVec3,
    angular_velocity: WireVec3,
    boost_amount: f32,
    input: WireInput,
}

impl From<WireCar> for CarState {
    fn from(c: WireCar) -> CarState {
        CarState {
            player_id: c.player_id,
            position: c.position.into(),
            rotation: c.rotation.into(),
            velocity: c.velocity.into(),
            angular_velocity: c.angular_velocity.into(),
            boost_amount: c.boost_amount,
            input: Some(c.input.into()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireBall {
    position: WireVec3,
    rotation: WireQuat,
    velocity: WireVec3,
    angular_velocity: WireVec3,
}

impl From<WireBall> for BallState {
    fn from(b: WireBall) -> BallState {
        BallState {
            position: b.position.into(),
            rotation: b.rotation.into(),
            velocity: b.velocity.into(),
            angular_velocity: b.angular_velocity.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireFrame {
    timestamp_secs: f32,
    ball: WireBall,
    #[serde(default)]
    cars: Vec<WireCar>,
}

impl From<WireFrame> for PhysicsFrame {
    fn from(f: WireFrame) -> PhysicsFrame {
        PhysicsFrame {
            timestamp_secs: f.timestamp_secs,
            ball: f.ball.into(),
            cars: f.cars.into_iter().map(CarState::from).collect(),
        }
    }
}

/// Parses one JSON-Lines line into a `PhysicsFrame`. Public within the crate
/// only — `lib.rs` is the one caller that has an actual line to parse.
pub(crate) fn parse_line(line: &str) -> Result<PhysicsFrame, serde_json::Error> {
    serde_json::from_str::<WireFrame>(line).map(PhysicsFrame::from)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn wire_vec3_converts_componentwise() {
        let v: Vec3 = WireVec3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        }
        .into();
        assert_eq!(v, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn wire_input_preserves_none_analog_axes() {
        let input: ControllerInput = WireInput {
            throttle: 0.5,
            steer: -0.5,
            pitch: None,
            yaw: None,
            roll: None,
            jump: true,
            boost: false,
            handbrake: false,
        }
        .into();
        assert_eq!(input.pitch, None);
        assert!(input.jump);
    }

    #[test]
    fn parse_line_decodes_a_full_frame_with_one_car() {
        let line = r#"{"timestamp_secs":1.5,"ball":{"position":{"x":0.0,"y":0.0,"z":93.0},"rotation":{"x":0.0,"y":0.0,"z":0.0,"w":1.0},"velocity":{"x":0.0,"y":0.0,"z":0.0},"angular_velocity":{"x":0.0,"y":0.0,"z":0.0}},"cars":[{"player_id":0,"position":{"x":10.0,"y":20.0,"z":17.0},"rotation":{"x":0.0,"y":0.0,"z":0.0,"w":1.0},"velocity":{"x":1.0,"y":0.0,"z":0.0},"angular_velocity":{"x":0.0,"y":0.0,"z":0.0},"boost_amount":33.3,"input":{"throttle":1.0,"steer":0.0,"pitch":0.2,"yaw":0.0,"roll":-0.4,"jump":false,"boost":true,"handbrake":false}}]}"#;

        let frame = parse_line(line).unwrap();
        assert_eq!(frame.timestamp_secs, 1.5);
        assert_eq!(frame.ball.position, Vec3::new(0.0, 0.0, 93.0));
        assert_eq!(frame.cars.len(), 1);

        let car = &frame.cars[0];
        assert_eq!(car.player_id, 0);
        assert!((car.boost_amount - 33.3).abs() < 1e-4);

        let input = car.input.unwrap();
        assert!((input.throttle - 1.0).abs() < 1e-6);
        assert_eq!(input.pitch, Some(0.2));
        assert!(input.boost);
        assert!(!input.jump);
    }

    #[test]
    fn parse_line_decodes_a_frame_with_no_cars() {
        let line = r#"{"timestamp_secs":0.0,"ball":{"position":{"x":0.0,"y":0.0,"z":93.0},"rotation":{"x":0.0,"y":0.0,"z":0.0,"w":1.0},"velocity":{"x":0.0,"y":0.0,"z":0.0},"angular_velocity":{"x":0.0,"y":0.0,"z":0.0}}}"#;

        let frame = parse_line(line).unwrap();
        assert!(frame.cars.is_empty());
    }

    #[test]
    fn parse_line_rejects_malformed_json() {
        assert!(parse_line("not json").is_err());
    }

    #[test]
    fn parse_line_rejects_a_frame_missing_the_ball() {
        assert!(parse_line(r#"{"timestamp_secs":0.0}"#).is_err());
    }
}
