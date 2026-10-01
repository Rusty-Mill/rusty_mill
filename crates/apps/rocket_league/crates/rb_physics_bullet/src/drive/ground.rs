//! Ground driving: throttle (with its speed taper), steering, tire grip
//! (with the handbrake), and the ground jump's initial impulse. Every
//! function here assumes the car is on the ground;
//! `super::apply_driven_forces` does the gating.

use super::{forward_axis, right_axis, up_axis, UNBOOSTED_MAX_CAR_SPEED};
use crate::body::{RigidBody, Shape, StaticPlane};
use rb_domain::{ControllerInput, Vec3};

/// Peak throttle acceleration (uu/s^2), at a standing start. Long an
/// uncalibrated placeholder, `RB-PHYSICS-001-FR-081` confirmed it against
/// RocketSim's `btVehicleRL::calcFrictionImpulses`: each wheel's engine
/// force (`THROTTLE_TORQUE_AMOUNT * UU_TO_BT = 1440`) becomes a `1440 *
/// dt` impulse on the `180` mass car, `8` m/s^2 per wheel, `1600` uu/s^2
/// over four wheels. Since
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

/// Octane axle positions along the car's local forward axis (uu):
/// RocketSim `CarConfig.cpp`, `CAR_CONFIG_OCTANE` wheel connection points.
pub(super) const FRONT_AXLE_X: f32 = 51.25;
pub(super) const REAR_AXLE_X: f32 = -33.75;

/// Front-wheel steer angle (rad) for `steer` at `forward_speed`
/// (`RB-PHYSICS-001-FR-086`): RocketSim's `STEER_ANGLE_FROM_SPEED_CURVE`,
/// blended toward `POWERSLIDE_STEER_ANGLE_FROM_SPEED_CURVE` by
/// `handbrake_amount` (`0..=1`) as RocketSim's `Car::_UpdateWheels` does.
/// Positive turns the front wheels toward the car's +right axis.
pub(super) fn steer_angle(forward_speed: f32, steer: f32, handbrake_amount: f32) -> f32 {
    let speed = forward_speed.abs();
    let normal = curve(&STEER_ANGLE_FROM_SPEED_CURVE, speed);
    let powerslide = curve(&POWERSLIDE_STEER_ANGLE_FROM_SPEED_CURVE, speed);
    steer.clamp(-1.0, 1.0) * (normal + (powerslide - normal) * handbrake_amount)
}

/// Sideways tire grip by slip ratio: RocketSim's `LAT_FRICTION_CURVE`
/// (`RLConst.h`). Full grip while rolling straight, a fifth of it when
/// sliding fully sideways.
pub(super) const LAT_FRICTION_CURVE: [(f32, f32); 2] = [(0.0, 1.0), (1.0, 0.2)];

/// Sideways grip factor at full handbrake: RocketSim's
/// `HANDBRAKE_LAT_FRICTION_FACTOR_CURVE`, a constant `0.1`.
pub(super) const HANDBRAKE_LAT_FRICTION_FACTOR: f32 = 0.1;

/// Forward/backward grip factor by slip ratio at full handbrake:
/// RocketSim's `HANDBRAKE_LONG_FRICTION_FACTOR_CURVE`.
pub(super) const HANDBRAKE_LONG_FRICTION_FACTOR_CURVE: [(f32, f32); 2] = [(0.0, 0.5), (1.0, 0.9)];

/// RocketSim's `POWERSLIDE_RISE_RATE` and `POWERSLIDE_FALL_RATE` (1/s): how
/// fast `DriveState::handbrake_amount` ramps toward `1` while the handbrake
/// is held and back toward `0` once released.
pub(super) const POWERSLIDE_RISE_RATE: f32 = 5.0;
pub(super) const POWERSLIDE_FALL_RATE: f32 = 2.0;

/// Ramps `handbrake_amount` one tick toward held (`1`) or released (`0`).
pub(super) fn ramp_handbrake(handbrake_amount: f32, held: bool, dt: f32) -> f32 {
    let rate = if held {
        POWERSLIDE_RISE_RATE
    } else {
        -POWERSLIDE_FALL_RATE
    };
    (handbrake_amount + rate * dt).clamp(0.0, 1.0)
}

/// Lateral speed (uu/s) at or below which slip reads as zero — RocketSim's
/// `_UpdateWheels` only computes a slip ratio above it.
pub(super) const SLIP_LATERAL_SPEED_THRESHOLD: f32 = 5.0;

/// Bullet's `resolveSingleBilateral` contact damping: each tick a wheel's
/// side impulse cancels this share of its contact's lateral velocity.
const SIDE_IMPULSE_DAMPING: f32 = 0.2;

/// `btVehicleRL::calcFrictionImpulses`' `frictionScale`: RocketSim's
/// `CAR_MASS_BT / 3`, multiplied into every wheel's friction impulse.
const FRICTION_SCALE: f32 = 180.0 / 3.0;

/// Octane wheels in the car's local frame (uu): RocketSim `CarConfig.cpp`
/// x/y offsets, and whether the wheel steers. Contacts sit at the floor
/// under this port's box.
const WHEELS: [(f32, f32, bool); 4] = [
    (FRONT_AXLE_X, 25.90, true),
    (FRONT_AXLE_X, -25.90, true),
    (REAR_AXLE_X, 29.50, false),
    (REAR_AXLE_X, -29.50, false),
];

/// Height of the Octane's wheel ray starts above the car origin (uu),
/// RocketSim `CarConfig.cpp`'s `connectionPointOffset.z`.
const WHEEL_RAY_START_Z: f32 = 20.755;

/// Wheel ray reach (uu): RocketSim's `btVehicleRL::rayCast` length,
/// `(restLength - MAX_SUSPENSION_TRAVEL) + MAX_SUSPENSION_TRAVEL + radius -
/// SUSPENSION_SUBTRACTION`, i.e. `restLength + radius - 2.5`. Front: rest
/// `38.755`, radius `12.5`; back: rest `37.055`, radius `15`.
const FRONT_WHEEL_RAY_LENGTH: f32 = 38.755 + 12.5 - 2.5;
const BACK_WHEEL_RAY_LENGTH: f32 = 37.055 + 15.0 - 2.5;

/// Wheels whose ray must reach a surface for the car to count as on the
/// ground: RocketSim `Car.cpp`'s `numWheelsInContact >= 3`.
const MIN_WHEELS_FOR_GROUND: usize = 3;

/// Whether `car` is on the ground (`RB-PHYSICS-001-FR-088`): at least three
/// of its four wheel rays, cast straight down the car's own axis from
/// RocketSim's Octane connection points, reach `plane` from its front side.
/// Unlike box-corner contact, this holds while the box hovers on its
/// suspension or bounces a few uu off the floor, as the real wheels do.
pub fn wheels_on_ground(car: &RigidBody, plane: &StaticPlane) -> bool {
    let down = car.orientation.rotate(&Vec3::new(0.0, 0.0, -1.0));
    let approach = -plane.normal.dot(&down);
    if approach <= 0.0 {
        return false;
    }
    let touching = WHEELS
        .iter()
        .filter(|&&(x, y, front)| {
            let start = car.position + car.orientation.rotate(&Vec3::new(x, y, WHEEL_RAY_START_Z));
            let height = plane.signed_distance(&start);
            let reach = if front {
                FRONT_WHEEL_RAY_LENGTH
            } else {
                BACK_WHEEL_RAY_LENGTH
            };
            height >= 0.0 && height <= reach * approach
        })
        .count();
    touching >= MIN_WHEELS_FOR_GROUND
}

/// Full brake deceleration (uu/s^2). RocketSim's per-wheel brake
/// (`BRAKE_TORQUE_AMOUNT * UU_TO_BT = 52.5`) times four wheels over
/// `frictionScale`'s `CAR_MASS_BT / 3` gives `70` m/s^2, i.e. `3500` uu/s^2.
pub(super) const BRAKE_DECELERATION: f32 = 3500.0;

/// Share of `BRAKE_DECELERATION` applied while coasting: RocketSim's
/// `COASTING_BRAKE_FACTOR` (`525` uu/s^2).
pub(super) const COASTING_BRAKE_FACTOR: f32 = 0.15;

/// Forward speed (uu/s) below which a coasting car brakes fully, and at or
/// below which opposing throttle drives rather than brakes: RocketSim's
/// `STOPPING_FORWARD_VEL`.
pub(super) const STOPPING_FORWARD_SPEED: f32 = 25.0;

/// Throttle magnitude below which RocketSim treats the car as coasting:
/// `THROTTLE_DEADZONE`.
pub(super) const THROTTLE_DEADZONE: f32 = 0.001;

/// Slip ratio RocketSim's tire curves are looked up by: the lateral share
/// of the contact speed, zero at or below `SLIP_LATERAL_SPEED_THRESHOLD`.
pub(super) fn slip_ratio(forward_speed: f32, lateral_speed: f32) -> f32 {
    let lateral = lateral_speed.abs();
    if lateral <= SLIP_LATERAL_SPEED_THRESHOLD {
        return 0.0;
    }
    lateral / (forward_speed.abs() + lateral)
}

/// Engine throttle and brake (`0..=1`) for one tick, per RocketSim's
/// `Car::_UpdateWheels` pedal logic. Holding the handbrake passes throttle
/// through with no brake. Otherwise, throttle against the car's motion
/// above `STOPPING_FORWARD_SPEED` brakes fully and cuts the engine, and
/// no throttle coasts: `COASTING_BRAKE_FACTOR`, or a full brake below
/// `STOPPING_FORWARD_SPEED`.
pub(super) fn pedals(throttle: f32, forward_speed: f32, handbrake: bool) -> (f32, f32) {
    if handbrake {
        return (throttle, 0.0);
    }
    if throttle.abs() < THROTTLE_DEADZONE {
        let brake = if forward_speed.abs() < STOPPING_FORWARD_SPEED {
            1.0
        } else {
            COASTING_BRAKE_FACTOR
        };
        return (0.0, brake);
    }
    if forward_speed.abs() > STOPPING_FORWARD_SPEED && throttle.signum() != forward_speed.signum() {
        return (0.0, 1.0);
    }
    (throttle, 0.0)
}

/// Lateral and longitudinal tire grip factors at `slip`, with the
/// handbrake's reductions blended in by `handbrake_amount` (`0..=1`).
pub(super) fn tire_grip(slip: f32, handbrake_amount: f32) -> (f32, f32) {
    let lateral = curve(&LAT_FRICTION_CURVE, slip)
        * (1.0 + (HANDBRAKE_LAT_FRICTION_FACTOR - 1.0) * handbrake_amount);
    let longitudinal =
        1.0 + (curve(&HANDBRAKE_LONG_FRICTION_FACTOR_CURVE, slip) - 1.0) * handbrake_amount;
    (lateral, longitudinal)
}

/// Each wheel's impulse for one tick, RocketSim's
/// `btVehicleRL::calcFrictionImpulses`. The side impulse
/// (`RB-PHYSICS-001-FR-086`) acts along the
/// wheel's axle (the front wheels turned by `steer_angle`), cancelling
/// `SIDE_IMPULSE_DAMPING` of the contact point's sideways velocity through
/// the car's effective mass there (`jacDiagABInv`), scaled by the tire's
/// lateral grip and `FRICTION_SCALE * dt`. The slip ratio each wheel's grip
/// is looked up by uses that wheel's own contact velocity, so a yawing car's
/// rear wheels slip and resist the yaw. Each wheel also carries a quarter of
/// `engine_acceleration` along its own heading (`RB-PHYSICS-001-FR-089`), so
/// the steered front wheels' push turns the car as well. Returns
/// `(impulse, point)` pairs,
/// all computed from the same pre-impulse state as RocketSim does, with the
/// point flattened onto the car's floor plane (RocketSim's
/// `applyFrictionImpulses`), so the impulses yaw the car without rolling it.
fn wheel_impulses(
    car: &RigidBody,
    steer: f32,
    handbrake_amount: f32,
    engine_acceleration: f32,
    dt: f32,
) -> Vec<(Vec3, Vec3)> {
    let Shape::Box { half_extents } = car.shape else {
        return Vec::new();
    };
    let forward = forward_axis(car);
    let right = right_axis(car);
    let up = up_axis(car);
    let inv_inertia = car.inv_inertia_world();
    let front_angle = steer_angle(car.linear_velocity.dot(&forward), steer, handbrake_amount);
    WHEELS
        .iter()
        .map(|&(x, y, steers)| {
            let angle = if steers { front_angle } else { 0.0 };
            let axle = right * angle.cos() - forward * angle.sin();
            let rolling = forward * angle.cos() + right * angle.sin();
            let flat = forward * x + right * y;
            let contact = flat - up * half_extents.z;
            let velocity = car.velocity_at_point(&contact);
            let lateral = velocity.dot(&axle);
            let (lateral_grip, _) = tire_grip(
                slip_ratio(velocity.dot(&rolling), lateral),
                handbrake_amount,
            );
            let arm = contact.cross(&axle);
            let inv_effective_mass = car.inv_mass() + arm.dot(&inv_inertia.mul_vec3(&arm));
            let impulse = -SIDE_IMPULSE_DAMPING * lateral / inv_effective_mass
                * lateral_grip
                * FRICTION_SCALE
                * dt;
            let drive = engine_acceleration * car.mass() / WHEELS.len() as f32 * dt;
            (axle * impulse + rolling * drive, flat)
        })
        .collect()
}

/// Throttle, steering and tire grip for a grounded car
/// (`RB-PHYSICS-001-FR-081`, `FR-086`; ADR-0012, ADR-0016). `throttle` is
/// the effective throttle (boosting forces it to `1`, as in RocketSim).
///
/// The car's box has no floor friction (see `PhysicsWorld`'s static
/// manifolds); its tires grip instead, as in RocketSim's
/// `Car::_UpdateWheels` and `btVehicleRL::calcFrictionImpulses`:
///
/// - **Engine**: the engine force, scaled by the longitudinal grip factor,
///   split over the four wheels along each wheel's heading
///   (`wheel_impulses`), so throttle through steered front wheels turns
///   the car too.
/// - **Brake**: `pedals`' brake at `BRAKE_DECELERATION`, never reversing
///   the car, applied at the centre of mass.
/// - **Sideways and steering**: each wheel's side impulse
///   (`wheel_impulses`) at its contact point. The steered front wheels'
///   impulses yaw the car; the rear wheels' resist it, so the turn rate
///   builds up and dies away instead of being set.
pub(super) fn apply_ground_control(
    car: &mut RigidBody,
    input: &ControllerInput,
    throttle: f32,
    forward: Vec3,
    handbrake_amount: f32,
    dt: f32,
) {
    let right = right_axis(car);
    let forward_speed = car.linear_velocity.dot(&forward);
    let lateral_speed = car.linear_velocity.dot(&right);
    let (_, longitudinal_grip) =
        tire_grip(slip_ratio(forward_speed, lateral_speed), handbrake_amount);
    let (engine, brake) = pedals(throttle, forward_speed, input.handbrake);

    let taper = drive_speed_taper(engine.signum() * forward_speed);
    let acceleration = engine * THROTTLE_ACCELERATION * taper * longitudinal_grip;

    // RB-PHYSICS-001-FR-086/FR-089: steering, sideways grip and the engine
    // are per-wheel impulses; the steered front wheels' impulses turn the car.
    for (impulse, point) in wheel_impulses(car, input.steer, handbrake_amount, acceleration, dt) {
        car.apply_impulse(impulse, point);
    }

    let speed_drop = (BRAKE_DECELERATION * brake * dt).min(forward_speed.abs());
    car.linear_velocity -= forward * (forward_speed.signum() * speed_drop);
}
