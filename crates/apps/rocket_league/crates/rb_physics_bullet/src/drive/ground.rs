//! Ground driving: throttle (with its speed taper), steering, handbrake, and
//! the ground jump's initial impulse. Every function here assumes the car
//! is on the ground; `super::apply_driven_forces` does the gating.

use super::{up_axis, MAX_CAR_SPEED, UNBOOSTED_MAX_CAR_SPEED};
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
    let speed = signed_speed_in_throttle_direction.max(0.0);
    let points = DRIVE_SPEED_TAPER_BREAKPOINTS;
    if speed <= points[0].0 {
        return points[0].1;
    }
    for window in points.windows(2) {
        let (x0, y0) = window[0];
        let (x1, y1) = window[1];
        if speed <= x1 {
            return y0 + (y1 - y0) * (speed - x0) / (x1 - x0);
        }
    }
    points[points.len() - 1].1
}

/// Uncalibrated placeholder steering torque magnitude (about the car's
/// local up axis, at full `steer` input and at/above `MAX_CAR_SPEED`) —
/// chosen only so a full-lock turn is visibly responsive for this car's
/// mass/inertia in tests, not derived from any measured or documented
/// Rocket League value.
///
/// `RB-PHYSICS-001-FR-065` fetched RocketSim's real `Car.cpp` (`_UpdateWheels`,
/// matching `RB-PHYSICS-001-FR-058`/`FR-059`/`FR-064`'s own
/// real-implementation-file method) and found real Rocket League's
/// steering isn't a direct yaw-torque model at all: a wheel's *steer
/// angle* (not a torque) is set from a confirmed real
/// `STEER_ANGLE_FROM_SPEED_CURVE` (`RLConst.h`, radians), and that angled
/// wheel's lateral tire friction — computed per-wheel by `btVehicleRL`, a
/// custom extension of Bullet's own raycast vehicle system
/// (`btDefaultVehicleRaycaster`), through a further confirmed
/// `LAT_FRICTION_CURVE` slip-friction curve — is what actually turns the
/// car. This port has no wheels, raycasting, or tire-slip model at all
/// (the car is one rigid box), so this real mechanism can't be ported
/// without a substantially larger architecture change, the same category
/// `RB-PHYSICS-001-FR-063` already established for per-contact-pair-type
/// restitution/friction.
///
/// One finding is still directly actionable even without that larger
/// change: the confirmed real curve's own *shape* is the opposite of this
/// port's own `speed_factor` below. Real Rocket League's maximum steering
/// angle is highest at a standstill (`0.53356` rad ≈ 30.6° at 0 uu/s) and
/// decreases sharply as speed rises (down to `0.03454` rad ≈ 2° at 3000
/// uu/s) — a car can turn tightest from a stop, only gently at speed. This
/// port's own `speed_factor` does the opposite: zero torque at a
/// standstill, scaling *up* to full `STEER_TORQUE` at `MAX_CAR_SPEED`.
/// Not adopted as a fix: unlike `RB-PHYSICS-001-FR-058`'s throttle taper
/// or `FR-059`'s dodge scale (direct multipliers on a force/impulse this
/// port already applies the same way real Rocket League does), the real
/// curve maps speed to a *wheel angle*, which real Rocket League then
/// feeds through nonlinear tire-slip friction (dependent on wheelbase
/// geometry and friction curves this port doesn't model at all) to
/// produce the actual turning force — there's no principled way to carry
/// even the curve's normalized shape onto this port's own direct-torque
/// model. Reversing `speed_factor`'s direction without that transfer
/// function would substitute one unconfirmed guess for another, not adopt
/// a confirmed real value — the same reasoning that kept
/// `RB-PHYSICS-001-FR-057`'s `AIR_CONTROL_TORQUE` and `FR-059`'s
/// `DODGE_SPEED` base magnitude as placeholders despite a real reference
/// existing for each.
pub(super) const STEER_TORQUE: f32 = 1_500_000.0;

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

/// Throttle force, speed-scaled steering torque, and handbrake friction for
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

    let steer = input.steer.clamp(-1.0, 1.0);
    if steer != 0.0 {
        // A stationary car can't carve a turn — scale the available
        // torque by how fast it's already going, up to MAX_CAR_SPEED.
        // RB-PHYSICS-001-FR-065: real Rocket League's own confirmed
        // steering curve has this backwards — maximum turning ability
        // is highest at a standstill and decreases with speed — but
        // that curve doesn't transfer onto this port's own
        // direct-torque model; see STEER_TORQUE's own doc comment.
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
