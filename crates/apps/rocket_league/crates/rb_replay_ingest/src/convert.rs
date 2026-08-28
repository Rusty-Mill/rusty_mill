//! Pure conversion between `boxcars`/`subtr_actor` replay types and
//! `rb_domain::state`. Kept separate from `lib.rs`'s `PhysicsStateSource`
//! implementation (which needs a real `.replay` file to exercise, since
//! `subtr_actor::FrameData`/`BallData`/`PlayerData` have no public
//! constructors) so these functions can be unit-tested directly with
//! hand-built `boxcars`/`subtr_actor` enum values instead.

use rb_domain::{BallState, CarState, ControllerInput, Quat, Vec3};
use subtr_actor::{BallFrame, PlayerFrame, ReplayData};

fn rigid_body_to_parts(rb: &boxcars::RigidBody) -> (Vec3, Quat, Vec3, Vec3) {
    let position = Vec3::new(rb.location.x, rb.location.y, rb.location.z);
    let rotation = Quat::new(rb.rotation.x, rb.rotation.y, rb.rotation.z, rb.rotation.w);
    let to_vec3 = |v: &boxcars::Vector3f| Vec3::new(v.x, v.y, v.z);
    // A sleeping rigid body may omit velocity entirely in the replay
    // stream rather than replicate an explicit zero — treating absence as
    // zero is correct, not a fallback for missing data.
    let velocity = rb
        .linear_velocity
        .as_ref()
        .map(to_vec3)
        .unwrap_or(Vec3::ZERO);
    let angular_velocity = rb
        .angular_velocity
        .as_ref()
        .map(to_vec3)
        .unwrap_or(Vec3::ZERO);
    (position, rotation, velocity, angular_velocity)
}

/// Rocket League replicates boost as a raw byte (0-255); the in-game HUD
/// shows it as a 0-100 percentage. `subtr_actor::PlayerFrame::Data::boost_amount`
/// is documented as the raw 0.0-255.0 value, so convert it here rather than
/// carry replay-specific units into `rb_domain::CarState`.
fn boost_raw_to_percent(raw: f32) -> f32 {
    (raw / 255.0 * 100.0).clamp(0.0, 100.0)
}

fn ball_frame_to_state(frame: &BallFrame) -> Option<BallState> {
    match frame {
        BallFrame::Empty => None,
        BallFrame::Data { rigid_body } => {
            let (position, rotation, velocity, angular_velocity) = rigid_body_to_parts(rigid_body);
            Some(BallState {
                position,
                rotation,
                velocity,
                angular_velocity,
            })
        }
    }
}

/// Rocket League replicates throttle/steer as a raw byte (~128 neutral, 0/255
/// at the extremes); `rb_domain::ControllerInput` uses -1.0..1.0 to match
/// BakkesMod's own `ControllerInput` convention (see ADR-0005) — normalize
/// here so both adapters' output lands in the same range.
fn axis_byte_to_analog(raw: u8) -> f32 {
    ((f32::from(raw) - 128.0) / 127.0).clamp(-1.0, 1.0)
}

/// `subtr_actor::PlayerInputFrame`'s throttle/steer bytes are themselves
/// `Option` (the replay didn't replicate a change to that property this
/// tick, not "this axis is unrecoverable") — absence defaults to neutral
/// (`0.0`), unlike `pitch`/`yaw`/`roll`, which stay permanently `None` for
/// replay-sourced input: a replay only ever exposes a one-shot dodge
/// impulse/torque vector, a different kind of quantity from an instantaneous
/// analog stick angle (see `rb_replay_ingest`'s non-goals) — fabricating a
/// pitch/yaw/roll value from it would misrepresent what was actually
/// recovered.
fn player_input_to_controller_input(
    input: &subtr_actor::PlayerInputFrame,
    boost_active: bool,
    jump_active: bool,
    double_jump_active: bool,
    powerslide_active: bool,
) -> ControllerInput {
    ControllerInput {
        throttle: input.throttle.map(axis_byte_to_analog).unwrap_or(0.0),
        steer: input.steer.map(axis_byte_to_analog).unwrap_or(0.0),
        pitch: None,
        yaw: None,
        roll: None,
        jump: jump_active || double_jump_active,
        boost: boost_active,
        handbrake: powerslide_active,
    }
}

/// `player_id` is a stable per-replay index (0, 1, 2, ... in the order
/// `subtr_actor` lists players), not a platform account ID —
/// `boxcars::RemoteId` is a multi-platform enum (Steam/Epic/PlayStation/...)
/// with no single numeric form, and `rb_domain::CarState.player_id` only
/// needs to distinguish cars within one replay, not identify a real
/// account. See `RB-VERIFY-001`.
fn player_frame_to_state(frame: &PlayerFrame, player_id: u32) -> Option<CarState> {
    match frame {
        PlayerFrame::Empty => None,
        PlayerFrame::Data {
            rigid_body,
            boost_amount,
            boost_active,
            powerslide_active,
            jump_active,
            double_jump_active,
            input,
            ..
        } => {
            let (position, rotation, velocity, angular_velocity) = rigid_body_to_parts(rigid_body);
            Some(CarState {
                player_id,
                position,
                rotation,
                velocity,
                angular_velocity,
                boost_amount: boost_raw_to_percent(*boost_amount),
                input: Some(player_input_to_controller_input(
                    input,
                    *boost_active,
                    *jump_active,
                    *double_jump_active,
                    *powerslide_active,
                )),
            })
        }
    }
}

/// Walks `replay_data`'s frame-indexed ball/player data and produces one
/// `PhysicsFrame` per index that has a resolvable ball state.
///
/// Frames where the ball itself is unavailable (`BallFrame::Empty` — ball
/// syncing disabled, or before the ball actor spawns) are omitted rather
/// than represented with a fabricated position: `rb_domain::PhysicsFrame`
/// requires a ball, and there is no meaningful placeholder for "no ball
/// yet." A car missing from a given frame (not yet spawned, or the replay
/// has fewer players than expected) is simply left out of that frame's
/// `cars`, rather than treated as an error — `RB-VERIFY-003`'s scoring
/// only depends on ball position today (see `RB-VERIFY-003-FR-002`).
pub fn to_physics_frames(replay_data: &ReplayData) -> Vec<rb_domain::PhysicsFrame> {
    let frame_data = &replay_data.frame_data;
    let ball_frames = frame_data.ball_data.frames();

    let mut frames = Vec::with_capacity(frame_data.metadata_frames.len());
    for (i, metadata) in frame_data.metadata_frames.iter().enumerate() {
        let Some(ball) = ball_frames.get(i).and_then(ball_frame_to_state) else {
            continue;
        };

        let cars = frame_data
            .players
            .iter()
            .enumerate()
            .filter_map(|(player_id, (_remote_id, player_data))| {
                let frame = player_data.frames().get(i)?;
                player_frame_to_state(frame, player_id as u32)
            })
            .collect();

        frames.push(rb_domain::PhysicsFrame {
            timestamp_secs: metadata.time,
            ball,
            cars,
        });
    }
    frames
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rigid_body(location: (f32, f32, f32)) -> boxcars::RigidBody {
        boxcars::RigidBody {
            sleeping: false,
            location: boxcars::Vector3f {
                x: location.0,
                y: location.1,
                z: location.2,
            },
            rotation: boxcars::Quaternion {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            linear_velocity: Some(boxcars::Vector3f {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            }),
            angular_velocity: None,
        }
    }

    #[test]
    fn rigid_body_conversion_maps_position_and_rotation() {
        let rb = rigid_body((10.0, 20.0, 30.0));
        let (position, rotation, velocity, angular_velocity) = rigid_body_to_parts(&rb);
        assert_eq!(position, Vec3::new(10.0, 20.0, 30.0));
        assert_eq!(rotation, Quat::IDENTITY);
        assert_eq!(velocity, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(
            angular_velocity,
            Vec3::ZERO,
            "None angular velocity defaults to zero"
        );
    }

    #[test]
    fn boost_raw_byte_converts_to_percent() {
        assert_eq!(boost_raw_to_percent(0.0), 0.0);
        assert!((boost_raw_to_percent(255.0) - 100.0).abs() < 1e-4);
        assert!((boost_raw_to_percent(127.5) - 50.0).abs() < 1e-2);
    }

    #[test]
    fn boost_raw_to_percent_clamps_out_of_range_input() {
        assert_eq!(boost_raw_to_percent(-10.0), 0.0);
        assert_eq!(boost_raw_to_percent(1000.0), 100.0);
    }

    #[test]
    fn ball_frame_empty_converts_to_none() {
        assert_eq!(ball_frame_to_state(&BallFrame::Empty), None);
    }

    #[test]
    fn ball_frame_data_converts_to_some_ball_state() {
        let frame = BallFrame::Data {
            rigid_body: rigid_body((0.0, 0.0, 100.0)),
        };
        let state = ball_frame_to_state(&frame).unwrap();
        assert_eq!(state.position, Vec3::new(0.0, 0.0, 100.0));
    }

    #[test]
    fn player_frame_empty_converts_to_none() {
        assert_eq!(player_frame_to_state(&PlayerFrame::Empty, 0), None);
    }

    #[test]
    fn player_frame_data_converts_to_some_car_state_with_given_id() {
        let frame = PlayerFrame::Data {
            rigid_body: rigid_body((5.0, -5.0, 17.0)),
            boost_amount: 255.0,
            boost_active: true,
            powerslide_active: false,
            jump_active: false,
            double_jump_active: false,
            dodge_active: false,
            player_name: None,
            team: None,
            is_team_0: None,
            camera: Default::default(),
            input: subtr_actor::PlayerInputFrame {
                throttle: Some(255),
                steer: Some(0),
                ..Default::default()
            },
        };
        let state = player_frame_to_state(&frame, 3).unwrap();
        assert_eq!(state.player_id, 3);
        assert_eq!(state.position, Vec3::new(5.0, -5.0, 17.0));
        assert!((state.boost_amount - 100.0).abs() < 1e-4);

        let input = state.input.unwrap();
        assert!((input.throttle - 1.0).abs() < 1e-2);
        assert!((input.steer - (-1.0)).abs() < 1e-2);
        assert_eq!(input.pitch, None);
        assert!(input.boost);
        assert!(!input.jump);
        assert!(!input.handbrake);
    }

    #[test]
    fn axis_byte_neutral_and_extremes_convert_correctly() {
        assert!((axis_byte_to_analog(128) - 0.0).abs() < 1e-2);
        assert!((axis_byte_to_analog(255) - 1.0).abs() < 1e-2);
        assert!((axis_byte_to_analog(0) - (-1.0)).abs() < 1e-2);
    }

    #[test]
    fn missing_axis_bytes_default_to_neutral_not_unrecoverable() {
        let input = player_input_to_controller_input(
            &subtr_actor::PlayerInputFrame::default(),
            false,
            false,
            false,
            false,
        );
        assert_eq!(input.throttle, 0.0);
        assert_eq!(input.steer, 0.0);
    }

    #[test]
    fn jump_active_or_double_jump_active_both_set_jump() {
        let input = player_input_to_controller_input(
            &subtr_actor::PlayerInputFrame::default(),
            false,
            false,
            true,
            false,
        );
        assert!(input.jump, "double_jump_active alone should still set jump");
    }

    #[test]
    fn replay_input_never_recovers_analog_pitch_yaw_roll() {
        let input = player_input_to_controller_input(
            &subtr_actor::PlayerInputFrame::default(),
            false,
            false,
            false,
            false,
        );
        assert_eq!(input.pitch, None);
        assert_eq!(input.yaw, None);
        assert_eq!(input.roll, None);
    }
}
