//! Airborne control: pitch/yaw/roll air control with its damping, and air
//! throttle, ported from RocketSim's `Car::_UpdateAirTorque`
//! (`RB-PHYSICS-001-FR-084`). The dodge's flip (in `jump`) decides whether
//! pitch is locked; air control otherwise always acts in the air
//! (`RB-PHYSICS-001-FR-093`).

use super::{forward_axis, right_axis, up_axis};
use crate::body::RigidBody;
use rb_domain::{ControllerInput, Vec3};

/// Air-control torque per unit stick for pitch, yaw and roll: RocketSim's
/// `CAR_AIR_CONTROL_TORQUE = (130, 95, 400)`.
pub(super) const AIR_CONTROL_TORQUE: Vec3 = Vec3::new(130.0, 95.0, 400.0);

/// Air-control damping per unit angular velocity for pitch, yaw and roll:
/// RocketSim's `CAR_AIR_CONTROL_DAMPING = (30, 20, 50)`. Pitch and yaw
/// damping fade out as their stick is held; roll damping always applies.
pub(super) const AIR_CONTROL_DAMPING: Vec3 = Vec3::new(30.0, 20.0, 50.0);

/// Scale from air-control torque units to angular acceleration (rad/s^2):
/// RocketSim's `CAR_TORQUE_SCALE = 2 * pi / 2^16 * 1000`. RocketSim applies
/// `(torque - damping) * CAR_TORQUE_SCALE` through the car's own inertia, so
/// the result is an angular acceleration independent of the car's mass
/// distribution.
pub(super) const CAR_TORQUE_SCALE: f32 = 2.0 * std::f32::consts::PI / 65_536.0 * 1000.0;

/// Forward acceleration (uu/s^2) from throttle while airborne: RocketSim's
/// `THROTTLE_AIR_ACCEL = 200 / 3`.
pub(super) const THROTTLE_AIR_ACCELERATION: f32 = 200.0 / 3.0;

/// Air control and air throttle for one airborne tick. `pitch_scale` is the
/// flip's pitch lock (`0` while locked, see `jump::flip_pitch_scale`).
/// Axes and signs are RocketSim's: pitch about -right (positive raises the
/// nose), yaw about up, roll about -forward (`RB-PHYSICS-001-FR-082`).
/// Damping acts even with the stick centered, so a free-spinning car slows
/// its rotation in the air, as in Rocket League.
pub(super) fn apply_air_control(
    car: &mut RigidBody,
    input: &ControllerInput,
    pitch_scale: f32,
    dt: f32,
) {
    let pitch_axis = -right_axis(car);
    let yaw_axis = up_axis(car);
    let roll_axis = -forward_axis(car);
    let pitch = input.pitch.unwrap_or(0.0).clamp(-1.0, 1.0) * pitch_scale;
    let yaw = input.yaw.unwrap_or(0.0).clamp(-1.0, 1.0);
    let roll = input.roll.unwrap_or(0.0).clamp(-1.0, 1.0);
    // The game drops the pitch torque while roll is held (bot-driven `aircombo_*`
    // recordings, level and nose-up/moving, either sign, with and without boost:
    // the spin is pure roll; yaw is kept). The pitch damping fade still follows
    // the stick. The keyboard `speed_flip` capture kept pitch (legacy, FR-155).
    let pitch_torque = if roll == 0.0 { pitch } else { 0.0 };

    let torque = pitch_axis * (pitch_torque * AIR_CONTROL_TORQUE.x)
        + yaw_axis * (yaw * AIR_CONTROL_TORQUE.y)
        + roll_axis * (roll * AIR_CONTROL_TORQUE.z);
    let spin = car.angular_velocity;
    let damping = pitch_axis
        * (pitch_axis.dot(&spin) * AIR_CONTROL_DAMPING.x * (1.0 - pitch.abs()))
        + yaw_axis * (yaw_axis.dot(&spin) * AIR_CONTROL_DAMPING.y * (1.0 - yaw.abs()))
        + roll_axis * (roll_axis.dot(&spin) * AIR_CONTROL_DAMPING.z);
    car.angular_velocity += (torque - damping) * (CAR_TORQUE_SCALE * dt);
}

/// RocketSim's air throttle: a small forward push from throttle while
/// airborne, whether or not air control is allowed.
pub(super) fn apply_air_throttle(car: &mut RigidBody, throttle: f32, forward: Vec3) {
    if throttle != 0.0 {
        car.apply_central_force(forward * (throttle * THROTTLE_AIR_ACCELERATION * car.mass()));
    }
}
