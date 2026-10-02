//! Boost: forward acceleration (grounded vs airborne magnitude) and tank
//! drain. Not gated on ground contact.

use super::MAX_CAR_SPEED;
use crate::body::RigidBody;
use rb_domain::Vec3;

/// Boost acceleration while grounded (uu/s^2) — unlike throttle, boost
/// doesn't taper with speed in real Rocket League, so this (like
/// `BOOST_ACCELERATION_AIR` below) is a flat constant, not a curve.
/// Confirmed exact against RocketSim's own `RLConst.h`
/// (`BOOST_ACCEL_GROUND = 2975.f / 3.f`, fetched directly during
/// `RB-PHYSICS-001-FR-056`) — written as the same fraction the reference
/// uses, matching `JUMP_SPEED`'s own precedent for an exact fractional
/// source value, rather than that fraction's earlier `991.667` decimal
/// approximation (the two are equal to float precision; this is a
/// clarity change, not a value change).
pub(super) const BOOST_ACCELERATION_GROUND: f32 = 2975.0 / 3.0;

/// Boost acceleration while airborne (uu/s^2) — genuinely different from
/// `BOOST_ACCELERATION_GROUND`, not a rounding of the same number.
/// `RB-PHYSICS-001-FR-056` fetched RocketSim's own `RLConst.h` directly
/// and found `BOOST_ACCEL_AIR = 3175.f / 3.f`, distinctly higher than the
/// grounded value — a split this port's own earlier single flat
/// `BOOST_ACCELERATION` constant didn't model at all (every airborne
/// boost this crate ever applied used the *grounded* number, understating
/// real airborne boost strength by about 6.5%). `apply_driven_forces`
/// now selects between the two by `on_ground`, matching the reference
/// split exactly — a genuine behavioral fix, not just a doc correction,
/// found via the same "fetch primary source directly" method this
/// project already applies throughout (see `RB-PHYSICS-001-FR-031`'s own
/// audit and every reference-validation FR since).
pub(super) const BOOST_ACCELERATION_AIR: f32 = 3175.0 / 3.0;

/// Commonly-cited full boost tank size, in the same units `ControllerInput`
/// and `CarState::boost_amount` use.
pub const MAX_BOOST: f32 = 100.0;

/// Default boost drain rate (units/s) while `boost` is held, RocketSim's
/// `MutatorConfig::boostUsedPerSecond` default — a full tank lasts ~3
/// seconds nonstop. Unlimited-boost freeplay is a rate of 0
/// (`PhysicsWorld::set_boost_used_per_second`, `RB-PHYSICS-001-FR-111`).
pub const BOOST_USED_PER_SECOND: f32 = 33.3;

/// Applies boost acceleration along `forward` while `boost_held` and the
/// tank isn't empty, below `MAX_CAR_SPEED`, and drains the tank by
/// `used_per_second` regardless of whether the force applied.
pub(super) fn apply_boost(
    car: &mut RigidBody,
    boost_held: bool,
    on_ground: bool,
    forward: Vec3,
    boost_amount: &mut f32,
    used_per_second: f32,
    dt: f32,
) {
    if !(boost_held && *boost_amount > 0.0) {
        return;
    }
    let forward_speed = car.linear_velocity.dot(&forward);
    if forward_speed < MAX_CAR_SPEED {
        // RB-PHYSICS-001-FR-056: real Rocket League's own boost
        // acceleration is genuinely higher airborne than grounded —
        // `on_ground` gates which magnitude applies, not whether
        // boost applies at all (it always does, regardless of ground
        // contact, per `apply_driven_forces`'s own doc comment).
        let boost_acceleration = if on_ground {
            BOOST_ACCELERATION_GROUND
        } else {
            BOOST_ACCELERATION_AIR
        };
        car.apply_central_force(forward * (boost_acceleration * car.mass()));
    }
    // Held boost drains the tank even when the force above didn't
    // apply (e.g. already at MAX_CAR_SPEED, or pushing into a wall) —
    // matching real Rocket League, where holding boost costs fuel
    // regardless of whether it's doing anything.
    *boost_amount = (*boost_amount - used_per_second * dt).max(0.0);
}
