//! Driven car input: couples `rb_domain::ControllerInput` into forces and
//! torques on a car `RigidBody`. Ground-driving only in this increment —
//! throttle (accelerate/reverse along the car's local forward axis) and
//! steering (yaw torque scaled by current speed), both gated on the car
//! actually touching the ground (a free-floating box has no wheels to
//! grip, so airborne input does nothing here).
//!
//! **Not implemented** (tracked in `RB-PHYSICS-001`, not silently
//! dropped): boost, jump, air-control (pitch/yaw/roll torque while
//! airborne), and handbrake/drift. A car with no input set (or all-neutral
//! `ControllerInput::default()`) behaves exactly as a free rigid box
//! always has — this module only ever adds force/torque, never removes
//! the existing physics.
//!
//! This is not a Bullet3 port (Bullet has no concept of "a car's engine")
//! — it's this project's own model of Rocket League's driving mechanics,
//! since the real numbers are not public. `MAX_CAR_SPEED` is a
//! commonly-cited community-reverse-engineered approximation (the same
//! body of public research `PhysicsWorld::new`'s gravity constant comes
//! from); `THROTTLE_ACCELERATION` is a simplified constant standing in for
//! Rocket League's real speed-dependent throttle curve; `STEER_TORQUE` is
//! an uncalibrated placeholder chosen only to produce a visibly responsive
//! turn in this car's mass/inertia for tests. None of these are
//! independently confirmed by this project — see `RB-PHYSICS-001-FR-005`.

use crate::body::RigidBody;
use rb_domain::{ControllerInput, Vec3};

/// Commonly-cited approximate max ground speed a car's engine alone can
/// reach (uu/s) — Rocket League's unboosted top speed.
pub const MAX_CAR_SPEED: f32 = 2300.0;

/// Simplified constant throttle acceleration (uu/s^2). Rocket League's
/// real throttle curve tapers off nonlinearly as speed rises toward
/// `MAX_CAR_SPEED`; this port uses one constant instead, a real
/// simplification (not a taper), pending calibration against recorded
/// data.
const THROTTLE_ACCELERATION: f32 = 1600.0;

/// Uncalibrated placeholder steering torque magnitude (about the car's
/// local up axis, at full `steer` input and at/above `MAX_CAR_SPEED`) —
/// chosen only so a full-lock turn is visibly responsive for this car's
/// mass/inertia in tests, not derived from any measured or documented
/// Rocket League value.
const STEER_TORQUE: f32 = 1_500_000.0;

fn forward_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(1.0, 0.0, 0.0))
}

fn up_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(0.0, 0.0, 1.0))
}

/// Applies throttle and steering as forces/torques on `car`, if `on_ground`
/// (a no-op otherwise — see the module doc comment). Call once per step,
/// before `integrate::integrate_velocities`, alongside `apply_gravity`.
pub fn apply_driven_forces(car: &mut RigidBody, input: &ControllerInput, on_ground: bool) {
    if !on_ground {
        return;
    }

    let forward = forward_axis(car);
    let forward_speed = car.linear_velocity.dot(&forward);

    let throttle = input.throttle.clamp(-1.0, 1.0);
    if throttle != 0.0 && throttle.signum() * forward_speed < MAX_CAR_SPEED {
        car.apply_central_force(forward * (throttle * THROTTLE_ACCELERATION * car.mass()));
    }

    let steer = input.steer.clamp(-1.0, 1.0);
    if steer != 0.0 {
        // A stationary car can't carve a turn — scale the available
        // torque by how fast it's already going, up to MAX_CAR_SPEED.
        let speed_factor = (car.linear_velocity.length() / MAX_CAR_SPEED).min(1.0);
        if speed_factor > 0.0 {
            car.apply_torque(up_axis(car) * (steer * STEER_TORQUE * speed_factor));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::integrate;

    fn car() -> RigidBody {
        RigidBody::car_box(Vec3::new(60.0, 30.0, 18.0), 180.0, Vec3::ZERO)
    }

    fn step_with_input(car: &mut RigidBody, input: &ControllerInput, on_ground: bool, dt: f32) {
        car.clear_forces();
        apply_driven_forces(car, input, on_ground);
        integrate::integrate_velocities(car, dt);
    }

    fn full_throttle() -> ControllerInput {
        ControllerInput {
            throttle: 1.0,
            ..Default::default()
        }
    }

    fn full_steer() -> ControllerInput {
        ControllerInput {
            steer: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn neutral_input_applies_no_force_or_torque() {
        let mut c = car();
        step_with_input(&mut c, &ControllerInput::default(), true, 1.0 / 60.0);
        assert_eq!(c.linear_velocity, Vec3::ZERO);
        assert_eq!(c.angular_velocity, Vec3::ZERO);
    }

    #[test]
    fn throttle_accelerates_a_grounded_car_forward() {
        let mut c = car();
        for _ in 0..60 {
            step_with_input(&mut c, &full_throttle(), true, 1.0 / 60.0);
        }
        assert!(
            c.linear_velocity.x > 0.0,
            "expected forward speed to increase, got {}",
            c.linear_velocity.x
        );
        assert!((c.linear_velocity.y).abs() < 1e-4);
        assert!((c.linear_velocity.z).abs() < 1e-4);
    }

    #[test]
    fn throttle_has_no_effect_while_airborne() {
        let mut c = car();
        for _ in 0..60 {
            step_with_input(&mut c, &full_throttle(), false, 1.0 / 60.0);
        }
        assert_eq!(
            c.linear_velocity.x, 0.0,
            "airborne throttle shouldn't add forward speed"
        );
    }

    #[test]
    fn throttle_stops_accelerating_at_max_speed() {
        let mut c = car();
        c.linear_velocity = Vec3::new(MAX_CAR_SPEED, 0.0, 0.0);
        step_with_input(&mut c, &full_throttle(), true, 1.0 / 60.0);
        assert!(
            (c.linear_velocity.x - MAX_CAR_SPEED).abs() < 1e-4,
            "expected throttle to stop pushing past MAX_CAR_SPEED, got {}",
            c.linear_velocity.x
        );
    }

    #[test]
    fn reverse_throttle_accelerates_backward() {
        let mut c = car();
        let input = ControllerInput {
            throttle: -1.0,
            ..Default::default()
        };
        for _ in 0..60 {
            step_with_input(&mut c, &input, true, 1.0 / 60.0);
        }
        assert!(
            c.linear_velocity.x < 0.0,
            "expected reverse throttle to push backward, got {}",
            c.linear_velocity.x
        );
    }

    #[test]
    fn steer_has_no_effect_on_a_stationary_car() {
        let mut c = car();
        step_with_input(&mut c, &full_steer(), true, 1.0 / 60.0);
        assert_eq!(
            c.angular_velocity,
            Vec3::ZERO,
            "a parked car shouldn't be able to turn in place"
        );
    }

    #[test]
    fn steer_yaws_a_moving_car() {
        let mut c = car();
        c.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
        step_with_input(&mut c, &full_steer(), true, 1.0 / 60.0);
        assert!(
            c.angular_velocity.z.abs() > 0.0,
            "expected a moving car to yaw under full steer, got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn opposite_steer_yaws_the_opposite_way() {
        let mut left = car();
        left.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
        step_with_input(&mut left, &full_steer(), true, 1.0 / 60.0);

        let mut right = car();
        right.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
        let opposite = ControllerInput {
            steer: -1.0,
            ..Default::default()
        };
        step_with_input(&mut right, &opposite, true, 1.0 / 60.0);

        assert!(left.angular_velocity.z * right.angular_velocity.z < 0.0);
    }
}
