//! Ground driving: throttle (with its speed taper), steering, handbrake, and
//! the ground jump's initial impulse. Every function here assumes the car
//! is on the ground; `super::apply_driven_forces` does the gating.

use super::{up_axis, UNBOOSTED_MAX_CAR_SPEED};
use crate::body::RigidBody;
use rb_domain::{ControllerInput, Vec3};

/// Peak throttle acceleration (uu/s^2), at a standing start — still an
/// uncalibrated placeholder pending calibration against recorded data
/// (unlike the taper shape it's scaled by, see `drive_speed_taper` below,
/// this magnitude itself has no confirmed real-world source). Since
/// `RB-PHYSICS-001-FR-058`, this is no longer applied flat: it's scaled by
/// `drive_speed_taper`'s own real curve as speed rises, tapering smoothly
/// to zero at `UNBOOSTED_MAX_CAR_SPEED` instead of applying at full
/// strength right up to a hard cutoff.
pub(super) const THROTTLE_ACCELERATION: f32 = 1600.0;

/// Real Rocket League's own speed-dependent drive-force taper —
/// RocketSim's `DRIVE_SPEED_TORQUE_FACTOR_CURVE`, a 3-point piecewise-
/// linear curve confirmed exact against its own `RLConst.h` during
/// `RB-PHYSICS-001-FR-058`'s audit: full torque from a standing start
/// (`(0, 1.0)`), tapering linearly down to 10% by `1400` uu/s
/// (`(1400, 0.1)`), then a final, much steeper linear drop to exactly
/// zero at `UNBOOSTED_MAX_CAR_SPEED` (`(1410, 0.0)`). Fetching RocketSim's
/// own `Car.cpp` confirmed this curve is looked up by the car's *signed*
/// forward speed (`abs()`'d there, since real RocketSim's own throttle
/// gate isn't direction-aware) and multiplied directly against the drive
/// force applied to each wheel — a pure, unitless ratio, unlike
/// `THROTTLE_TORQUE_AMOUNT` (RocketSim's own name for this project's
/// `THROTTLE_ACCELERATION`), which is expressed in Bullet's own internal
/// mass/distance units and doesn't transfer to this port's own
/// differently-calibrated car body the same clean way — see
/// `RB-PHYSICS-001-FR-031`'s and `FR-057`'s own "false precision" findings
/// for absolute torque/force magnitudes. Only the curve's *shape* is
/// adopted here, not a new peak magnitude.
pub(super) const DRIVE_SPEED_TAPER_BREAKPOINTS: [(f32, f32); 3] =
    [(0.0, 1.0), (1400.0, 0.1), (UNBOOSTED_MAX_CAR_SPEED, 0.0)];

/// Linearly interpolates `DRIVE_SPEED_TAPER_BREAKPOINTS` at
/// `signed_speed_in_throttle_direction` (the same
/// `throttle.signum() * forward_speed` quantity `apply_driven_forces`'s
/// own throttle gate already computes) — `1.0` (full acceleration) at or
/// below the first breakpoint, `0.0` at or beyond the last. Deliberately
/// evaluated against this port's own pre-existing *signed*,
/// direction-aware speed (clamped to non-negative here, since a negative
/// value means "not yet moving this way," which should read as a
/// standing start, not an out-of-range lookup) rather than switching to
/// real RocketSim's own direction-agnostic `abs(forward speed)` — that
/// would be a second, independent behavioral change (whether accelerating
/// against your own current motion tapers too) this requirement doesn't
/// take on; see its own Non-goals.
pub(super) fn drive_speed_taper(signed_speed_in_throttle_direction: f32) -> f32 {
    curve(
        &DRIVE_SPEED_TAPER_BREAKPOINTS,
        signed_speed_in_throttle_direction.max(0.0),
    )
}

/// Piecewise-linear lookup of `x` in `points` (sorted by x), clamped to the
/// first and last values outside their range — the evaluation RocketSim's
/// own `LinearPieceCurve` performs for every `RLConst.h` curve this module
/// ports.
pub(super) fn curve(points: &[(f32, f32)], x: f32) -> f32 {
    let (Some(&(x_first, y_first)), Some(&(_, y_last))) = (points.first(), points.last()) else {
        return 0.0;
    };
    if x <= x_first {
        return y_first;
    }
    for window in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (window[0], window[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    y_last
}

/// Real Rocket League's maximum front-wheel steer angle (rad) by forward
/// speed (uu/s): RocketSim's `STEER_ANGLE_FROM_SPEED_CURVE` (`RLConst.h`),
/// tightest from a standstill and only gentle at speed.
pub(super) const STEER_ANGLE_FROM_SPEED_CURVE: [(f32, f32); 6] = [
    (0.0, 0.53356),
    (500.0, 0.31930),
    (1000.0, 0.18203),
    (1500.0, 0.10570),
    (1750.0, 0.08507),
    (3000.0, 0.03454),
];

/// The same, while the handbrake is held (powersliding): RocketSim's
/// `POWERSLIDE_STEER_ANGLE_FROM_SPEED_CURVE`.
pub(super) const POWERSLIDE_STEER_ANGLE_FROM_SPEED_CURVE: [(f32, f32); 2] =
    [(0.0, 0.39235), (2500.0, 0.12610)];

/// Octane wheelbase (uu): front axle `+51.25` to rear axle `-33.75` along
/// the car's local forward axis (RocketSim `CarConfig.cpp`,
/// `CAR_CONFIG_OCTANE` wheel connection points).
pub(super) const WHEELBASE: f32 = 51.25 + 33.75;

/// Yaw rate (rad/s, about the car's up axis) a grounded car at
/// `forward_speed` turns at with `steer` held — `RB-PHYSICS-001-FR-080`.
///
/// Real Rocket League steers by angling the front wheels (the steer-angle
/// curves above) and letting per-wheel tire friction turn the car
/// (`btVehicleRL`, `RB-PHYSICS-001-FR-065`). This port has no wheels, so it
/// uses the kinematic bicycle model that geometry implies without slip:
/// `yaw_rate = forward_speed * tan(steer_angle) / WHEELBASE`. Signed
/// `forward_speed` makes reversing turn the other way, and zero speed gives
/// zero yaw (no turning in place). Tire slip is not modeled, so this is an
/// upper bound on how fast the real car's heading turns (ADR-0011).
pub(super) fn steer_yaw_rate(forward_speed: f32, steer: f32, handbrake: bool) -> f32 {
    let max_angle = if handbrake {
        curve(
            &POWERSLIDE_STEER_ANGLE_FROM_SPEED_CURVE,
            forward_speed.abs(),
        )
    } else {
        curve(&STEER_ANGLE_FROM_SPEED_CURVE, forward_speed.abs())
    };
    forward_speed * (steer.clamp(-1.0, 1.0) * max_angle).tan() / WHEELBASE
}

/// Uncalibrated placeholder: while grounded and `handbrake` is held, the
/// car's `RigidBody.friction` is multiplied by this factor before the
/// ground-contact solver runs, sharply reducing grip so existing momentum
/// carries the car into a slide instead of a clean turn. Chosen only to
/// produce a visibly reduced (not zero) grip in tests, not derived from any
/// measured or documented Rocket League value — this port has no per-wheel
/// tire model to calibrate a real rear-grip-loss number against.
///
/// `RB-PHYSICS-001-FR-066` fetched RocketSim's real `Car.cpp`
/// (`_UpdateWheels`, matching `RB-PHYSICS-001-FR-058`/`FR-059`/`FR-064`/
/// `FR-065`'s own real-implementation-file method) and found real Rocket
/// League's own handbrake friction reduction is genuinely anisotropic, not
/// a single uniform multiplier: two separate confirmed real curves,
/// `HANDBRAKE_LAT_FRICTION_FACTOR_CURVE` (`0.1` at every speed — this
/// value's own coincidental exact match to this port's own `0.1` is
/// striking but not a confirmation, see below) and
/// `HANDBRAKE_LONG_FRICTION_FACTOR_CURVE` (`0.5` at a standstill, `0.9` at
/// and above 1 uu/s — effectively a near-constant, barely-reduced `0.9`
/// for any real driving speed), are applied to lateral and longitudinal
/// tire friction independently. Real Rocket League's handbrake drift
/// keeps a car's forward/backward grip almost intact (`x0.9`) while
/// cutting sideways grip to a tenth (`x0.1`) — this port's own single
/// isotropic `RigidBody.friction` scalar, read identically by both of
/// `solver::friction_directions`' own two tangent rows, has no way to
/// apply a different factor to each direction without threading a second,
/// direction-specific friction coefficient through every one of
/// `solver.rs`'s several row-limit-computation call sites
/// (`resolve_contacts`, `resolve_contacts_between`,
/// `resolve_static_manifolds`, `resolve_dynamic_manifolds`,
/// `resolve_manifolds`) — a substantially larger architecture change than
/// this finding alone justifies, the same category
/// `RB-PHYSICS-001-FR-063`/`FR-065` already established. `0.1`'s own
/// coincidental match to the real lateral-only factor is exactly that: a
/// coincidence, since this port's uniform `0.1` also (wrongly) crushes
/// longitudinal grip to a tenth, where real Rocket League keeps it near
/// `0.9` — this port's own handbrake understates real forward-momentum
/// retention during a drift. Not adopted as a fix; left for a future,
/// dedicated requirement.
pub(super) const HANDBRAKE_FRICTION_MULTIPLIER: f32 = 0.1;

/// Throttle force, curve-based steering yaw rate, and handbrake friction for
/// a grounded car. `base_friction` is the car's nominal friction; handbrake
/// scales it down while held and this call restores it otherwise.
pub(super) fn apply_ground_control(
    car: &mut RigidBody,
    input: &ControllerInput,
    forward: Vec3,
    base_friction: f32,
) {
    let forward_speed = car.linear_velocity.dot(&forward);
    let throttle = input.throttle.clamp(-1.0, 1.0);
    if throttle != 0.0 {
        let taper = drive_speed_taper(throttle.signum() * forward_speed);
        if taper > 0.0 {
            car.apply_central_force(
                forward * (throttle * THROTTLE_ACCELERATION * taper * car.mass()),
            );
        }
    }

    // RB-PHYSICS-001-FR-080: steering sets the car's yaw rate about its
    // own up axis directly (see `steer_yaw_rate`), replacing the old
    // speed-scaled torque. With no steer input the yaw rate is left alone,
    // so contacts and momentum still govern a car that isn't steering.
    let steer = input.steer.clamp(-1.0, 1.0);
    if steer != 0.0 {
        let up = up_axis(car);
        let target = steer_yaw_rate(forward_speed, steer, input.handbrake);
        let current = car.angular_velocity.dot(&up);
        car.angular_velocity += up * (target - current);
    }

    car.friction = if input.handbrake {
        base_friction * HANDBRAKE_FRICTION_MULTIPLIER
    } else {
        base_friction
    };
}
