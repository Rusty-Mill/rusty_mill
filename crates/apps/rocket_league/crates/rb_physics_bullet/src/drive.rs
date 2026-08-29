//! Driven car input: couples `rb_domain::ControllerInput` into forces and
//! torques on a car `RigidBody`. Ground-driving (throttle, steering),
//! boost, handbrake/drift, a single ground jump, and air control in this
//! increment. Throttle, steering, handbrake, and jump are gated on the car
//! actually touching the ground (a free-floating box has no wheels to
//! grip, lock, or push off of, so airborne input does nothing for any of
//! them here); boost is not — it's a rocket, not an engine, so it works
//! identically grounded or airborne, the same way it does in real Rocket
//! League. Air control is the mirror image: gated on the car *not*
//! touching the ground (real air control needs no wheels at all — it's
//! pure torque, so it would be redundant with steering while grounded).
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
//! Jump is a single, fixed-height vertical impulse fired on the *rising
//! edge* of `ControllerInput.jump` (a fresh press, not merely "held") while
//! grounded — holding the button through the resulting airborne period
//! doesn't re-fire it, and releasing then re-pressing while still airborne
//! doesn't fire it either (this increment has no double jump to grant).
//! Edge detection needs one bit of state to remember "was jump held as of
//! last step," carried by the caller (`PhysicsWorld::car_jump_held`) and
//! passed in as `jump_held`, the same pattern `boost_amount` already uses
//! for a resource that must persist across calls.
//!
//! Pitch, yaw, and roll each apply torque about one of the car's three
//! local axes (right, up, forward respectively), scaled directly by the
//! analog `Option<f32>` value — `None` (an input source that can't recover
//! an analog value, e.g. replay-derived input, see `rb_domain`) is treated
//! as zero, same as a centered stick. Unlike ground steering, air control
//! isn't speed-scaled: a car can spin freely from a standing start in the
//! air, since there's no wheel grip to require momentum for. All three
//! axes share one `AIR_CONTROL_TORQUE` constant — a real simplification,
//! since Rocket League's actual pitch/yaw/roll rates differ from each
//! other; this port doesn't model that difference.
//!
//! Double jump reuses the ground jump's own rising-edge detection
//! (`jump_pressed`) rather than a second edge-detector: the same fresh
//! press, while airborne, fires one more instantaneous `JUMP_SPEED`
//! impulse — gated on a per-car `double_jump_available` flag instead of on
//! `on_ground`. Landing (any step where `on_ground` is true) unconditionally
//! restores availability; an airborne fresh press consumes it, so it can
//! fire at most once per airborne period no matter how many times jump is
//! released and re-pressed after that. This deliberately excludes the
//! "dodge" directional flip real Rocket League pairs a double jump with (a
//! sideways/forward impulse and torque from the stick direction at the
//! moment of the second press) — that's a distinct, still-unimplemented
//! mechanic, not folded into this increment.
//!
//! **Not implemented** (tracked in `RB-PHYSICS-001`, not silently
//! dropped): the dodge directional impulse/torque a real double jump pairs
//! with (see above), variable jump height (real Rocket League adds extra
//! upward accel for as long as jump is held, up to a cap — this port
//! always applies the same fixed impulse regardless of how long the
//! button is held), and wall jump (needs arena walls, which don't exist in
//! this scope). A car with no input set (or all-neutral
//! `ControllerInput::default()`) behaves exactly as a free rigid box
//! always has — this module only ever adds force/torque/impulse or
//! adjusts the existing friction property, never removes physics outright.
//!
//! This is not a Bullet3 port (Bullet has no concept of "a car's engine")
//! — it's this project's own model of Rocket League's driving mechanics,
//! since the real numbers are not public. `MAX_CAR_SPEED`, `MAX_BOOST`,
//! `BOOST_ACCELERATION`, and `JUMP_SPEED` are commonly-cited
//! community-reverse-engineered approximations (the same body of public
//! research `PhysicsWorld::new`'s gravity constant comes from);
//! `THROTTLE_ACCELERATION` and `BOOST_CONSUMPTION_RATE` are simplified
//! constants standing in for Rocket League's real speed-dependent throttle
//! curve and boost-drain behavior; `STEER_TORQUE`,
//! `HANDBRAKE_FRICTION_MULTIPLIER`, and `AIR_CONTROL_TORQUE` are
//! uncalibrated placeholders chosen only to produce a visibly responsive
//! turn/slide/spin for this car's mass/inertia in tests. None of these are
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

/// Commonly-cited approximate jump impulse speed (uu/s), applied as an
/// instantaneous vertical velocity change (not a continuous force) on a
/// fresh grounded jump press — a flat speed regardless of the car's mass,
/// matching how the real jump impulse doesn't scale with car mass either.
/// Also reused as the double jump's impulse magnitude (see the module doc
/// comment) — `pub` so `world.rs`'s end-to-end tests can assert against it
/// directly, the same way `MAX_CAR_SPEED`/`MAX_BOOST` already are.
pub const JUMP_SPEED: f32 = 292.0;

/// Uncalibrated placeholder air-control torque magnitude, shared by pitch,
/// yaw, and roll (about the car's local right, up, and forward axes
/// respectively) at full analog input — chosen only so a full-stick
/// rotation is visibly responsive for this car's mass/inertia in tests,
/// not derived from any measured or documented Rocket League value. Real
/// Rocket League's pitch/yaw/roll rates differ from each other; this port
/// doesn't model that difference.
const AIR_CONTROL_TORQUE: f32 = 1_000_000.0;

fn forward_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(1.0, 0.0, 0.0))
}

fn up_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(0.0, 0.0, 1.0))
}

/// The car's local "right" axis (local +Y) — completes the right-handed
/// (forward, right, up) basis `up_axis × forward_axis` gives. Used only for
/// pitch torque (nose up/down about this axis); throttle/steer/boost/
/// handbrake/jump never need it.
fn right_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(0.0, 1.0, 0.0))
}

/// Applies throttle, steering, boost, handbrake, jump, double jump, and air
/// control as forces/torques/impulses (or, for handbrake, a temporary
/// friction adjustment) on `car`. Throttle, steering, handbrake, and jump
/// are a no-op unless `on_ground`; air control and double jump are the
/// reverse — a no-op unless *not* `on_ground`; boost isn't gated on ground
/// contact at all, but is a no-op once `*boost_amount` reaches zero.
/// `base_friction` is the car's own nominal (non-handbraking) friction —
/// handbrake temporarily reduces `car.friction` below it while held and
/// grounded, and restores it otherwise, so callers don't need a separate
/// restore step. `jump_held` is the car's `input.jump` value as of the
/// *previous* call — jump (ground or double) fires only on a rising edge
/// (`input.jump && !*jump_held`), so a continued press doesn't re-fire
/// every step; it's updated to `input.jump` on every call, including while
/// airborne, so a fresh press is still required for a double jump even if
/// the button was never released after the ground jump. `double_jump_available`
/// is whether the car still has a double jump to spend this airborne
/// period — landing (`on_ground`) unconditionally sets it back to `true`;
/// a fresh airborne press that fires the double jump sets it to `false`
/// until the next landing. Call once per step, before
/// `integrate::integrate_velocities`, alongside `apply_gravity`.
#[allow(clippy::too_many_arguments)]
pub fn apply_driven_forces(
    car: &mut RigidBody,
    input: &ControllerInput,
    on_ground: bool,
    boost_amount: &mut f32,
    jump_held: &mut bool,
    double_jump_available: &mut bool,
    base_friction: f32,
    dt: f32,
) {
    let forward = forward_axis(car);
    let jump_pressed = input.jump && !*jump_held;
    *jump_held = input.jump;

    if on_ground {
        // Landing (or simply resting) always restores the double jump,
        // regardless of this step's input.
        *double_jump_available = true;

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

        if jump_pressed {
            // An instantaneous velocity change, not a continuous force —
            // apply_impulse divides by mass internally, so scaling by
            // car.mass() here cancels that out and yields a flat
            // JUMP_SPEED velocity change regardless of the car's mass.
            car.apply_impulse(Vec3::new(0.0, 0.0, JUMP_SPEED * car.mass()), Vec3::ZERO);
        }
    } else {
        // Air control: pitch/yaw/roll torque about the car's local
        // right/up/forward axes. Unlike ground steering, not scaled by
        // speed — a car can spin from a standing start in the air, since
        // there's no wheel grip to require momentum for.
        let pitch = input.pitch.unwrap_or(0.0).clamp(-1.0, 1.0);
        if pitch != 0.0 {
            car.apply_torque(right_axis(car) * (pitch * AIR_CONTROL_TORQUE));
        }

        let yaw = input.yaw.unwrap_or(0.0).clamp(-1.0, 1.0);
        if yaw != 0.0 {
            car.apply_torque(up_axis(car) * (yaw * AIR_CONTROL_TORQUE));
        }

        let roll = input.roll.unwrap_or(0.0).clamp(-1.0, 1.0);
        if roll != 0.0 {
            car.apply_torque(forward * (roll * AIR_CONTROL_TORQUE));
        }

        if jump_pressed && *double_jump_available {
            // Same fixed-magnitude impulse as the ground jump — reusing
            // JUMP_SPEED rather than a second, separately-calibrated
            // constant, since this port has no public reference for a
            // distinct double-jump speed either.
            car.apply_impulse(Vec3::new(0.0, 0.0, JUMP_SPEED * car.mass()), Vec3::ZERO);
            *double_jump_available = false;
        }
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
        let mut jump_held = false;
        step_with_input_and_jump_state(car, input, on_ground, boost_amount, &mut jump_held, dt);
    }

    fn step_with_input_and_jump_state(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        boost_amount: &mut f32,
        jump_held: &mut bool,
        dt: f32,
    ) {
        let mut double_jump_available = true;
        step_with_input_and_double_jump_state(
            car,
            input,
            on_ground,
            boost_amount,
            jump_held,
            &mut double_jump_available,
            dt,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn step_with_input_and_double_jump_state(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        boost_amount: &mut f32,
        jump_held: &mut bool,
        double_jump_available: &mut bool,
        dt: f32,
    ) {
        car.clear_forces();
        apply_driven_forces(
            car,
            input,
            on_ground,
            boost_amount,
            jump_held,
            double_jump_available,
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

    fn full_jump() -> ControllerInput {
        ControllerInput {
            jump: true,
            ..Default::default()
        }
    }

    fn full_pitch() -> ControllerInput {
        ControllerInput {
            pitch: Some(1.0),
            ..Default::default()
        }
    }

    fn full_yaw() -> ControllerInput {
        ControllerInput {
            yaw: Some(1.0),
            ..Default::default()
        }
    }

    fn full_roll() -> ControllerInput {
        ControllerInput {
            roll: Some(1.0),
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

    #[test]
    fn jump_gives_a_grounded_car_upward_velocity() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_jump(), true, &mut boost, 1.0 / 60.0);
        assert!(
            (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
            "expected roughly JUMP_SPEED upward velocity, got {}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn holding_jump_does_not_refire_every_step() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        step_with_input_and_jump_state(
            &mut c,
            &full_jump(),
            true,
            &mut boost,
            &mut jump_held,
            1.0 / 60.0,
        );
        let velocity_after_first_press = c.linear_velocity.z;
        // Still held, still (nominally) grounded — a second call with the
        // same jump_held state must not add a second impulse.
        step_with_input_and_jump_state(
            &mut c,
            &full_jump(),
            true,
            &mut boost,
            &mut jump_held,
            1.0 / 60.0,
        );
        assert!(
            c.linear_velocity.z <= velocity_after_first_press + 1.0,
            "expected holding jump to not re-fire a second impulse, \
             velocity after first press={velocity_after_first_press}, after second={}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn releasing_and_repressing_jump_fires_again() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        step_with_input_and_jump_state(
            &mut c,
            &full_jump(),
            true,
            &mut boost,
            &mut jump_held,
            1.0 / 60.0,
        );
        let velocity_after_first_press = c.linear_velocity.z;
        // Release, then press again — this must fire a second impulse.
        step_with_input_and_jump_state(
            &mut c,
            &ControllerInput::default(),
            true,
            &mut boost,
            &mut jump_held,
            1.0 / 60.0,
        );
        step_with_input_and_jump_state(
            &mut c,
            &full_jump(),
            true,
            &mut boost,
            &mut jump_held,
            1.0 / 60.0,
        );
        assert!(
            c.linear_velocity.z > velocity_after_first_press,
            "expected releasing then re-pressing jump to fire again, \
             velocity after first press={velocity_after_first_press}, after second press={}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn double_jump_gives_an_airborne_car_upward_velocity_when_available() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        // step_with_input's throwaway jump_held/double_jump_available both
        // start fresh (unheld, available), so a single airborne jump press
        // here is exactly the "double jump available" case.
        step_with_input(&mut c, &full_jump(), false, &mut boost, 1.0 / 60.0);
        assert!(
            (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
            "expected roughly JUMP_SPEED upward velocity from an available double jump, got {}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn double_jump_has_no_effect_when_unavailable() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = false;
        step_with_input_and_double_jump_state(
            &mut c,
            &full_jump(),
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert_eq!(
            c.linear_velocity.z, 0.0,
            "expected an unavailable double jump to add no upward velocity"
        );
    }

    #[test]
    fn double_jump_is_consumed_after_use_and_does_not_refire() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        step_with_input_and_double_jump_state(
            &mut c,
            &full_jump(),
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        let velocity_after_double_jump = c.linear_velocity.z;
        assert!(
            !double_jump_available,
            "expected using the double jump to consume it"
        );

        // Release, then press again while still airborne — must not fire a
        // second impulse now that the double jump is spent.
        step_with_input_and_double_jump_state(
            &mut c,
            &ControllerInput::default(),
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        step_with_input_and_double_jump_state(
            &mut c,
            &full_jump(),
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            (c.linear_velocity.z - velocity_after_double_jump).abs() < 1.0,
            "expected a spent double jump to not refire, velocity after first use=\
             {velocity_after_double_jump}, after second press={}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn landing_restores_double_jump_availability() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = false;
        step_with_input_and_double_jump_state(
            &mut c,
            &ControllerInput::default(),
            true,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            double_jump_available,
            "expected touching the ground to restore the double jump"
        );
    }

    #[test]
    fn air_control_has_no_effect_while_grounded() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let input = ControllerInput {
            pitch: Some(1.0),
            yaw: Some(1.0),
            roll: Some(1.0),
            ..Default::default()
        };
        step_with_input(&mut c, &input, true, &mut boost, 1.0 / 60.0);
        assert_eq!(
            c.angular_velocity,
            Vec3::ZERO,
            "grounded air control shouldn't spin the car — steering already owns yaw on the ground"
        );
    }

    #[test]
    fn a_stationary_airborne_car_can_pitch_yaw_and_roll() {
        // Unlike ground steering, air control isn't speed-scaled — a car
        // with zero velocity should still spin freely.
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_pitch(), false, &mut boost, 1.0 / 60.0);
        assert!(
            c.angular_velocity.y.abs() > 0.0,
            "expected pitch to produce angular velocity about the local right (Y) axis, got {:?}",
            c.angular_velocity
        );

        let mut c = car();
        step_with_input(&mut c, &full_yaw(), false, &mut boost, 1.0 / 60.0);
        assert!(
            c.angular_velocity.z.abs() > 0.0,
            "expected yaw to produce angular velocity about the local up (Z) axis, got {:?}",
            c.angular_velocity
        );

        let mut c = car();
        step_with_input(&mut c, &full_roll(), false, &mut boost, 1.0 / 60.0);
        assert!(
            c.angular_velocity.x.abs() > 0.0,
            "expected roll to produce angular velocity about the local forward (X) axis, got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn no_analog_value_is_treated_as_neutral_air_control() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(
            &mut c,
            &ControllerInput::default(),
            false,
            &mut boost,
            1.0 / 60.0,
        );
        assert_eq!(
            c.angular_velocity,
            Vec3::ZERO,
            "pitch/yaw/roll all None (e.g. replay-derived input) should behave like neutral input"
        );
    }

    #[test]
    fn opposite_yaw_spins_the_opposite_way_in_the_air() {
        let mut left = car();
        let mut left_boost = MAX_BOOST;
        step_with_input(&mut left, &full_yaw(), false, &mut left_boost, 1.0 / 60.0);

        let mut right = car();
        let mut right_boost = MAX_BOOST;
        let opposite = ControllerInput {
            yaw: Some(-1.0),
            ..Default::default()
        };
        step_with_input(&mut right, &opposite, false, &mut right_boost, 1.0 / 60.0);

        assert!(left.angular_velocity.z * right.angular_velocity.z < 0.0);
    }
}
