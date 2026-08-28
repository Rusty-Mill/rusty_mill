//! Sequential-impulse contact solver, ported from
//! `bullet3/src/BulletDynamics/ConstraintSolver/btSequentialImpulseConstraintSolver.cpp`
//! (zlib license — see `THIRD_PARTY_NOTICES.md`). Two paths:
//! - `resolve_contacts`: one dynamic body (sphere or box, via `RigidBody`'s
//!   general 3x3 inverse inertia tensor — see `body.rs`) against one static
//!   plane's contact manifold (1 to 4 points depending on shape/orientation).
//! - `resolve_contact_between`: two dynamic bodies against each other's
//!   single contact point (the ball-vs-car case — `sphere_vs_box` never
//!   produces more than one). This is the generic two-body path Bullet's
//!   real solver always runs (every constraint row carries both bodies'
//!   mass/inertia contributions); `resolve_contacts` only got away with a
//!   one-body-only version because a static plane's side of that math is
//!   always zero.
//!
//! Deliberate, documented deviations from Bullet's actual solver (tracked
//! as open follow-up work in `RB-PHYSICS-001`, not silently assumed away):
//! - **No split impulse.** Bullet's default (`m_splitImpulse = true`) runs
//!   a second pseudo-velocity pass so penetration correction doesn't add
//!   energy to the real velocity used for restitution. This port always
//!   takes Bullet's non-split branch (`m_rhs = penetrationImpulse +
//!   velocityImpulse`), which is simpler and still stable at Rocket
//!   League's real contact depths, but is a real behavioral difference at
//!   high penetration.
//! - **No SIMD.** Scalar translation of the SSE2/SSE4/FMA3 code paths —
//!   this is a from-scratch Rust port, not a binding (see ADR-0004), and
//!   correctness came before micro-optimization for v0.
//! - **No warm-starting or sleeping.** Real Bullet carries `m_appliedImpulse`
//!   across frames (warm starting) and puts settled bodies to sleep. This
//!   port re-derives every contact's impulses from zero each frame, which
//!   means a *bouncy* resting contact (restitution > 0) never actually
//!   settles — each frame's fresh gravity-induced velocity is solved as a
//!   new "impact" and restitution bounces it back up, indefinitely. An
//!   inelastic resting contact (restitution 0) settles fine (see
//!   `world::tests::resting_ball_stays_at_rest`); the bouncy case is a real
//!   limitation, tracked as follow-up work in `RB-PHYSICS-001`, not hidden
//!   behind a loosened test.
//! - **Restitution/friction combine mode**: average of the two surfaces'
//!   coefficients (`(a + b) * 0.5`), matching
//!   `btManifoldResult::calculateCombinedRestitution`'s structure. Bullet's
//!   *default* combine mode is actually `btMax` for both — this port uses
//!   average instead, pending calibration against real recorded ball/
//!   ground behavior (`RB-VERIFY-001`/`RB-VERIFY-002` data). Tracked as an
//!   open item in `RB-PHYSICS-001`, not asserted as settled.

use crate::body::{RigidBody, StaticPlane};
use crate::collision::Contact;
use rb_domain::Vec3;

/// `btContactSolverInfo`'s defaults this port fixes rather than exposes as
/// config yet (bullet3/src/BulletDynamics/ConstraintSolver/btContactSolverInfo.h).
const ERP2: f32 = 0.2;
const GLOBAL_CFM: f32 = 0.0;
const LINEAR_SLOP: f32 = 0.0;
const RESTITUTION_VELOCITY_THRESHOLD: f32 = 0.2;
const RELAXATION: f32 = 1.0;
const SOLVER_ITERATIONS: u32 = 10;
const UPPER_LIMIT: f32 = 1e10;

fn combine_restitution(a: f32, b: f32) -> f32 {
    (a + b) * 0.5
}

fn combine_friction(a: f32, b: f32) -> f32 {
    (a + b) * 0.5
}

/// Port of `btSequentialImpulseConstraintSolver::restitutionCurve`.
fn restitution_curve(rel_vel: f32, restitution: f32, velocity_threshold: f32) -> f32 {
    if rel_vel.abs() < velocity_threshold {
        0.0
    } else {
        (restitution * -rel_vel).max(0.0)
    }
}

/// Port of `btPlaneSpace1`: builds two vectors orthogonal to `n` (and to
/// each other), used as the two friction directions in the tangent plane.
fn plane_space(n: &Vec3) -> (Vec3, Vec3) {
    if n.z.abs() > std::f32::consts::FRAC_1_SQRT_2 {
        let a = n.y * n.y + n.z * n.z;
        let k = 1.0 / a.sqrt();
        let p = Vec3::new(0.0, -n.z * k, n.y * k);
        let q = Vec3::new(a * k, -n.x * p.z, n.x * p.y);
        (p, q)
    } else {
        let a = n.x * n.x + n.y * n.y;
        let k = 1.0 / a.sqrt();
        let p = Vec3::new(-n.y * k, n.x * k, 0.0);
        let q = Vec3::new(-n.z * p.y, n.z * p.x, a * k);
        (p, q)
    }
}

/// One constraint row: a contact normal or a friction direction, solved
/// with the projected-Gauss-Seidel iteration below. Mirrors the fields of
/// `btSolverConstraint` this port actually needs.
struct ConstraintRow {
    /// World-space direction the impulse acts along (contact normal, or a
    /// friction tangent).
    direction: Vec3,
    /// `direction × rel_pos`, i.e. `m_relpos1CrossNormal` — dotted with
    /// angular *velocity* to get a point's velocity contribution along
    /// `direction`.
    torque_axis: Vec3,
    /// `inv_inertia * torque_axis`, i.e. `m_angularComponentA` — how much
    /// angular velocity a unit impulse along `direction` produces.
    angular_component: Vec3,
    jac_diag_ab_inv: f32,
    rhs: f32,
    cfm: f32,
    lower_limit: f32,
    upper_limit: f32,
    applied_impulse: f32,
}

fn effective_mass_denom(body: &RigidBody, rel_pos: &Vec3, direction: &Vec3) -> (Vec3, Vec3, f32) {
    let torque_axis = rel_pos.cross(direction);
    let angular_component = body.inv_inertia_world().mul_vec3(&torque_axis);
    let denom = body.inv_mass() + direction.dot(&angular_component.cross(rel_pos));
    (torque_axis, angular_component, denom)
}

/// Accumulates velocity change across solver iterations before it's
/// applied to the body once — matches Bullet's `btSolverBody` separating
/// "delta" velocity from the pre-solve velocity used to compute `rhs`, so
/// resolving the normal row before the friction rows within one iteration
/// doesn't corrupt the `rhs` baseline computed at setup time.
struct DeltaVelocity {
    linear: Vec3,
    angular: Vec3,
}

impl DeltaVelocity {
    fn zero() -> DeltaVelocity {
        DeltaVelocity {
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
        }
    }
}

/// Sets up the normal + two friction constraint rows for one contact,
/// porting `setupContactConstraint` + `setFrictionConstraintImpulse`
/// against a static body B (the plane), which is why every `rb1`-branch in
/// the original is simply the zero case here.
fn setup_rows(body: &RigidBody, contact: &Contact, dt: f32) -> [ConstraintRow; 3] {
    let rel_pos = contact.point - body.position;
    let inv_dt = 1.0 / dt;

    let (normal_torque_axis, normal_angular_component, denom) =
        effective_mass_denom(body, &rel_pos, &contact.normal);
    let jac_diag_ab_inv = RELAXATION / (denom + GLOBAL_CFM);

    let rel_vel = contact.normal.dot(&body.velocity_at_point(&rel_pos));
    let restitution = restitution_curve(rel_vel, body.restitution, RESTITUTION_VELOCITY_THRESHOLD);

    let gap_with_slop = -contact.penetration_depth + LINEAR_SLOP;
    let (positional_error, velocity_error) = if gap_with_slop > 0.0 {
        (0.0, restitution - rel_vel - gap_with_slop * inv_dt)
    } else {
        (-gap_with_slop * ERP2 * inv_dt, restitution - rel_vel)
    };

    let normal_row = ConstraintRow {
        direction: contact.normal,
        torque_axis: normal_torque_axis,
        angular_component: normal_angular_component,
        jac_diag_ab_inv,
        rhs: (positional_error + velocity_error) * jac_diag_ab_inv,
        cfm: GLOBAL_CFM * jac_diag_ab_inv,
        lower_limit: 0.0,
        upper_limit: UPPER_LIMIT,
        applied_impulse: 0.0,
    };

    let (t1, t2) = plane_space(&contact.normal);
    // Port of `setupFrictionConstraint`: target zero relative velocity
    // along the tangent direction (Bullet's `desiredVelocity` parameter is
    // 0 in the default no-conveyor-belt case) — a friction row needs a
    // nonzero `rhs` to ever apply an impulse; it isn't "no correction".
    let friction_row = |dir: Vec3| -> ConstraintRow {
        let (torque_axis, angular_component, denom) = effective_mass_denom(body, &rel_pos, &dir);
        let jac_diag_ab_inv = RELAXATION / (denom + GLOBAL_CFM);
        let rel_vel = dir.dot(&body.velocity_at_point(&rel_pos));
        ConstraintRow {
            direction: dir,
            torque_axis,
            angular_component,
            jac_diag_ab_inv,
            rhs: -rel_vel * jac_diag_ab_inv,
            cfm: 0.0,
            lower_limit: 0.0, // recomputed from the normal row's impulse each iteration
            upper_limit: 0.0,
            applied_impulse: 0.0,
        }
    };

    [normal_row, friction_row(t1), friction_row(t2)]
}

/// Port of `resolveSingleConstraintRowGeneric`/`...LowerLimit`: one
/// projected-Gauss-Seidel update of a single constraint row against the
/// body's currently-accumulated delta velocity.
fn resolve_row(row: &mut ConstraintRow, inv_mass: f32, delta: &mut DeltaVelocity) {
    let delta_vel_dot_n = row.direction.dot(&delta.linear) + row.torque_axis.dot(&delta.angular);

    let mut delta_impulse = row.rhs - row.applied_impulse * row.cfm;
    delta_impulse -= delta_vel_dot_n * row.jac_diag_ab_inv;

    let sum = row.applied_impulse + delta_impulse;
    if sum < row.lower_limit {
        delta_impulse = row.lower_limit - row.applied_impulse;
        row.applied_impulse = row.lower_limit;
    } else if sum > row.upper_limit {
        delta_impulse = row.upper_limit - row.applied_impulse;
        row.applied_impulse = row.upper_limit;
    } else {
        row.applied_impulse = sum;
    }

    delta.linear += row.direction * (inv_mass * delta_impulse);
    delta.angular += row.angular_component * delta_impulse;
}

/// Resolves an entire contact manifold (1 to 4 points — a box resting flat
/// generates up to 4, matching `RB-PHYSICS-001-FR-004`'s multi-contact
/// requirement; a sphere always generates exactly 1) against one static
/// plane. Runs `SOLVER_ITERATIONS` passes of the sequential impulse
/// solver, each pass resolving every contact's normal row and then every
/// contact's friction rows (limits re-derived from that same contact's
/// current normal impulse — matching Bullet's per-iteration friction
/// reclamping), sharing one accumulated `DeltaVelocity` across the whole
/// manifold so an earlier contact's resolution already influences a later
/// contact's `rhs` baseline within the same iteration — then applies the
/// accumulated velocity change to `body` once.
pub fn resolve_contacts(body: &mut RigidBody, plane: &StaticPlane, contacts: &[Contact], dt: f32) {
    if contacts.is_empty() {
        return;
    }

    let combined_restitution = combine_restitution(body.restitution, plane.restitution);
    let combined_friction = combine_friction(body.friction, plane.friction);

    let mut effective_body = *body;
    effective_body.restitution = combined_restitution;

    let mut manifold: Vec<[ConstraintRow; 3]> = contacts
        .iter()
        .map(|c| setup_rows(&effective_body, c, dt))
        .collect();
    let mut delta = DeltaVelocity::zero();
    let inv_mass = body.inv_mass();

    for _ in 0..SOLVER_ITERATIONS {
        for rows in &mut manifold {
            resolve_row(&mut rows[0], inv_mass, &mut delta);

            let friction_limit = combined_friction * rows[0].applied_impulse;
            rows[1].lower_limit = -friction_limit;
            rows[1].upper_limit = friction_limit;
            rows[2].lower_limit = -friction_limit;
            rows[2].upper_limit = friction_limit;

            resolve_row(&mut rows[1], inv_mass, &mut delta);
            resolve_row(&mut rows[2], inv_mass, &mut delta);
        }
    }

    body.linear_velocity += delta.linear;
    body.angular_velocity += delta.angular;
}

/// Like `ConstraintRow`, but carrying both bodies' torque axis/angular
/// component (`_a`/`_b`) instead of assuming one side is static.
struct TwoBodyRow {
    direction: Vec3,
    torque_axis_a: Vec3,
    angular_component_a: Vec3,
    torque_axis_b: Vec3,
    angular_component_b: Vec3,
    jac_diag_ab_inv: f32,
    rhs: f32,
    cfm: f32,
    lower_limit: f32,
    upper_limit: f32,
    applied_impulse: f32,
}

/// Two-body version of `effective_mass_denom`: each body contributes its
/// own `inv_mass + direction · (angular_component × rel_pos)` term to the
/// shared denominator, matching Bullet's generic (both-sides-dynamic)
/// constraint setup.
#[allow(clippy::type_complexity)]
fn effective_mass_denom_two_body(
    a: &RigidBody,
    b: &RigidBody,
    rel_pos_a: &Vec3,
    rel_pos_b: &Vec3,
    direction: &Vec3,
) -> (Vec3, Vec3, Vec3, Vec3, f32) {
    let torque_axis_a = rel_pos_a.cross(direction);
    let angular_component_a = a.inv_inertia_world().mul_vec3(&torque_axis_a);
    let torque_axis_b = rel_pos_b.cross(direction);
    let angular_component_b = b.inv_inertia_world().mul_vec3(&torque_axis_b);
    let denom = a.inv_mass()
        + b.inv_mass()
        + direction.dot(&angular_component_a.cross(rel_pos_a))
        + direction.dot(&angular_component_b.cross(rel_pos_b));
    (
        torque_axis_a,
        angular_component_a,
        torque_axis_b,
        angular_component_b,
        denom,
    )
}

/// Per-body accumulated velocity change for the two-body solver — the
/// two-body analog of `DeltaVelocity`.
struct TwoBodyDelta {
    linear_a: Vec3,
    angular_a: Vec3,
    linear_b: Vec3,
    angular_b: Vec3,
}

impl TwoBodyDelta {
    fn zero() -> TwoBodyDelta {
        TwoBodyDelta {
            linear_a: Vec3::ZERO,
            angular_a: Vec3::ZERO,
            linear_b: Vec3::ZERO,
            angular_b: Vec3::ZERO,
        }
    }
}

/// Two-body version of `setup_rows`. `combined_restitution` is passed in
/// explicitly (rather than stashed on a copied body, as `resolve_contacts`
/// does) since here there's no single "the body" to stash it on.
fn setup_two_body_rows(
    a: &RigidBody,
    b: &RigidBody,
    contact: &Contact,
    combined_restitution: f32,
    dt: f32,
) -> [TwoBodyRow; 3] {
    let rel_pos_a = contact.point - a.position;
    let rel_pos_b = contact.point - b.position;
    let inv_dt = 1.0 / dt;

    let relative_velocity_along = |dir: &Vec3| -> f32 {
        dir.dot(&(a.velocity_at_point(&rel_pos_a) - b.velocity_at_point(&rel_pos_b)))
    };

    let (
        normal_torque_axis_a,
        normal_angular_component_a,
        normal_torque_axis_b,
        normal_angular_component_b,
        denom,
    ) = effective_mass_denom_two_body(a, b, &rel_pos_a, &rel_pos_b, &contact.normal);
    let jac_diag_ab_inv = RELAXATION / (denom + GLOBAL_CFM);

    let rel_vel = relative_velocity_along(&contact.normal);
    let restitution = restitution_curve(
        rel_vel,
        combined_restitution,
        RESTITUTION_VELOCITY_THRESHOLD,
    );

    let gap_with_slop = -contact.penetration_depth + LINEAR_SLOP;
    let (positional_error, velocity_error) = if gap_with_slop > 0.0 {
        (0.0, restitution - rel_vel - gap_with_slop * inv_dt)
    } else {
        (-gap_with_slop * ERP2 * inv_dt, restitution - rel_vel)
    };

    let normal_row = TwoBodyRow {
        direction: contact.normal,
        torque_axis_a: normal_torque_axis_a,
        angular_component_a: normal_angular_component_a,
        torque_axis_b: normal_torque_axis_b,
        angular_component_b: normal_angular_component_b,
        jac_diag_ab_inv,
        rhs: (positional_error + velocity_error) * jac_diag_ab_inv,
        cfm: GLOBAL_CFM * jac_diag_ab_inv,
        lower_limit: 0.0,
        upper_limit: UPPER_LIMIT,
        applied_impulse: 0.0,
    };

    let (t1, t2) = plane_space(&contact.normal);
    let friction_row = |dir: Vec3| -> TwoBodyRow {
        let (torque_axis_a, angular_component_a, torque_axis_b, angular_component_b, denom) =
            effective_mass_denom_two_body(a, b, &rel_pos_a, &rel_pos_b, &dir);
        let jac_diag_ab_inv = RELAXATION / (denom + GLOBAL_CFM);
        let rel_vel = relative_velocity_along(&dir);
        TwoBodyRow {
            direction: dir,
            torque_axis_a,
            angular_component_a,
            torque_axis_b,
            angular_component_b,
            jac_diag_ab_inv,
            rhs: -rel_vel * jac_diag_ab_inv,
            cfm: 0.0,
            lower_limit: 0.0,
            upper_limit: 0.0,
            applied_impulse: 0.0,
        }
    };

    [normal_row, friction_row(t1), friction_row(t2)]
}

/// Two-body version of `resolve_row`: the relative-velocity term along a
/// row's direction is body A's contribution minus body B's, and a solved
/// impulse pushes A along `+direction` and B along `-direction` (Newton's
/// third law) — matching `contact_between`'s normal convention (points
/// from B toward A).
fn resolve_two_body_row(
    row: &mut TwoBodyRow,
    inv_mass_a: f32,
    inv_mass_b: f32,
    delta: &mut TwoBodyDelta,
) {
    let delta_vel_dot_n = row.direction.dot(&delta.linear_a)
        + row.torque_axis_a.dot(&delta.angular_a)
        - row.direction.dot(&delta.linear_b)
        - row.torque_axis_b.dot(&delta.angular_b);

    let mut delta_impulse = row.rhs - row.applied_impulse * row.cfm;
    delta_impulse -= delta_vel_dot_n * row.jac_diag_ab_inv;

    let sum = row.applied_impulse + delta_impulse;
    if sum < row.lower_limit {
        delta_impulse = row.lower_limit - row.applied_impulse;
        row.applied_impulse = row.lower_limit;
    } else if sum > row.upper_limit {
        delta_impulse = row.upper_limit - row.applied_impulse;
        row.applied_impulse = row.upper_limit;
    } else {
        row.applied_impulse = sum;
    }

    delta.linear_a += row.direction * (inv_mass_a * delta_impulse);
    delta.angular_a += row.angular_component_a * delta_impulse;
    delta.linear_b -= row.direction * (inv_mass_b * delta_impulse);
    delta.angular_b -= row.angular_component_b * delta_impulse;
}

/// Resolves one contact point between two dynamic bodies (`a`, `b`) —
/// `contact` must have come from `collision::contact_between(a, b)` (its
/// `normal` convention and `rel_pos` derivation both assume that argument
/// order). Only ever called with exactly one contact (unlike
/// `resolve_contacts`'s manifold): `sphere_vs_box`, the only two-dynamic-
/// body pairing in this scope, always produces at most one point.
pub fn resolve_contact_between(a: &mut RigidBody, b: &mut RigidBody, contact: &Contact, dt: f32) {
    let combined_restitution = combine_restitution(a.restitution, b.restitution);
    let combined_friction = combine_friction(a.friction, b.friction);

    let mut rows = setup_two_body_rows(a, b, contact, combined_restitution, dt);
    let mut delta = TwoBodyDelta::zero();
    let inv_mass_a = a.inv_mass();
    let inv_mass_b = b.inv_mass();

    for _ in 0..SOLVER_ITERATIONS {
        resolve_two_body_row(&mut rows[0], inv_mass_a, inv_mass_b, &mut delta);

        let friction_limit = combined_friction * rows[0].applied_impulse;
        rows[1].lower_limit = -friction_limit;
        rows[1].upper_limit = friction_limit;
        rows[2].lower_limit = -friction_limit;
        rows[2].upper_limit = friction_limit;

        resolve_two_body_row(&mut rows[1], inv_mass_a, inv_mass_b, &mut delta);
        resolve_two_body_row(&mut rows[2], inv_mass_a, inv_mass_b, &mut delta);
    }

    a.linear_velocity += delta.linear_a;
    a.angular_velocity += delta.angular_a;
    b.linear_velocity += delta.linear_b;
    b.angular_velocity += delta.angular_b;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::collision::{contact_between, contacts_vs_plane};

    fn ground() -> StaticPlane {
        StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0)
    }

    fn resting_sphere() -> RigidBody {
        let mut s = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        s.restitution = 0.0;
        s.friction = 0.5;
        s
    }

    fn resolve_single(body: &mut RigidBody, plane: &StaticPlane, dt: f32) {
        let contacts = contacts_vs_plane(body, plane);
        resolve_contacts(body, plane, &contacts, dt);
    }

    #[test]
    fn resting_sphere_with_zero_restitution_has_zero_bounce_velocity() {
        let mut s = resting_sphere();
        let ground = ground();
        resolve_single(&mut s, &ground, 1.0 / 60.0);
        assert!(s.linear_velocity.z.abs() < 1e-4);
    }

    #[test]
    fn downward_impact_bounces_up_proportional_to_restitution() {
        let mut s = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        s.restitution = 1.0;
        s.linear_velocity = Vec3::new(0.0, 0.0, -10.0);
        let ground = ground();
        resolve_single(&mut s, &ground, 1.0 / 60.0);
        // Combined restitution averages sphere (1.0) and plane (0.5
        // default) to 0.75, so expect a strong but not fully elastic bounce.
        assert!(
            s.linear_velocity.z > 6.0,
            "expected a strong bounce, got {}",
            s.linear_velocity.z
        );
    }

    #[test]
    fn zero_restitution_impact_does_not_bounce_past_the_planes_own_restitution() {
        let mut s = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        s.restitution = 0.0;
        s.linear_velocity = Vec3::new(0.0, 0.0, -10.0);
        // Zero out the plane's contribution too, isolating "no bounce" from
        // the combine-mode averaging tested above.
        let ground = StaticPlane {
            restitution: 0.0,
            ..ground()
        };
        resolve_single(&mut s, &ground, 1.0 / 60.0);
        assert!(
            s.linear_velocity.z <= 1e-3,
            "expected no bounce, got {}",
            s.linear_velocity.z
        );
    }

    #[test]
    fn sliding_sphere_decelerates_due_to_friction() {
        let mut s = resting_sphere();
        // A resting contact only generates a normal impulse (and hence a
        // nonzero friction limit, which is derived from it) when there's
        // an inward velocity for the solver to resist — exactly what
        // `PhysicsWorld::step` provides every frame via gravity. Mirror
        // that here rather than testing an unrealistic zero-normal-force
        // contact, where friction is correctly zero (nothing to grip
        // against).
        s.linear_velocity = Vec3::new(5.0, 0.0, -1.0);
        let ground = ground();
        resolve_single(&mut s, &ground, 1.0 / 60.0);
        assert!(
            s.linear_velocity.x < 5.0,
            "friction should have removed some tangential speed"
        );
        // Friction couples into spin for a sphere in contact.
        assert!(s.angular_velocity.length() > 0.0);
    }

    #[test]
    fn resolve_contacts_with_an_empty_manifold_is_a_no_op() {
        let mut s = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 10.0));
        let before = s.linear_velocity;
        resolve_contacts(&mut s, &ground(), &[], 1.0 / 60.0);
        assert_eq!(s.linear_velocity, before);
    }

    #[test]
    fn resting_box_with_symmetric_contacts_settles_without_net_rotation() {
        // A box resting flat and falling straight down generates 4
        // symmetric contacts (RB-PHYSICS-001-FR-004's multi-contact case);
        // resolving them together should cancel out to zero net torque by
        // symmetry, not spin the box from solving corners one at a time.
        let mut b = RigidBody::car_box(Vec3::new(1.0, 1.0, 0.5), 1.0, Vec3::new(0.0, 0.0, 0.5));
        b.restitution = 0.0;
        b.friction = 0.5;
        b.linear_velocity = Vec3::new(0.0, 0.0, -1.0);
        let ground = StaticPlane {
            restitution: 0.0,
            ..ground()
        };
        let contacts = contacts_vs_plane(&b, &ground);
        assert_eq!(contacts.len(), 4);
        resolve_contacts(&mut b, &ground, &contacts, 1.0 / 60.0);
        assert!(b.linear_velocity.z.abs() < 1e-3);
        assert!(
            b.angular_velocity.length() < 1e-3,
            "expected no net spin from symmetric contacts, got {:?}",
            b.angular_velocity
        );
    }

    #[test]
    fn plane_space_directions_are_orthonormal_and_perpendicular_to_normal() {
        let n = Vec3::new(0.0, 0.0, 1.0).normalize().unwrap();
        let (p, q) = plane_space(&n);
        assert!(n.dot(&p).abs() < 1e-6);
        assert!(n.dot(&q).abs() < 1e-6);
        assert!(p.dot(&q).abs() < 1e-6);
        assert!((p.length() - 1.0).abs() < 1e-5);
        assert!((q.length() - 1.0).abs() < 1e-5);
    }

    fn car_at_origin() -> RigidBody {
        RigidBody::car_box(Vec3::new(60.0, 30.0, 18.0), 180.0, Vec3::ZERO)
    }

    fn overlapping_ball() -> RigidBody {
        RigidBody::sphere(92.75, 1.0, Vec3::new(60.0 + 50.0, 0.0, 0.0))
    }

    /// Zero penetration on purpose: the deep overlap `overlapping_ball()`
    /// gives is realistic for "detected mid-frame", but its Baumgarte
    /// positional-correction term (proportional to penetration depth) then
    /// dominates the post-solve relative velocity, which isn't what this
    /// test means to check — same reasoning as `downward_impact_bounces_up_
    /// proportional_to_restitution` using an exactly-touching sphere.
    fn touching_ball() -> RigidBody {
        RigidBody::sphere(92.75, 1.0, Vec3::new(60.0 + 92.75, 0.0, 0.0))
    }

    #[test]
    fn inelastic_head_on_collision_leaves_no_residual_closing_speed() {
        let mut ball = touching_ball();
        ball.restitution = 0.0;
        ball.linear_velocity = Vec3::new(-100.0, 0.0, 0.0);
        let mut car = car_at_origin();
        car.restitution = 0.0;
        let contact = contact_between(&ball, &car).unwrap();
        resolve_contact_between(&mut ball, &mut car, &contact, 1.0 / 60.0);
        let rel_vel = contact
            .normal
            .dot(&(ball.linear_velocity - car.linear_velocity));
        assert!(
            rel_vel.abs() < 1e-2,
            "expected no residual closing speed, got {rel_vel}"
        );
    }

    #[test]
    fn collision_conserves_linear_momentum() {
        let mut ball = overlapping_ball();
        ball.restitution = 0.8;
        ball.linear_velocity = Vec3::new(-500.0, 0.0, 0.0);
        let mut car = car_at_origin();
        car.restitution = 0.8;
        car.linear_velocity = Vec3::new(50.0, 0.0, 0.0);
        let before = ball.linear_velocity * ball.mass() + car.linear_velocity * car.mass();
        let contact = contact_between(&ball, &car).unwrap();
        resolve_contact_between(&mut ball, &mut car, &contact, 1.0 / 60.0);
        let after = ball.linear_velocity * ball.mass() + car.linear_velocity * car.mass();
        assert!(
            (before - after).length() < 1.0,
            "expected momentum conservation, before={before:?} after={after:?}"
        );
    }

    #[test]
    fn a_much_heavier_body_barely_moves_from_the_collision() {
        // The car (mass 180) is vastly heavier than the ball (mass 1) —
        // the ball should bounce back while the car barely budges, the
        // same qualitative behavior as a ball bouncing off a wall.
        let mut ball = touching_ball();
        ball.restitution = 0.6;
        ball.linear_velocity = Vec3::new(-500.0, 0.0, 0.0);
        let mut car = car_at_origin();
        car.restitution = 0.6;
        let contact = contact_between(&ball, &car).unwrap();
        resolve_contact_between(&mut ball, &mut car, &contact, 1.0 / 60.0);
        assert!(
            ball.linear_velocity.x > 0.0,
            "expected the light ball to bounce back, got {}",
            ball.linear_velocity.x
        );
        assert!(
            car.linear_velocity.x.abs() < 10.0,
            "expected the much heavier car to barely move, got {}",
            car.linear_velocity.x
        );
    }
}
