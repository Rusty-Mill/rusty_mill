//! Driven car input: couples `rb_domain::ControllerInput` into forces and
//! torques on a car `RigidBody`. Ground-driving (throttle, steering),
//! boost, and handbrake/drift in this increment. Throttle, steering, and
//! handbrake are gated on the car actually touching the ground (a
//! free-floating box has no wheels to grip or lock, so airborne input does
//! nothing for any of them here); boost is not — it's a rocket, not an
//! engine, so it works identically grounded or airborne, the same way it
//! does in real Rocket League.
//!
//! Handbrake is modeled as a temporary ground-friction reduction rather
//! than a separate lateral-slip system: this port has no per-wheel tire
//! model (the car is one rigid box), so there's no distinct "rear grip"
//! to lose the way a real car's handbrake works. Instead, while `handbrake`
//! is held and the car is grounded, `RigidBody.friction` — the same
//! material property the ground-contact solver already reads for Coulomb
//! friction — is temporarily reduced, letting the box's existing momentum
//! carry it into a slide instead of gripping the ground and turning
//! cleanly. Releasing handbrake restores the car's original friction. This
//! reuses machinery the solver already has rather than inventing a second
//! grip model.
//!
//! **Not implemented** (tracked in `RB-PHYSICS-001`, not silently
//! dropped): jump and air-control (pitch/yaw/roll torque while airborne).
//! A car with no input set (or all-neutral `ControllerInput::default()`)
//! behaves exactly as a free rigid box always has — this module only ever
//! adds force/torque or adjusts the existing friction property, never
//! removes physics outright.
//!
//! This is not a Bullet3 port (Bullet has no concept of "a car's engine")
//! — it's this project's own model of Rocket League's driving mechanics,
//! since the real numbers are not public. `MAX_CAR_SPEED`, `MAX_BOOST`,
//! and `BOOST_ACCELERATION` are commonly-cited community-reverse-engineered
//! approximations (the same body of public research `PhysicsWorld::new`'s
//! gravity constant comes from); `THROTTLE_ACCELERATION` and
//! `BOOST_CONSUMPTION_RATE` are simplified constants standing in for
//! Rocket League's real speed-dependent throttle curve and boost-drain
//! behavior; `STEER_TORQUE` and `HANDBRAKE_FRICTION_MULTIPLIER` are
//! uncalibrated placeholders chosen only to produce a visibly responsive
//! turn/slide for this car's mass/inertia in tests. None of these are
//! independently confirmed by this project — see `RB-PHYSICS-001-FR-005`.

use crate::body::RigidBody;
use rb_domain::{ControllerInput, Vec3};

/// Commonly-cited approximate max ground speed a car's engine alone can
/// reach (uu/s) — Rocket League's unboosted top speed. Also used here as
/// boost's own speed cap (real Rocket League's actual top speed, boosted
/// or not, is the same number) — a simplification, since throttle and
/// boost don't share one real top speed, but this port doesn't yet model
/// two separate ceilings.
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

/// Commonly-cited approximate boost acceleration (uu/s^2), a flat
/// constant (unlike throttle, boost doesn't taper with speed in real
/// Rocket League either).
const BOOST_ACCELERATION: f32 = 991.667;

/// Commonly-cited full boost tank size, in the same units `ControllerInput`
/// and `CarState::boost_amount` use.
pub const MAX_BOOST: f32 = 100.0;

/// Simplified constant boost drain rate (units/s) while `boost` is held —
/// a full tank lasting ~3 seconds nonstop is the commonly-cited number
/// this approximates; real Rocket League's actual drain behavior around
/// zero-throttle/wavedash edge cases isn't modeled.
const BOOST_CONSUMPTION_RATE: f32 = 33.3;

/// Uncalibrated placeholder: while grounded and `handbrake` is held, the
/// car's `RigidBody.friction` is multiplied by this factor before the
/// ground-contact solver runs, sharply reducing grip so existing momentum
/// carries the car into a slide instead of a clean turn. Chosen only to
/// produce a visibly reduced (not zero) grip in tests, not derived from any
/// measured or documented Rocket League value — this port has no per-wheel
/// tire model to calibrate a real rear-grip-loss number against.
const HANDBRAKE_FRICTION_MULTIPLIER: f32 = 0.1;

fn forward_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(1.0, 0.0, 0.0))
}

fn up_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(0.0, 0.0, 1.0))
}

/// Applies throttle, steering, boost, and handbrake as forces/torques (or,
/// for handbrake, a temporary friction adjustment) on `car`. Throttle,
/// steering, and handbrake are a no-op unless `on_ground`; boost isn't
/// gated on ground contact, but is a no-op once `*boost_amount` reaches
/// zero. `base_friction` is the car's own nominal (non-handbraking)
/// friction — handbrake temporarily reduces `car.friction` below it while
/// held and grounded, and restores it otherwise, so callers don't need a
/// separate restore step. Call once per step, before
/// `integrate::integrate_velocities`, alongside `apply_gravity`.
pub fn apply_driven_forces(
    car: &mut RigidBody,
    input: &ControllerInput,
    on_ground: bool,
    boost_amount: &mut f32,
    base_friction: f32,
    dt: f32,
) {
    let forward = forward_axis(car);

    if on_ground {
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

        car.friction = if input.handbrake {
            base_friction * HANDBRAKE_FRICTION_MULTIPLIER
        } else {
            base_friction
        };
    }

    if input.boost && *boost_amount > 0.0 {
        let forward_speed = car.linear_velocity.dot(&forward);
        if forward_speed < MAX_CAR_SPEED {
            car.apply_central_force(forward * (BOOST_ACCELERATION * car.mass()));
        }
        // Held boost drains the tank even when the force above didn't
        // apply (e.g. already at MAX_CAR_SPEED, or pushing into a wall) —
        // matching real Rocket League, where holding boost costs fuel
        // regardless of whether it's doing anything.
        *boost_amount = (*boost_amount - BOOST_CONSUMPTION_RATE * dt).max(0.0);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::integrate;

    const DEFAULT_TEST_FRICTION: f32 = 0.5;

    fn car() -> RigidBody {
        RigidBody::car_box(Vec3::new(60.0, 30.0, 18.0), 180.0, Vec3::ZERO)
    }

    fn step_with_input(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        boost_amount: &mut f32,
        dt: f32,
    ) {
        car.clear_forces();
        apply_driven_forces(
            car,
            input,
            on_ground,
            boost_amount,
            DEFAULT_TEST_FRICTION,
            dt,
        );
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

    fn full_boost() -> ControllerInput {
        ControllerInput {
            boost: true,
            ..Default::default()
        }
    }

    fn full_handbrake() -> ControllerInput {
        ControllerInput {
            handbrake: true,
            ..Default::default()
        }
    }

    #[test]
    fn neutral_input_applies_no_force_or_torque() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(
            &mut c,
            &ControllerInput::default(),
            true,
            &mut boost,
            1.0 / 60.0,
        );
        assert_eq!(c.linear_velocity, Vec3::ZERO);
        assert_eq!(c.angular_velocity, Vec3::ZERO);
        assert_eq!(boost, MAX_BOOST, "unused boost shouldn't drain");
        assert_eq!(
            c.friction, DEFAULT_TEST_FRICTION,
            "no handbrake input shouldn't touch friction"
        );
    }

    #[test]
    fn throttle_accelerates_a_grounded_car_forward() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        for _ in 0..60 {
            step_with_input(&mut c, &full_throttle(), true, &mut boost, 1.0 / 60.0);
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
        let mut boost = MAX_BOOST;
        for _ in 0..60 {
            step_with_input(&mut c, &full_throttle(), false, &mut boost, 1.0 / 60.0);
        }
        assert_eq!(
            c.linear_velocity.x, 0.0,
            "airborne throttle shouldn't add forward speed"
        );
    }

    #[test]
    fn throttle_stops_accelerating_at_max_speed() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        c.linear_velocity = Vec3::new(MAX_CAR_SPEED, 0.0, 0.0);
        step_with_input(&mut c, &full_throttle(), true, &mut boost, 1.0 / 60.0);
        assert!(
            (c.linear_velocity.x - MAX_CAR_SPEED).abs() < 1e-4,
            "expected throttle to stop pushing past MAX_CAR_SPEED, got {}",
            c.linear_velocity.x
        );
    }

    #[test]
    fn reverse_throttle_accelerates_backward() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let input = ControllerInput {
            throttle: -1.0,
            ..Default::default()
        };
        for _ in 0..60 {
            step_with_input(&mut c, &input, true, &mut boost, 1.0 / 60.0);
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
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_steer(), true, &mut boost, 1.0 / 60.0);
        assert_eq!(
            c.angular_velocity,
            Vec3::ZERO,
            "a parked car shouldn't be able to turn in place"
        );
    }

    #[test]
    fn steer_yaws_a_moving_car() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        c.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
        step_with_input(&mut c, &full_steer(), true, &mut boost, 1.0 / 60.0);
        assert!(
            c.angular_velocity.z.abs() > 0.0,
            "expected a moving car to yaw under full steer, got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn opposite_steer_yaws_the_opposite_way() {
        let mut left = car();
        let mut left_boost = MAX_BOOST;
        left.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
        step_with_input(&mut left, &full_steer(), true, &mut left_boost, 1.0 / 60.0);

        let mut right = car();
        let mut right_boost = MAX_BOOST;
        right.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
        let opposite = ControllerInput {
            steer: -1.0,
            ..Default::default()
        };
        step_with_input(&mut right, &opposite, true, &mut right_boost, 1.0 / 60.0);

        assert!(left.angular_velocity.z * right.angular_velocity.z < 0.0);
    }

    #[test]
    fn boost_accelerates_a_car_regardless_of_ground_contact() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        for _ in 0..60 {
            step_with_input(&mut c, &full_boost(), false, &mut boost, 1.0 / 60.0);
        }
        assert!(
            c.linear_velocity.x > 0.0,
            "expected boost to accelerate an airborne car, got {}",
            c.linear_velocity.x
        );
    }

    #[test]
    fn boost_drains_the_tank_over_time_and_clamps_at_zero() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        for _ in 0..600 {
            step_with_input(&mut c, &full_boost(), false, &mut boost, 1.0 / 60.0);
        }
        assert_eq!(boost, 0.0, "expected a full tank to run out within 10s");
    }

    #[test]
    fn boost_has_no_effect_when_the_tank_is_empty() {
        let mut c = car();
        let mut boost = 0.0;
        step_with_input(&mut c, &full_boost(), false, &mut boost, 1.0 / 60.0);
        assert_eq!(c.linear_velocity, Vec3::ZERO);
        assert_eq!(boost, 0.0);
    }

    #[test]
    fn boost_still_drains_at_max_speed_even_though_it_stops_accelerating() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        c.linear_velocity = Vec3::new(MAX_CAR_SPEED, 0.0, 0.0);
        step_with_input(&mut c, &full_boost(), false, &mut boost, 1.0 / 60.0);
        assert!(
            (c.linear_velocity.x - MAX_CAR_SPEED).abs() < 1e-4,
            "expected boost to stop pushing past MAX_CAR_SPEED, got {}",
            c.linear_velocity.x
        );
        assert!(
            boost < MAX_BOOST,
            "expected held boost to still drain even at max speed"
        );
    }

    #[test]
    fn handbrake_reduces_friction_while_grounded() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_handbrake(), true, &mut boost, 1.0 / 60.0);
        assert_eq!(
            c.friction,
            DEFAULT_TEST_FRICTION * HANDBRAKE_FRICTION_MULTIPLIER,
            "expected handbrake to reduce friction below its base value"
        );
    }

    #[test]
    fn handbrake_has_no_effect_on_friction_while_airborne() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_handbrake(), false, &mut boost, 1.0 / 60.0);
        assert_eq!(
            c.friction, DEFAULT_TEST_FRICTION,
            "airborne handbrake shouldn't touch friction — no wheels to lock"
        );
    }

    #[test]
    fn releasing_handbrake_restores_friction() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_handbrake(), true, &mut boost, 1.0 / 60.0);
        assert!(
            c.friction < DEFAULT_TEST_FRICTION,
            "handbrake should have engaged"
        );
        step_with_input(
            &mut c,
            &ControllerInput::default(),
            true,
            &mut boost,
            1.0 / 60.0,
        );
        assert_eq!(
            c.friction, DEFAULT_TEST_FRICTION,
            "releasing handbrake should restore the car's base friction"
        );
    }
}
