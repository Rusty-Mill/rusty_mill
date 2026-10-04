//! Wheel rays and suspension (`RB-PHYSICS-001-FR-088`, `FR-090`; ADR-0017,
//! ADR-0018): RocketSim's `btVehicleRL` raycast, spring/damper and pushback,
//! and `Car::_UpdateWheels`' sticky force. Every number below is RocketSim's
//! Octane configuration (`CarConfig.cpp`, `RLConst.h`). Bullet's suspension
//! formulas carry over to uu unchanged: force and length scale together.

use super::ground::{FRONT_AXLE_X, REAR_AXLE_X};
use super::{forward_axis, up_axis};
use crate::body::RigidBody;
use crate::collision::RayHit;
use rb_domain::Vec3;

/// Octane wheels in the car's local frame (uu): RocketSim `CarConfig.cpp`
/// connection-point x/y offsets, and whether the wheel is a (steered)
/// front wheel.
pub(super) const WHEELS: [(f32, f32, bool); 4] = [
    (FRONT_AXLE_X, 25.90, true),
    (FRONT_AXLE_X, -25.90, true),
    (REAR_AXLE_X, 29.50, false),
    (REAR_AXLE_X, -29.50, false),
];

/// Height of the wheel ray starts above the car origin (uu),
/// `connectionPointOffset.z`.
pub(super) const WHEEL_RAY_START_Z: f32 = 20.755;

/// `RLConst::BTVehicle::MAX_SUSPENSION_TRAVEL` (uu).
const MAX_SUSPENSION_TRAVEL: f32 = 12.0;

/// `SUSPENSION_SUBTRACTION`, `0.05` Bullet units (uu). RocketSim takes it
/// off both the ray length and the pushback reach; this port keeps it on
/// the pushback only (`RB-PHYSICS-001-FR-092`).
const SUSPENSION_SUBTRACTION: f32 = 2.5;

const SUSPENSION_STIFFNESS: f32 = 500.0;
const WHEELS_DAMPING_COMPRESSION: f32 = 25.0;
const WHEELS_DAMPING_RELAXATION: f32 = 40.0;

/// `m_clippedInvContactDotSuspension` on a surface too steep to measure.
const STEEP_CONTACT_CLIP: f32 = 10.0;

/// The pushback's positional correction (`RB-PHYSICS-001-FR-122`,
/// ADR-0042). Bullet's `resolveSingleCollision` uses its default `m_erp`,
/// 0.2; the owner's bottomed-out landing (`test2.jsonl` 18.358 s) rebounds
/// as 0.1, and every recording's car error falls with it. A calibration,
/// not a sourced constant.
const PUSHBACK_ERP: f32 = 0.1;

/// Wheels whose ray must reach a surface for the car to count as on the
/// ground: `Car.cpp`'s `numWheelsInContact >= 3`.
const MIN_WHEELS_FOR_GROUND: usize = 3;

/// `RLConst::GRAVITY_Z` (uu/s^2): the sticky force scales the default
/// gravity, not the world's.
const STICKY_GRAVITY_Z: f32 = -650.0;

/// Base sticky force, as a share of gravity, while any wheel touches.
const STICKY_FORCE_BASE: f32 = 0.5;

/// Forward speed (uu/s) above which the sticky force grows on slopes:
/// `STOPPING_FORWARD_VEL`.
const STICKY_FULL_SPEED: f32 = 25.0;

/// One wheel's suspension geometry.
struct WheelSpec {
    /// `suspensionRestLength - MAX_SUSPENSION_TRAVEL` (uu).
    rest_length: f32,
    radius: f32,
    force_scale: f32,
}

const FRONT: WheelSpec = WheelSpec {
    rest_length: 38.755 - MAX_SUSPENSION_TRAVEL,
    radius: 12.5,
    force_scale: 36.0 - 0.25,
};

const BACK: WheelSpec = WheelSpec {
    rest_length: 37.055 - MAX_SUSPENSION_TRAVEL,
    radius: 15.0,
    force_scale: 54.0 + 0.25 + 0.015,
};

fn spec(front: bool) -> &'static WheelSpec {
    if front {
        &FRONT
    } else {
        &BACK
    }
}

/// One wheel ray's hit, from `btVehicleRL::rayCast`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelContact {
    /// Hit point, world space.
    pub point: Vec3,
    /// Surface normal at the hit.
    pub normal: Vec3,
    front: bool,
    /// Clamped to `rest_length ± MAX_SUSPENSION_TRAVEL`.
    suspension_length: f32,
    /// `1 / (normal · car up)`, or `None` when the surface is too steep
    /// (`approach <= 0.1`), where Bullet zeroes the suspension velocity and
    /// clips the spring by 10.
    inv_contact_dot: Option<f32>,
    /// `m_extraPushback`: a quarter of the impulse that would stop the
    /// wheel sinking past its rest reach.
    pushback: f32,
}

/// The four wheels' hits, in `WHEELS` order.
pub type WheelContacts = [Option<WheelContact>; 4];

/// Casts the four wheel rays down the car's own axis with `ray`
/// (`btVehicleRL::rayCast`): `ray(origin, direction, length)` is the first
/// static surface hit, so a ray hits walls, curves, meshes and the floor
/// alike (`RB-PHYSICS-001-FR-102`, FR-106; floor only before that). A ray
/// reaches `rest_length + MAX_SUSPENSION_TRAVEL + radius`, the fully
/// extended wheel (51.255 uu front, 52.055 back, so a level car touches up
/// to an origin height of 30.5 / 31.3 uu), and only hits a surface from
/// its front side.
///
/// RocketSim takes `SUSPENSION_SUBTRACTION` (2.5 uu) off that reach; the
/// owner's capture does not (`RB-PHYSICS-001-FR-092`): after the 4.142 s
/// jump the recorded wheels still grip in the step from origin height 29.7
/// and stop in the step from 32.3, where RocketSim's reach ends at 28.0.
pub fn cast_wheels(
    car: &RigidBody,
    ray: impl Fn(Vec3, Vec3, f32) -> Option<RayHit>,
    dt: f32,
) -> WheelContacts {
    WHEELS.map(|(x, y, front)| cast_wheel(car, &ray, Vec3::new(x, y, WHEEL_RAY_START_Z), front, dt))
}

fn cast_wheel(
    car: &RigidBody,
    ray: &impl Fn(Vec3, Vec3, f32) -> Option<RayHit>,
    start_local: Vec3,
    front: bool,
    dt: f32,
) -> Option<WheelContact> {
    let wheel = spec(front);
    let up = up_axis(car);
    let hard_point = car.position + car.orientation.rotate(&start_local);
    let reach = wheel.rest_length + MAX_SUSPENSION_TRAVEL + wheel.radius;
    let hit = ray(hard_point, -up, reach)?;
    let normal = hit.normal;
    let approach = normal.dot(&up);
    if approach <= 0.0 {
        return None;
    }
    let trace = hit.distance;
    let point = hard_point - up * trace;
    let suspension_length = (trace - wheel.radius).clamp(
        wheel.rest_length - MAX_SUSPENSION_TRAVEL,
        wheel.rest_length + MAX_SUSPENSION_TRAVEL,
    );
    let rel_pos = point - car.position;
    let inv_contact_dot = (approach > 0.1).then(|| 1.0 / approach);
    let pushback_reach = wheel.rest_length + wheel.radius - SUSPENSION_SUBTRACTION;
    let pushback = if trace < pushback_reach {
        stopping_impulse(car, rel_pos, normal, trace - pushback_reach, dt) / WHEELS.len() as f32
    } else {
        0.0
    };
    Some(WheelContact {
        point,
        normal,
        front,
        suspension_length,
        inv_contact_dot,
        pushback,
    })
}

/// Bullet's `resolveSingleCollision` against a static surface, without
/// applying it: the impulse along `normal` at `rel_pos` that cancels the
/// approach speed and `PUSHBACK_ERP` of the `distance` penetration per
/// tick, never pulling.
fn stopping_impulse(car: &RigidBody, rel_pos: Vec3, normal: Vec3, distance: f32, dt: f32) -> f32 {
    let normal_speed = normal.dot(&car.velocity_at_point(&rel_pos));
    let positional_error = PUSHBACK_ERP * -distance / dt;
    let arm = car
        .inv_inertia_world()
        .mul_vec3(&rel_pos.cross(&normal))
        .cross(&rel_pos);
    let denominator = car.inv_mass() + normal.dot(&arm);
    ((positional_error - normal_speed) / denominator).max(0.0)
}

/// Whether at least three wheels touch (`RB-PHYSICS-001-FR-088`).
pub fn is_on_ground(contacts: &WheelContacts) -> bool {
    contacts.iter().flatten().count() >= MIN_WHEELS_FOR_GROUND
}

/// Wheels touching nothing.
pub const NO_WHEEL_CONTACTS: WheelContacts = [None; 4];

/// Suspension impulses and the sticky force for one tick
/// (`RB-PHYSICS-001-FR-090`), from contacts cast at the start of the tick.
///
/// - **Suspension** (`btVehicleRL::updateSuspension`): per touching wheel,
///   `(rest - length) * SUSPENSION_STIFFNESS * inv_contact_dot` minus the
///   compression or relaxation damping times `suspension_velocity`, scaled
///   by the wheel's force scale and never pulling, applied as an impulse
///   along the normal at the contact point together with the pushback.
/// - **Sticky force** (`Car::_UpdateWheels`): half of default gravity
///   pushes the car along the average contact normal into the surface; with
///   throttle engaged or above `STICKY_FULL_SPEED` it grows by
///   `1 - |normal.z|` on slopes and walls. It acts on the step after any
///   wheel touched, along that step's normal (`previous_surface_up`), one
///   tick later than RocketSim (`RB-PHYSICS-001-FR-092`): the owner's
///   capture keeps it one tick past the wheels' last grip after a jump.
///
/// Returns this step's average contact normal, `None` with no wheel
/// touching, for the next step's sticky force.
pub fn apply_wheel_forces(
    car: &mut RigidBody,
    contacts: &WheelContacts,
    previous_surface_up: Option<Vec3>,
    throttle_engaged: bool,
    dt: f32,
) -> Option<Vec3> {
    for contact in contacts.iter().flatten() {
        apply_suspension(car, contact, dt);
    }
    if let Some(surface_up) = previous_surface_up {
        apply_sticky_force(car, surface_up, throttle_engaged);
    }
    average_normal(contacts)
}

/// The touching wheels' average contact normal, `None` with none touching.
pub(super) fn average_normal(contacts: &WheelContacts) -> Option<Vec3> {
    contacts
        .iter()
        .flatten()
        .fold(Vec3::ZERO, |sum, contact| sum + contact.normal)
        .normalize()
}

fn apply_sticky_force(car: &mut RigidBody, surface_up: Vec3, throttle_engaged: bool) {
    let forward_speed = car.linear_velocity.dot(&forward_axis(car));
    let full_stick = throttle_engaged || forward_speed.abs() > STICKY_FULL_SPEED;
    let slope_share = if full_stick {
        1.0 - surface_up.z.abs()
    } else {
        0.0
    };
    let scale = STICKY_FORCE_BASE + slope_share;
    car.apply_central_force(surface_up * (scale * STICKY_GRAVITY_Z * car.mass()));
}

/// The contact point's speed along the normal over `normal · car up`,
/// negative while compressing. Read from the car's current velocity, after
/// this step's drive impulses (`RB-PHYSICS-001-FR-091`): RocketSim reads it
/// before them, but the owner's capture shows no spring push on a jump's
/// press tick, the jump's upward speed relaxing the damper as it would if
/// the game handled the press first (as it does a dodge, FR-085).
fn suspension_velocity(car: &RigidBody, contact: &WheelContact) -> f32 {
    let Some(inv_contact_dot) = contact.inv_contact_dot else {
        return 0.0;
    };
    let rel_pos = contact.point - car.position;
    contact.normal.dot(&car.velocity_at_point(&rel_pos)) * inv_contact_dot
}

fn apply_suspension(car: &mut RigidBody, contact: &WheelContact, dt: f32) {
    let wheel = spec(contact.front);
    let spring = (wheel.rest_length - contact.suspension_length)
        * SUSPENSION_STIFFNESS
        * contact.inv_contact_dot.unwrap_or(STEEP_CONTACT_CLIP);
    let velocity = suspension_velocity(car, contact);
    let damping = if velocity < 0.0 {
        WHEELS_DAMPING_COMPRESSION
    } else {
        WHEELS_DAMPING_RELAXATION
    };
    let force = ((spring - damping * velocity) * wheel.force_scale).max(0.0);
    if force == 0.0 {
        return;
    }
    let impulse = contact.normal * (force * dt + contact.pushback);
    car.apply_impulse(impulse, contact.point - car.position);
}

/// All four wheels touching a surface along the car's own up axis, each
/// suspension exactly at rest and still: grounded, with no spring force.
/// For drive tests that place a car "on the ground" without a floor.
#[cfg(test)]
pub(crate) fn resting_contacts(car: &RigidBody) -> WheelContacts {
    let up = up_axis(car);
    WHEELS.map(|(x, y, front)| {
        let wheel = spec(front);
        let start = car.position + car.orientation.rotate(&Vec3::new(x, y, WHEEL_RAY_START_Z));
        Some(WheelContact {
            point: start - up * (wheel.rest_length + wheel.radius),
            normal: up,
            front,
            suspension_length: wheel.rest_length,
            inv_contact_dot: Some(1.0),
            pushback: 0.0,
        })
    })
}
