//! Sequential-impulse contact solver, ported from
//! `bullet3/src/BulletDynamics/ConstraintSolver/btSequentialImpulseConstraintSolver.cpp`
//! (zlib license — see `THIRD_PARTY_NOTICES.md`). Two paths:
//! - `resolve_contacts`: one dynamic body (sphere or box, via `RigidBody`'s
//!   general 3x3 inverse inertia tensor — see `body.rs`) against one static
//!   body's contact manifold (1 to 4 points depending on shape/orientation)
//!   — the static body is identified only by its restitution/friction, so
//!   this same path serves a `StaticPlane` and, since
//!   `RB-PHYSICS-001-FR-020`, a `StaticQuarterPipe` fillet equally.
//! - `resolve_contacts_between`: two dynamic bodies against each other's
//!   contact manifold (1 point for sphere-vs-box or an edge-edge box
//!   contact, up to 4 for a box-vs-box face contact — see `collision`).
//!   This is the generic two-body path Bullet's real solver always runs
//!   (every constraint row carries both bodies' mass/inertia
//!   contributions); `resolve_contacts` only got away with a one-body-only
//!   version because a static plane's side of that math is always zero.
//! - `resolve_dynamic_manifolds` (`RB-PHYSICS-001-FR-030`): every
//!   dynamic-vs-dynamic manifold in the scene at once, sharing one
//!   `DeltaVelocity` accumulator per body index across every manifold that
//!   body takes part in — the combined multi-body solve
//!   `resolve_contacts_between` alone can't give a body touching two others
//!   in the same step (see its own doc comment).
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

use crate::body::RigidBody;
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
/// body, identified only by its `restitution`/`friction` (its actual shape
/// — a `StaticPlane` or, since `RB-PHYSICS-001-FR-020`, a
/// `StaticQuarterPipe` — is irrelevant here: every `Contact`'s normal/point/
/// depth is already fully resolved by the caller's own narrow-phase test,
/// so this function never needs the static shape itself, only the two
/// material properties it combines with `body`'s own). Runs
/// `SOLVER_ITERATIONS` passes of the sequential impulse solver, each pass
/// resolving every contact's normal row and then every contact's friction
/// rows (limits re-derived from that same contact's current normal impulse
/// — matching Bullet's per-iteration friction reclamping), sharing one
/// accumulated `DeltaVelocity` across the whole manifold so an earlier
/// contact's resolution already influences a later contact's `rhs`
/// baseline within the same iteration — then applies the accumulated
/// velocity change to `body` once.
pub fn resolve_contacts(
    body: &mut RigidBody,
    static_restitution: f32,
    static_friction: f32,
    contacts: &[Contact],
    dt: f32,
) {
    if contacts.is_empty() {
        return;
    }

    let combined_restitution = combine_restitution(body.restitution, static_restitution);
    let combined_friction = combine_friction(body.friction, static_friction);

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
/// third law) — matching `contacts_between`'s normal convention (points
/// from B toward A). Takes each body's `DeltaVelocity` accumulator
/// separately (rather than one combined struct) so `resolve_dynamic_manifolds`
/// can share a single accumulator per body index across every manifold that
/// body takes part in, not just the one pair currently being resolved.
fn resolve_two_body_row(
    row: &mut TwoBodyRow,
    inv_mass_a: f32,
    inv_mass_b: f32,
    delta_a: &mut DeltaVelocity,
    delta_b: &mut DeltaVelocity,
) {
    let delta_vel_dot_n = row.direction.dot(&delta_a.linear)
        + row.torque_axis_a.dot(&delta_a.angular)
        - row.direction.dot(&delta_b.linear)
        - row.torque_axis_b.dot(&delta_b.angular);

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

    delta_a.linear += row.direction * (inv_mass_a * delta_impulse);
    delta_a.angular += row.angular_component_a * delta_impulse;
    delta_b.linear -= row.direction * (inv_mass_b * delta_impulse);
    delta_b.angular -= row.angular_component_b * delta_impulse;
}

/// Resolves an entire contact manifold (1 to 4 points — a box-vs-box face
/// contact can produce up to 4, an edge-edge or sphere-vs-box contact
/// exactly 1) between two dynamic bodies (`a`, `b`) — every `Contact` must
/// have come from `collision::contacts_between(a, b)` (its `normal`
/// convention and `rel_pos` derivation both assume that argument order).
/// Mirrors `resolve_contacts`' shared-delta multi-iteration structure
/// (see its doc comment), generalized to two dynamic bodies instead of one
/// dynamic body against a static plane.
pub fn resolve_contacts_between(
    a: &mut RigidBody,
    b: &mut RigidBody,
    contacts: &[Contact],
    dt: f32,
) {
    if contacts.is_empty() {
        return;
    }

    let combined_restitution = combine_restitution(a.restitution, b.restitution);
    let combined_friction = combine_friction(a.friction, b.friction);

    let mut manifold: Vec<[TwoBodyRow; 3]> = contacts
        .iter()
        .map(|c| setup_two_body_rows(a, b, c, combined_restitution, dt))
        .collect();
    let mut delta_a = DeltaVelocity::zero();
    let mut delta_b = DeltaVelocity::zero();
    let inv_mass_a = a.inv_mass();
    let inv_mass_b = b.inv_mass();

    for _ in 0..SOLVER_ITERATIONS {
        for rows in &mut manifold {
            resolve_two_body_row(
                &mut rows[0],
                inv_mass_a,
                inv_mass_b,
                &mut delta_a,
                &mut delta_b,
            );

            let friction_limit = combined_friction * rows[0].applied_impulse;
            rows[1].lower_limit = -friction_limit;
            rows[1].upper_limit = friction_limit;
            rows[2].lower_limit = -friction_limit;
            rows[2].upper_limit = friction_limit;

            resolve_two_body_row(
                &mut rows[1],
                inv_mass_a,
                inv_mass_b,
                &mut delta_a,
                &mut delta_b,
            );
            resolve_two_body_row(
                &mut rows[2],
                inv_mass_a,
                inv_mass_b,
                &mut delta_a,
                &mut delta_b,
            );
        }
    }

    a.linear_velocity += delta_a.linear;
    a.angular_velocity += delta_a.angular;
    b.linear_velocity += delta_b.linear;
    b.angular_velocity += delta_b.angular;
}

/// Index of each body taking part in a `resolve_dynamic_manifolds` pair,
/// paired with mutable access to its own `DeltaVelocity` accumulator without
/// a duplicate-borrow error — the general (arbitrary `a`/`b`, not just
/// `b == a + 1`) version of the `Vec::split_at_mut` trick
/// `PhysicsWorld::step` already uses for its car-vs-car loop.
fn delta_pair_mut(
    deltas: &mut [DeltaVelocity],
    a: usize,
    b: usize,
) -> (&mut DeltaVelocity, &mut DeltaVelocity) {
    assert_ne!(a, b, "a body cannot form a contact manifold with itself");
    if a < b {
        let (left, right) = deltas.split_at_mut(b);
        (&mut left[a], &mut right[0])
    } else {
        let (left, right) = deltas.split_at_mut(a);
        (&mut right[0], &mut left[b])
    }
}

/// Resolves every dynamic-vs-dynamic contact manifold in the scene (every
/// ball-vs-car and car-vs-car pair with at least one contact this step) as
/// one shared island solve, fixing `RB-PHYSICS-001-FR-030`'s "combined
/// multi-body solve" gap: calling `resolve_contacts_between` once per pair,
/// as `PhysicsWorld::step` did before this function existed, fully resolves
/// and applies one pair's `SOLVER_ITERATIONS` iterations before the next
/// pair's setup even reads a body's velocity — a body touching two others
/// in the same step (e.g. a car pinned between the ball and another car)
/// never has both contacts reasoned about together, only sequentially, each
/// one seeing the other's already-*finished* correction rather than genuinely
/// sharing the solve.
///
/// Here, every body index that takes part in at least one manifold gets its
/// own `DeltaVelocity` accumulator (`deltas[i]`, indexed the same way as
/// `bodies`), and every manifold's rows draw from and add to whichever two
/// accumulators its own two body indices name — shared across the *whole*
/// `SOLVER_ITERATIONS` loop, not just within one manifold's own rows, so a
/// third body's contact genuinely participates in the same convergence as
/// the other two, the way Bullet's real per-island solver does (see this
/// module's own doc comment for what's still simplified relative to that:
/// no split impulse, no warm-starting, average rather than max combine
/// mode). `manifolds` is `(index_a, index_b, contacts)` triples indexing
/// into `bodies`; omit a pair from `manifolds` entirely rather than passing
/// it with an empty `contacts` (an empty manifold would still allocate a
/// `DeltaVelocity`-touching no-op).
///
/// Static contacts (ground, arena walls, curves, goal geometry) are
/// deliberately NOT part of this shared solve — each body's own contact
/// with a *static* surface only depends on that one body, so resolving it
/// independently (via `resolve_contacts`, unchanged) loses no cross-body
/// information; the actual gap this function closes is specifically the
/// dynamic-vs-dynamic case a static contact can't have.
pub fn resolve_dynamic_manifolds(
    bodies: &mut [RigidBody],
    manifolds: &[(usize, usize, Vec<Contact>)],
    dt: f32,
) {
    if manifolds.is_empty() {
        return;
    }

    struct Manifold {
        a: usize,
        b: usize,
        combined_friction: f32,
        rows: Vec<[TwoBodyRow; 3]>,
    }

    let mut solved: Vec<Manifold> = manifolds
        .iter()
        .map(|(a, b, contacts)| {
            let combined_restitution =
                combine_restitution(bodies[*a].restitution, bodies[*b].restitution);
            let combined_friction = combine_friction(bodies[*a].friction, bodies[*b].friction);
            let rows = contacts
                .iter()
                .map(|c| setup_two_body_rows(&bodies[*a], &bodies[*b], c, combined_restitution, dt))
                .collect();
            Manifold {
                a: *a,
                b: *b,
                combined_friction,
                rows,
            }
        })
        .collect();

    let inv_masses: Vec<f32> = bodies.iter().map(RigidBody::inv_mass).collect();
    let mut deltas: Vec<DeltaVelocity> = (0..bodies.len()).map(|_| DeltaVelocity::zero()).collect();

    for _ in 0..SOLVER_ITERATIONS {
        for m in &mut solved {
            let inv_mass_a = inv_masses[m.a];
            let inv_mass_b = inv_masses[m.b];
            let (delta_a, delta_b) = delta_pair_mut(&mut deltas, m.a, m.b);
            for rows in &mut m.rows {
                resolve_two_body_row(&mut rows[0], inv_mass_a, inv_mass_b, delta_a, delta_b);

                let friction_limit = m.combined_friction * rows[0].applied_impulse;
                rows[1].lower_limit = -friction_limit;
                rows[1].upper_limit = friction_limit;
                rows[2].lower_limit = -friction_limit;
                rows[2].upper_limit = friction_limit;

                resolve_two_body_row(&mut rows[1], inv_mass_a, inv_mass_b, delta_a, delta_b);
                resolve_two_body_row(&mut rows[2], inv_mass_a, inv_mass_b, delta_a, delta_b);
            }
        }
    }

    for (body, delta) in bodies.iter_mut().zip(deltas.iter()) {
        body.linear_velocity += delta.linear;
        body.angular_velocity += delta.angular;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::body::StaticPlane;
    use crate::collision::{contacts_between, contacts_vs_plane};

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
        resolve_contacts(body, plane.restitution, plane.friction, &contacts, dt);
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
        let g = ground();
        resolve_contacts(&mut s, g.restitution, g.friction, &[], 1.0 / 60.0);
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
        resolve_contacts(
            &mut b,
            ground.restitution,
            ground.friction,
            &contacts,
            1.0 / 60.0,
        );
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
        let contacts = contacts_between(&ball, &car);
        resolve_contacts_between(&mut ball, &mut car, &contacts, 1.0 / 60.0);
        let rel_vel = contacts[0]
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
        let contacts = contacts_between(&ball, &car);
        resolve_contacts_between(&mut ball, &mut car, &contacts, 1.0 / 60.0);
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
        let contacts = contacts_between(&ball, &car);
        resolve_contacts_between(&mut ball, &mut car, &contacts, 1.0 / 60.0);
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

    /// Symmetric setup for `resolve_dynamic_manifolds_keeps_more_of_every_bodys_contact_than_resolving_pairs_independently`:
    /// a ball at the origin exactly touching two identical cars closing in
    /// from either side at equal and opposite speed. Zero restitution and
    /// exact (zero-penetration) contact throughout, same reasoning as
    /// `touching_ball`, isolates the multi-body coupling this test checks
    /// from restitution bounce and Baumgarte positional correction.
    fn symmetric_pinch() -> (RigidBody, RigidBody, RigidBody) {
        let half = Vec3::new(60.0, 30.0, 18.0);
        let ball_radius = 92.75;
        let gap = half.x + ball_radius;

        let mut ball = RigidBody::sphere(ball_radius, 1.0, Vec3::ZERO);
        ball.restitution = 0.0;
        let mut left = RigidBody::car_box(half, 180.0, Vec3::new(-gap, 0.0, 0.0));
        left.restitution = 0.0;
        left.linear_velocity = Vec3::new(100.0, 0.0, 0.0);
        let mut right = RigidBody::car_box(half, 180.0, Vec3::new(gap, 0.0, 0.0));
        right.restitution = 0.0;
        right.linear_velocity = Vec3::new(-100.0, 0.0, 0.0);
        (ball, left, right)
    }

    #[test]
    fn resolve_dynamic_manifolds_keeps_more_of_every_bodys_contact_than_resolving_pairs_independently(
    ) {
        // The real point of RB-PHYSICS-001-FR-030: a ball exactly, mutually
        // touching two cars closing on it from opposite sides at equal
        // speed is left-right symmetric, so a true simultaneous solve must
        // leave it near stationary (both contacts equally constrain it,
        // and total momentum is exactly zero). Resolving each pair to its
        // own full, independent convergence — one call to
        // `resolve_contacts_between` per pair, the pre-FR-030 shape of
        // `PhysicsWorld::step` — can't see that: the *second* call's setup
        // reads the ball's velocity only *after* the first pair's contact
        // has already been fully solved and applied, so the ball ends up
        // essentially adopting whichever car was resolved last (about 99%
        // of that car's own closing speed), as if the first contact barely
        // mattered. `resolve_dynamic_manifolds` shares one accumulator per
        // body across both manifolds for the whole solve instead, so
        // neither contact's information gets thrown away by the other —
        // it doesn't fully converge to the true zero-velocity answer in
        // just `SOLVER_ITERATIONS` iterations for this extreme a mass
        // ratio (ball mass 1 vs. car mass 180 — a known, common limitation
        // of projected Gauss-Seidel solvers for a light body sandwiched
        // between two heavy ones, not unique to this port), but it must
        // land measurably closer to it.
        let (mut ball_a, mut left_a, mut right_a) = symmetric_pinch();
        let contacts_left = contacts_between(&ball_a, &left_a);
        resolve_contacts_between(&mut ball_a, &mut left_a, &contacts_left, 1.0 / 60.0);
        let contacts_right = contacts_between(&ball_a, &right_a);
        resolve_contacts_between(&mut ball_a, &mut right_a, &contacts_right, 1.0 / 60.0);
        let independent_ball_speed = ball_a.linear_velocity.x.abs();

        let (ball_b, left_b, right_b) = symmetric_pinch();
        let mut bodies = vec![ball_b, left_b, right_b];
        let manifolds = vec![
            (0usize, 1usize, contacts_between(&bodies[0], &bodies[1])),
            (0usize, 2usize, contacts_between(&bodies[0], &bodies[2])),
        ];
        resolve_dynamic_manifolds(&mut bodies, &manifolds, 1.0 / 60.0);
        let combined_ball_speed = bodies[0].linear_velocity.x.abs();

        assert!(
            independent_ball_speed > 98.0,
            "expected resolving each pair independently to leave the ball near a single car's \
             own closing speed, got {independent_ball_speed}"
        );
        assert!(
            combined_ball_speed < independent_ball_speed - 5.0,
            "expected the combined solve to leave the ball measurably slower than resolving \
             each pair independently, independent={independent_ball_speed}, \
             combined={combined_ball_speed}"
        );
    }

    #[test]
    fn boxes_colliding_face_to_face_settle_without_net_rotation() {
        // Two identical boxes closing head-on, face-to-face: a box-vs-box
        // manifold (4 symmetric contacts, unlike sphere-vs-box's single
        // point) resolved between two dynamic bodies should cancel out to
        // zero net spin by symmetry, the same property
        // `resting_box_with_symmetric_contacts_settles_without_net_rotation`
        // checks for the one-body ground-manifold case.
        let mut a = RigidBody::car_box(Vec3::new(10.0, 10.0, 10.0), 1.0, Vec3::ZERO);
        a.restitution = 0.0;
        a.linear_velocity = Vec3::new(1.0, 0.0, 0.0);
        let mut b = RigidBody::car_box(Vec3::new(10.0, 10.0, 10.0), 1.0, Vec3::new(15.0, 0.0, 0.0));
        b.restitution = 0.0;
        b.linear_velocity = Vec3::new(-1.0, 0.0, 0.0);

        let contacts = contacts_between(&a, &b);
        assert_eq!(contacts.len(), 4);
        resolve_contacts_between(&mut a, &mut b, &contacts, 1.0 / 60.0);

        assert!(
            a.angular_velocity.length() < 1e-3,
            "expected no net spin on a, got {:?}",
            a.angular_velocity
        );
        assert!(
            b.angular_velocity.length() < 1e-3,
            "expected no net spin on b, got {:?}",
            b.angular_velocity
        );
    }
}
