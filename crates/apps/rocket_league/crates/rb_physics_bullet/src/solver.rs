//! Sequential-impulse contact solver, ported from
//! `bullet3/src/BulletDynamics/ConstraintSolver/btSequentialImpulseConstraintSolver.cpp`
//! (zlib license — see `THIRD_PARTY_NOTICES.md`), scoped to one dynamic
//! sphere against one static plane per contact.
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
//! - **Scalar inertia only.** Valid for a sphere (isotropic inertia); a
//!   general 3x3 inverse inertia tensor is required once box-shaped car
//!   bodies are added.
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

use crate::body::{Sphere, StaticPlane};
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

fn effective_mass_denom(sphere: &Sphere, rel_pos: &Vec3, direction: &Vec3) -> (Vec3, Vec3, f32) {
    let torque_axis = rel_pos.cross(direction);
    let angular_component = torque_axis * sphere.inv_inertia();
    let denom = sphere.inv_mass() + direction.dot(&angular_component.cross(rel_pos));
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
fn setup_rows(sphere: &Sphere, contact: &Contact, dt: f32) -> [ConstraintRow; 3] {
    let rel_pos = contact.point - sphere.position;
    let inv_dt = 1.0 / dt;

    let (normal_torque_axis, normal_angular_component, denom) =
        effective_mass_denom(sphere, &rel_pos, &contact.normal);
    let jac_diag_ab_inv = RELAXATION / (denom + GLOBAL_CFM);

    let rel_vel = contact.normal.dot(&sphere.velocity_at_point(&rel_pos));
    let restitution =
        restitution_curve(rel_vel, sphere.restitution, RESTITUTION_VELOCITY_THRESHOLD);

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
        let (torque_axis, angular_component, denom) = effective_mass_denom(sphere, &rel_pos, &dir);
        let jac_diag_ab_inv = RELAXATION / (denom + GLOBAL_CFM);
        let rel_vel = dir.dot(&sphere.velocity_at_point(&rel_pos));
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
fn resolve_row(row: &mut ConstraintRow, sphere_inv_mass: f32, delta: &mut DeltaVelocity) {
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

    delta.linear += row.direction * (sphere_inv_mass * delta_impulse);
    delta.angular += row.angular_component * delta_impulse;
}

/// Resolves one sphere-vs-plane contact by running `SOLVER_ITERATIONS`
/// passes of the sequential impulse solver (normal row, then both friction
/// rows with limits re-derived from the normal row's current impulse —
/// matching Bullet's per-iteration friction reclamping), then applies the
/// accumulated velocity change to `sphere` once.
pub fn resolve_contact(sphere: &mut Sphere, plane: &StaticPlane, contact: &Contact, dt: f32) {
    let combined_restitution = combine_restitution(sphere.restitution, plane.restitution);
    let combined_friction = combine_friction(sphere.friction, plane.friction);

    let mut effective_sphere = *sphere;
    effective_sphere.restitution = combined_restitution;

    let mut rows = setup_rows(&effective_sphere, contact, dt);
    let mut delta = DeltaVelocity::zero();
    let inv_mass = sphere.inv_mass();

    for _ in 0..SOLVER_ITERATIONS {
        resolve_row(&mut rows[0], inv_mass, &mut delta);

        let friction_limit = combined_friction * rows[0].applied_impulse;
        rows[1].lower_limit = -friction_limit;
        rows[1].upper_limit = friction_limit;
        rows[2].lower_limit = -friction_limit;
        rows[2].upper_limit = friction_limit;

        resolve_row(&mut rows[1], inv_mass, &mut delta);
        resolve_row(&mut rows[2], inv_mass, &mut delta);
    }

    sphere.linear_velocity += delta.linear;
    sphere.angular_velocity += delta.angular;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::collision::sphere_vs_plane;

    fn ground() -> StaticPlane {
        StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0)
    }

    fn resting_sphere() -> Sphere {
        let mut s = Sphere::new(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        s.restitution = 0.0;
        s.friction = 0.5;
        s
    }

    #[test]
    fn resting_sphere_with_zero_restitution_has_zero_bounce_velocity() {
        let mut s = resting_sphere();
        let ground = ground();
        let contact = sphere_vs_plane(&s, &ground).unwrap();
        resolve_contact(&mut s, &ground, &contact, 1.0 / 60.0);
        assert!(s.linear_velocity.z.abs() < 1e-4);
    }

    #[test]
    fn downward_impact_bounces_up_proportional_to_restitution() {
        let mut s = Sphere::new(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        s.restitution = 1.0;
        s.linear_velocity = Vec3::new(0.0, 0.0, -10.0);
        let ground = ground();
        let contact = sphere_vs_plane(&s, &ground).unwrap();
        resolve_contact(&mut s, &ground, &contact, 1.0 / 60.0);
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
        let mut s = Sphere::new(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        s.restitution = 0.0;
        s.linear_velocity = Vec3::new(0.0, 0.0, -10.0);
        // Zero out the plane's contribution too, isolating "no bounce" from
        // the combine-mode averaging tested above.
        let ground = StaticPlane {
            restitution: 0.0,
            ..ground()
        };
        let contact = sphere_vs_plane(&s, &ground).unwrap();
        resolve_contact(&mut s, &ground, &contact, 1.0 / 60.0);
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
        let contact = sphere_vs_plane(&s, &ground).unwrap();
        resolve_contact(&mut s, &ground, &contact, 1.0 / 60.0);
        assert!(
            s.linear_velocity.x < 5.0,
            "friction should have removed some tangential speed"
        );
        // Friction couples into spin for a sphere in contact.
        assert!(s.angular_velocity.length() > 0.0);
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
}
