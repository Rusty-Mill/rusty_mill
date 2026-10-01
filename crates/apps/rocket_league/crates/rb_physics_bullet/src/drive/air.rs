//! Airborne rotation: pitch/yaw/roll air control and the landing
//! auto-orientation assist. Only runs while the car is airborne; see
//! `super::apply_driven_forces`.

use super::{right_axis, up_axis};
use crate::body::RigidBody;
use rb_domain::{ControllerInput, Vec3};

/// Uncalibrated placeholder air-control torque magnitude for *pitch*
/// specifically (about the car's local right axis) at full analog input —
/// chosen only so a full-stick pitch is visibly responsive for this car's
/// mass/inertia in tests, not derived from any measured or documented
/// Rocket League value. Since `RB-PHYSICS-001-FR-068`, yaw and roll no
/// longer share this same magnitude: they're scaled from it by
/// `AIR_CONTROL_YAW_SCALE`/`AIR_CONTROL_ROLL_SCALE`, RocketSim's own
/// confirmed real per-axis ratios — see that constant's own doc comment
/// for the full finding. Real Rocket League's pitch/yaw/roll rates still
/// differ in absolute magnitude too, not just this ratio, from this port's
/// own uncalibrated pitch baseline; only the *ratio* between axes is
/// adopted here.
pub(super) const AIR_CONTROL_TORQUE: f32 = 1_000_000.0;

/// Confirmed real ratio: yaw's real air-control torque is this fraction of
/// pitch's. `RB-PHYSICS-001-FR-068` fetched RocketSim's real `Car.cpp`
/// (`_UpdateAirTorque`) and found real Rocket League's air control is the
/// same *kind* of mechanism this port already models — a torque about each
/// local axis, scaled directly by analog stick input, not a wheel/tire
/// model like steering (`RB-PHYSICS-001-FR-065`) or a friction split like
/// handbrake (`FR-066`) turned out to need — computed as
/// `pitch * dirPitch_right * CAR_AIR_CONTROL_TORQUE.x + yaw * dirYaw_up *
/// CAR_AIR_CONTROL_TORQUE.y + roll * dirRoll_forward *
/// CAR_AIR_CONTROL_TORQUE.z`. RocketSim's own `RLConst.h` confirms
/// `CAR_AIR_CONTROL_TORQUE = Vec(130, 95, 400)` ("Angle order is PYR"),
/// giving `95.0 / 130.0` for yaw relative to pitch. Because the real
/// mechanism matches this port's own structurally (a direct per-axis
/// torque, not a value requiring a transfer function this port's
/// architecture can't represent), this per-axis *ratio* is adoptable the
/// same way `RB-PHYSICS-001-FR-058`'s throttle taper and `FR-059`'s dodge
/// scale are — a direct multiplier on a torque this port already applies
/// the same way real Rocket League does, transferring cleanly regardless
/// of this port's own uncalibrated `AIR_CONTROL_TORQUE` magnitude (unlike
/// the real curve's own *absolute* torque values, which `RB-PHYSICS-001-FR-031`'s
/// "false precision" finding already ruled out for this port's
/// differently-calibrated car body).
pub(super) const AIR_CONTROL_YAW_SCALE: f32 = 95.0 / 130.0;

/// Confirmed real ratio: roll's real air-control torque is this multiple of
/// pitch's — see `AIR_CONTROL_YAW_SCALE`'s own doc comment for the full
/// finding and reasoning. Same source: `CAR_AIR_CONTROL_TORQUE.z /
/// CAR_AIR_CONTROL_TORQUE.x` = `400.0 / 130.0`.
///
/// `RB-PHYSICS-001-FR-071` closes a thread `RB-PHYSICS-001-FR-068`'s own
/// Non-goals left open — RocketSim's `CAR_AIR_CONTROL_DAMPING = Vec(30, 20,
/// 50)`, which that requirement's own fetch of `_UpdateAirTorque` found but
/// didn't examine. The full mechanism: for each axis, real air control
/// subtracts a damping torque `(angular velocity along that axis) *
/// CAR_AIR_CONTROL_DAMPING[axis] * (1 - abs(analog input on that axis))`
/// from the applied torque *before* scaling by inertia — pitch's own input
/// term additionally multiplies by `pitchTorqueScale`
/// (`RB-PHYSICS-001-FR-070`). Releasing the stick on an axis (input `0`)
/// gives full damping strength on that axis, continuously bleeding off any
/// existing spin; holding it fully (input `±1`) zeroes the damping,
/// granting full torque authority with no resistance. Not adopted: unlike
/// the pitch/yaw/roll *ratio* above, this port has no existing damping
/// quantity to apply a ratio to — this is a wholly new torque contribution,
/// not a multiplier on one this port already computes the same way, so it
/// doesn't transfer the way `RB-PHYSICS-001-FR-058`/`FR-059`/`FR-068`'s own
/// ratios did. Its real absolute coefficients are also calibrated against
/// real Rocket League's own specific inertia tensor, the same "false
/// precision" reasoning that already keeps `AIR_CONTROL_TORQUE` itself a
/// placeholder. Introducing this mechanism for real remains a candidate for
/// a future, dedicated requirement, exactly as `FR-068`'s own Non-goals
/// already flagged.
pub(super) const AIR_CONTROL_ROLL_SCALE: f32 = 400.0 / 130.0;

/// Uncalibrated placeholder landing-auto-orientation restoring-torque
/// magnitude — applied while airborne with no active `pitch`/`roll` air
/// control input, scaled by `up_axis(car).cross(&world_up)` (already
/// proportional to the sine of the car's tilt off level, since both
/// vectors are unit length, so a bigger tilt earns a stronger nudge and an
/// already-level car earns none). Chosen only to be a visibly gentler
/// correction than full active air control (`AIR_CONTROL_TORQUE`) for this
/// car's mass/inertia in tests — a full order of magnitude smaller — not
/// derived from any measured or documented Rocket League value; this port
/// has no public reference for the real assist's actual strength or
/// trigger condition either (see the module doc comment).
pub(super) const LANDING_AUTO_UPRIGHT_TORQUE: f32 = 100_000.0;

/// Pitch/yaw/roll air-control torque about the car's local right/up/forward
/// axes, followed by the landing auto-orientation assist when no pitch/roll
/// is held and `jump_pressed` is false.
pub(super) fn apply_air_control(
    car: &mut RigidBody,
    input: &ControllerInput,
    forward: Vec3,
    jump_pressed: bool,
) {
    // Unlike ground steering, not scaled by speed — a car can spin from a
    // standing start in the air, since there's no wheel grip to require
    // momentum for.
    let pitch = input.pitch.unwrap_or(0.0).clamp(-1.0, 1.0);
    if pitch != 0.0 {
        // RocketSim's `dirPitch_right = -GetRightDir()`: positive pitch
        // raises the nose (RB-PHYSICS-001-FR-082).
        car.apply_torque(right_axis(car) * (-pitch * AIR_CONTROL_TORQUE));
    }

    let yaw = input.yaw.unwrap_or(0.0).clamp(-1.0, 1.0);
    if yaw != 0.0 {
        car.apply_torque(up_axis(car) * (yaw * AIR_CONTROL_TORQUE * AIR_CONTROL_YAW_SCALE));
    }

    let roll = input.roll.unwrap_or(0.0).clamp(-1.0, 1.0);
    if roll != 0.0 {
        // RocketSim's `dirRoll_forward = -GetForwardDir()`.
        car.apply_torque(forward * (-roll * AIR_CONTROL_TORQUE * AIR_CONTROL_ROLL_SCALE));
    }

    // Landing auto-orientation assistance: with no active pitch/roll
    // air control this step (so the assist never fights the player's
    // own input) and no fresh jump press this step (so it never
    // interacts, within the same integrate_velocities call, with a
    // dodge/wall-jump-dodge/double-jump/flip-cancel's own direct
    // velocity or angular-velocity change — those already dominate the
    // car's rotation for that instant anyway), gently nudge the car's
    // local up axis toward world up. `up.cross(&world_up)` gives both
    // the correction axis and, since both are unit vectors, a
    // magnitude already proportional to the sine of the tilt angle —
    // a level car (or one resting exactly upside-down, an unlikely
    // singularity this simple scheme doesn't resolve) gets no
    // correction, a heavily tilted one gets a stronger nudge. See the
    // parent module doc comment for why this applies continuously
    // whenever airborne rather than only near the ground.
    if pitch == 0.0 && roll == 0.0 && !jump_pressed {
        let world_up = Vec3::new(0.0, 0.0, 1.0);
        let correction_axis = up_axis(car).cross(&world_up);
        if correction_axis.length() > 0.0 {
            car.apply_torque(correction_axis * LANDING_AUTO_UPRIGHT_TORQUE);
        }
    }
}
