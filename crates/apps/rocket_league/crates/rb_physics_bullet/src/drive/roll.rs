//! Auto-roll (`RB-PHYSICS-001-FR-131`): RocketSim's `Car::_UpdateAutoRoll`.
//! A car on its throttle with one to three wheels on a surface, or that
//! touched one with its body last tick, is pressed toward that surface and
//! turned to lie flat on it, so a car driving over a wall's seam or landing
//! on a wheel rights itself.

use super::wheels::{average_normal, WheelContacts};
use super::{forward_axis, right_axis, DriveState};
use crate::body::RigidBody;

/// `CAR_AUTOROLL_FORCE` (uu/s^2 toward the surface).
const AUTOROLL_FORCE: f32 = 100.0;
/// `CAR_AUTOROLL_TORQUE` (rad/s^2 at full misalignment).
const AUTOROLL_TORQUE: f32 = 80.0;

/// One tick of auto-roll for `car`, from the wheels' contacts at the start
/// of the step, `state`'s remembered body contact and the raw throttle.
pub(super) fn auto_roll(
    car: &mut RigidBody,
    wheels: &WheelContacts,
    state: &DriveState,
    throttle: f32,
    dt: f32,
) {
    let touching = wheels.iter().flatten().count();
    let on_body = state.world_contact_normal;
    if throttle == 0.0 || !((1..4).contains(&touching) || on_body.is_some()) {
        return;
    }
    let ground_up = if touching > 0 {
        average_normal(wheels)
    } else {
        on_body
    };
    let Some(ground_up) = ground_up else {
        return;
    };
    let ground_down = -ground_up;
    let forward = forward_axis(car);
    let right = right_axis(car);
    let cross_right = ground_up.cross(&forward);
    let cross_forward = ground_down.cross(&cross_right);
    let right_factor = 1.0 - right.dot(&cross_right).clamp(0.0, 1.0);
    let forward_factor = 1.0 - forward.dot(&cross_forward).clamp(0.0, 1.0);
    let direction_right = forward
        * if right.dot(&ground_up) >= 0.0 {
            -1.0
        } else {
            1.0
        };
    let direction_forward = right
        * if forward.dot(&ground_up) >= 0.0 {
            1.0
        } else {
            -1.0
        };
    car.apply_central_force(ground_down * (AUTOROLL_FORCE * car.mass()));
    car.angular_velocity += (direction_forward * forward_factor + direction_right * right_factor)
        * (AUTOROLL_TORQUE * dt);
}
