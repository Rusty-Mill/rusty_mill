//! The two pure conversions between RLBot's records and `rb_domain`'s.

use rb_domain::{BallState, CarState, ControllerInput, PhysicsFrame, Quat, Vec3};
use rb_rlbot_wire::{ControllerState, GamePacket, Physics, Rotator, Vec3 as WireVec3};
use rb_scenario::{quat_to_rotator, rotator_to_quat};

fn vec3(v: WireVec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn rotation(p: &Physics) -> Quat {
    rotator_to_quat([p.rotation.pitch, p.rotation.yaw, p.rotation.roll])
}

/// The observation `rb_env` takes (`Env::reset`, a policy's input) for one game packet.
///
/// Cars keep the packet's order, so a car's index here is its `player_index` in `PlayerInput`;
/// `player_id` is that index, as for a scenario. Units are the game's (uu, uu/s, rad/s, boost
/// 0 to 100), which `rb_env` uses too. The timestamp is the match clock, `seconds_elapsed`.
/// `None` when the packet has no ball (between matches), so there is nothing to observe.
pub fn observation(packet: &GamePacket) -> Option<PhysicsFrame> {
    let ball = packet.balls.first()?.physics;
    Some(PhysicsFrame {
        timestamp_secs: packet.match_info.seconds_elapsed,
        ball: BallState {
            position: vec3(ball.location),
            rotation: rotation(&ball),
            velocity: vec3(ball.velocity),
            angular_velocity: vec3(ball.angular_velocity),
        },
        cars: packet
            .players
            .iter()
            .enumerate()
            .map(|(index, p)| CarState {
                player_id: index as u32,
                position: vec3(p.physics.location),
                rotation: rotation(&p.physics),
                velocity: vec3(p.physics.velocity),
                angular_velocity: vec3(p.physics.angular_velocity),
                boost_amount: p.boost,
                input: None,
            })
            .collect(),
    })
}

/// A policy's input as RLBot's controller state. Axes the policy left unset (`None`) are
/// centred, which is what a stick at rest sends. `use_item` is never pressed (Soccar has no items).
pub fn controller_state(input: &ControllerInput) -> ControllerState {
    ControllerState {
        throttle: input.throttle,
        steer: input.steer,
        pitch: input.pitch.unwrap_or(0.0),
        yaw: input.yaw.unwrap_or(0.0),
        roll: input.roll.unwrap_or(0.0),
        jump: input.jump,
        boost: input.boost,
        handbrake: input.handbrake,
        use_item: false,
    }
}

/// The inverse of [`controller_state`], for a core that applies a bot's `PlayerInput`: every
/// stick axis is `Some`, because a wire controller state always carries all of them. `use_item`
/// has no `ControllerInput` counterpart and is dropped.
pub fn controller_input(state: &ControllerState) -> ControllerInput {
    ControllerInput {
        throttle: state.throttle,
        steer: state.steer,
        pitch: Some(state.pitch),
        yaw: Some(state.yaw),
        roll: Some(state.roll),
        jump: state.jump,
        boost: state.boost,
        handbrake: state.handbrake,
    }
}

/// RLBot's physics record for a body's state; the inverse of the conversion [`observation`]
/// does per body. `rotation` goes through `rb_scenario::quat_to_rotator`.
pub fn physics(position: Vec3, rotation: Quat, velocity: Vec3, angular_velocity: Vec3) -> Physics {
    let wire = |v: Vec3| WireVec3 {
        x: v.x,
        y: v.y,
        z: v.z,
    };
    let [pitch, yaw, roll] = quat_to_rotator(rotation);
    Physics {
        location: wire(position),
        rotation: Rotator { pitch, yaw, roll },
        velocity: wire(velocity),
        angular_velocity: wire(angular_velocity),
    }
}
