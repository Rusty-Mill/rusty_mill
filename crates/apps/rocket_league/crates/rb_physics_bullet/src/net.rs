//! A genuine mass-spring net panel (`RB-PHYSICS-001-FR-033`), replacing
//! part of `RB-PHYSICS-001-FR-029`'s solid-bounding-box stand-in with real
//! springy/catching behavior for the ball — the "ball tangles in netting"
//! case `RB-PHYSICS-001`'s own Non-goals had explicitly deferred ever since
//! FR-029 shipped.
//!
//! `NetMesh` is a rectangular grid of point masses (`points`, each a tiny
//! `RigidBody::sphere` — reusing this crate's existing rigid-body/collision/
//! solver machinery rather than inventing a bespoke penalty-force system)
//! connected by structural (horizontal/vertical) and shear (diagonal)
//! springs (`springs`, Hooke's law plus velocity damping along each
//! spring's own axis). Every point on the grid's own perimeter is anchored
//! (`anchored[i] == true`) — fixed in place, representing the net's real
//! attachment to the rigid goal frame (crossbar, both posts, and the
//! ground/back line) — while every interior point is free to move under
//! gravity, spring forces, and ball contact.
//!
//! `NetMesh::step` advances the net's own internal physics by the caller's
//! `dt`, split into `NET_SUBSTEPS` smaller sub-steps (a mass-spring system
//! this stiff would go numerically unstable integrated with Bullet's own
//! typical single-step `dt` values, e.g. 1/60s) and resolves every body's
//! contact against every free point it currently overlaps via
//! `collision::sphere_vs_sphere`/`solver::resolve_dynamic_manifolds` (since
//! `RB-PHYSICS-001-FR-050` — see `step`'s own doc comment) — the same
//! two-dynamic-body sequential-impulse machinery every other dynamic-vs-
//! dynamic contact in this crate already uses (ball-vs-car, car-vs-car),
//! not a special-cased shortcut. This is why a net point is a real (if
//! artificially tiny and light) `RigidBody` rather than a plain `Vec3`
//! position/velocity pair: it lets this module add zero new solver code,
//! only the spring-force accumulation Hooke's law itself needs and the
//! sphere-vs-sphere contact test this crate had no prior caller for (see
//! `collision::sphere_vs_sphere`'s own doc comment).
//!
//! `NET_POINT_MASS`, `NET_POINT_RADIUS`, `NET_SPRING_CONSTANT`,
//! `NET_SPRING_DAMPING`, `NET_LINEAR_DAMPING`, `NET_RESTITUTION`, and
//! `NET_FRICTION` are all uncalibrated placeholders, the same
//! "no public reference exists for this, tuned empirically for stable,
//! plausible behavior" category `RB-PHYSICS-001-FR-031`'s audit already
//! flagged several other constants under (e.g. `drive::AIR_CONTROL_TORQUE`)
//! — real Rocket League's actual net material properties have never been
//! published, and even if they had, this port's own point-mass/spring
//! topology is already a simplification of a real net's continuum cloth
//! behavior, so a "correct" numeric match isn't really a coherent target
//! yet. `NET_RESTITUTION` is deliberately low (the net *catches*, it
//! doesn't bounce the ball back out) and `NET_FRICTION` deliberately high
//! (grippy netting), matching the qualitative behavior a real net has
//! without claiming either number is measured.
//!
//! Since `RB-PHYSICS-001-FR-038`, `step` takes every body that can touch the
//! net (the ball and every car, via a `&mut [RigidBody]` slice) rather than
//! the ball alone — a car is resolved against every free point exactly the
//! same way the ball always was (`collision::contacts_between`, dispatching
//! to the same box-vs-sphere path
//! `ball_bounces_off_a_stationary_car_instead_of_passing_through` already
//! exercises for ball-vs-car), closing the "a car still passes through
//! untouched" gap this module's own doc comment used to carry as an
//! explicit Non-goal.
//!
//! Explicitly still out of scope (tracked in `RB-PHYSICS-001`, not silently
//! dropped): manifold richness beyond one contact per overlapping point per
//! body (no clipped-face-style manifold the way `box_vs_box` builds one); a
//! full 3D "sock" shape billowing backward from the goal mouth (this models
//! a single flat rest-shape panel instead, which still deforms backward
//! dynamically under a real impact via its own springs — just not a
//! pre-shaped pocket); and bending stiffness (only structural + shear
//! springs, no springs resisting the mesh folding along a diagonal) — none
//! of this crate's existing cloth-adjacent shapes need rendering-quality
//! draping, only enough structure to catch a ball or car believably.

use crate::body::RigidBody;
use crate::collision;
use crate::integrate;
use crate::solver;
use rb_domain::Vec3;
use std::collections::HashMap;

/// Mass of one free net point — deliberately light relative to a typical
/// ball mass (see this module's own doc comment on why this and the other
/// constants below are uncalibrated placeholders).
pub const NET_POINT_MASS: f32 = 0.5;
/// Radius of one net point's own collision sphere — large enough relative
/// to a typical grid spacing that a ball passing through the mesh reliably
/// contacts at least one point rather than slipping through a geometric gap
/// between them (a "coverage radius," not a literal netting-strand
/// thickness).
pub const NET_POINT_RADIUS: f32 = 120.0;
/// Hooke's-law spring constant for every structural/shear spring in the
/// mesh.
pub const NET_SPRING_CONSTANT: f32 = 400.0;
/// Damping coefficient applied to each spring's own relative velocity along
/// its axis, on top of `NET_LINEAR_DAMPING`'s whole-point damping — without
/// this, a mass-spring grid this stiff oscillates almost indefinitely once
/// disturbed.
pub const NET_SPRING_DAMPING: f32 = 6.0;
/// Whole-point exponential velocity damping (`RigidBody.linear_damping`),
/// mirroring the same "no warm-starting/sleeping, so damping is what
/// actually lets things settle" role gravity/ground damping already plays
/// elsewhere in this crate.
pub const NET_LINEAR_DAMPING: f32 = 0.6;
/// Ball-vs-net restitution: deliberately low — a real net *catches* the
/// ball rather than bouncing it back out, the opposite intent from this
/// crate's other surfaces' default 0.5.
pub const NET_RESTITUTION: f32 = 0.1;
/// Ball-vs-net friction: deliberately high (grippy netting).
pub const NET_FRICTION: f32 = 0.8;
/// How many smaller internal steps `NetMesh::step` splits the caller's own
/// `dt` into — a mass-spring system at `NET_SPRING_CONSTANT`'s stiffness
/// integrated with a single large Bullet-style step (e.g. 1/60s) goes
/// numerically unstable; sub-stepping is the standard cloth-simulation fix,
/// the same idea `solver::SOLVER_ITERATIONS` applies to contact resolution
/// rather than a literal analog of it.
pub const NET_SUBSTEPS: u32 = 8;

/// One structural or shear spring connecting two of a `NetMesh`'s own
/// `points` by index, at whatever distance separated them when the mesh was
/// built (`rest_length`) — always the flat, undeformed grid spacing for
/// `NetMesh::rectangular_grid`, since every spring is measured at
/// construction time before anything can have moved.
struct Spring {
    a: usize,
    b: usize,
    rest_length: f32,
}

/// A rectangular mass-spring net panel — see this module's own doc comment
/// for the overall design. `points[i]`'s own `restitution`/`friction`
/// fields (set once at construction to `NET_RESTITUTION`/`NET_FRICTION`)
/// are what `solver::resolve_contacts_between` actually reads when the ball
/// touches that point; `NetMesh` itself carries no separate copy of either.
pub struct NetMesh {
    pub points: Vec<RigidBody>,
    anchored: Vec<bool>,
    springs: Vec<Spring>,
}

impl NetMesh {
    /// Builds a `cols` x `rows` grid of points spanning `2 * half_width`
    /// (along `width_axis`) by `height` (along `height_axis`, starting from
    /// `center - height_axis * height * 0.5` up to `center + height_axis *
    /// height * 0.5`), centered at `center`. `width_axis`/`height_axis` must
    /// be unit length and mutually perpendicular (the same "derive, don't
    /// hardcode an axis" convention `StaticGoalWall`/`StaticBoundedWall`'s
    /// own `u_axis`/`v_axis` already use), so this works for any goal
    /// orientation, not just one hardcoded to a particular wall.
    ///
    /// Every point on the grid's own perimeter (`row == 0`, `row == rows -
    /// 1`, `col == 0`, or `col == cols - 1`) is anchored; every interior
    /// point is free. Springs connect every pair of horizontally or
    /// vertically adjacent points (structural) and every pair of diagonally
    /// adjacent points within a grid cell (shear, resisting the mesh
    /// collapsing sideways under load) — each spring's `rest_length` is
    /// measured directly from the two points' own just-computed flat-grid
    /// positions, so the mesh starts in perfect equilibrium (zero spring
    /// force) before anything disturbs it.
    ///
    /// Panics if `cols < 2 || rows < 2` — a grid that thin has no interior/
    /// perimeter distinction to speak of.
    pub fn rectangular_grid(
        center: Vec3,
        width_axis: Vec3,
        height_axis: Vec3,
        half_width: f32,
        height: f32,
        cols: usize,
        rows: usize,
    ) -> NetMesh {
        assert!(
            cols >= 2 && rows >= 2,
            "a net grid needs at least 2 columns and 2 rows"
        );

        let mut points = Vec::with_capacity(cols * rows);
        let mut anchored = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            for col in 0..cols {
                let u = (col as f32 / (cols - 1) as f32) * 2.0 - 1.0; // -1..1
                let v = (row as f32 / (rows - 1) as f32) - 0.5; // -0.5..0.5
                let position = center + width_axis * (u * half_width) + height_axis * (v * height);
                let mut point = RigidBody::sphere(NET_POINT_RADIUS, NET_POINT_MASS, position);
                point.restitution = NET_RESTITUTION;
                point.friction = NET_FRICTION;
                point.linear_damping = NET_LINEAR_DAMPING;
                points.push(point);
                anchored.push(row == 0 || row == rows - 1 || col == 0 || col == cols - 1);
            }
        }

        let index = |row: usize, col: usize| row * cols + col;
        let mut springs = Vec::new();
        for row in 0..rows {
            for col in 0..cols {
                if col + 1 < cols {
                    push_spring(&mut springs, &points, index(row, col), index(row, col + 1));
                }
                if row + 1 < rows {
                    push_spring(&mut springs, &points, index(row, col), index(row + 1, col));
                }
                if row + 1 < rows && col + 1 < cols {
                    push_spring(
                        &mut springs,
                        &points,
                        index(row, col),
                        index(row + 1, col + 1),
                    );
                    push_spring(
                        &mut springs,
                        &points,
                        index(row, col + 1),
                        index(row + 1, col),
                    );
                }
            }
        }

        NetMesh {
            points,
            anchored,
            springs,
        }
    }

    /// Whether `points[i]` is anchored (fixed in place) rather than free.
    pub fn is_anchored(&self, i: usize) -> bool {
        self.anchored[i]
    }

    /// Adds every spring's own Hooke's-law-plus-damping force to its two
    /// endpoints' force accumulators (skipping an anchored endpoint, which
    /// never accumulates force or moves) — ported from the standard
    /// mass-spring-cloth force law, not from Bullet itself (Bullet has no
    /// soft-body/cloth code this port has adopted; see ADR-0004's scope).
    fn apply_spring_forces(&mut self) {
        for spring in &self.springs {
            let pos_a = self.points[spring.a].position;
            let pos_b = self.points[spring.b].position;
            let delta = pos_b - pos_a;
            let distance = delta.length();
            if distance < 1e-6 {
                continue;
            }
            let direction = delta * (1.0 / distance);
            let stretch = distance - spring.rest_length;
            let relative_velocity = (self.points[spring.b].linear_velocity
                - self.points[spring.a].linear_velocity)
                .dot(&direction);
            let force_magnitude =
                NET_SPRING_CONSTANT * stretch + NET_SPRING_DAMPING * relative_velocity;
            let force = direction * force_magnitude;

            if !self.anchored[spring.a] {
                self.points[spring.a].apply_central_force(force);
            }
            if !self.anchored[spring.b] {
                self.points[spring.b].apply_central_force(-force);
            }
        }
    }

    /// Advances the net's own internal physics by `dt` (split into
    /// `NET_SUBSTEPS` sub-steps — see this module's own doc comment) and
    /// resolves every one of `bodies`' contact against every free point it
    /// currently overlaps (`RB-PHYSICS-001-FR-038`: the ball and every car,
    /// via `PhysicsWorld::step`'s own call site — a single-element slice for
    /// the ball alone works identically to how this function behaved before
    /// this requirement, since a slice of length 1 is resolved exactly the
    /// same way). Each sub-step: accumulate spring forces, apply gravity
    /// (a real net does sag a little under its own weight) and damping,
    /// integrate every free point's velocity (mirroring
    /// `PhysicsWorld::apply_forces_and_integrate_velocities`'s own
    /// gravity-damping-integrate sequence), resolve every overlapping
    /// body-vs-point contact this sub-step together (see below), then
    /// integrate every free point's transform. An anchored point never
    /// accumulates force, never integrates, and is skipped by every one of
    /// these phases — its position is simply whatever `rectangular_grid`
    /// built it at, forever.
    ///
    /// Since `RB-PHYSICS-001-FR-050`, every body-vs-point contact detected
    /// this sub-step is resolved together as one combined multi-body solve
    /// (`solver::resolve_dynamic_manifolds`), not as a sequence of fully
    /// independent, sequentially-applied `solver::resolve_contacts_between`
    /// calls the way this function used to. A ball or car pressed into the
    /// net commonly overlaps two or more free points at once (`NET_POINT_RADIUS`'s
    /// own generous "coverage radius" sizing all but guarantees it near the
    /// net's own center) — exactly `RB-PHYSICS-001-FR-030`'s "a shared body
    /// touched by 2+ others in the same step" scenario, which that
    /// requirement already proved independent-pairwise resolution
    /// under-converges for. This module's own doc comment used to wave that
    /// off as irrelevant here because a net point's own mass is "tiny
    /// enough" relative to a real ball or car — an untested claim this
    /// requirement's own investigation found false in practice
    /// (`NET_POINT_MASS = 0.5` is only half a typical ball's own mass of
    /// `1.0`, not a lopsided ratio). Worse than under-convergence, the old
    /// per-point sequential loop was genuinely *order-dependent*: for a
    /// perfectly left-right-symmetric impact straddling two points, which
    /// point happened to come first in `self.points`' own iteration order
    /// decided which way the ball ended up deflected sideways — a purely
    /// arbitrary artifact with no physical basis, not merely a slower
    /// convergence to the same answer (see
    /// `sequential_net_point_resolution_is_order_dependent_but_the_combined_solve_is_not`,
    /// which pins the exact mechanism, and
    /// `a_ball_shot_squarely_into_the_net_stays_close_to_a_straight_line_instead_of_veering_sideways`,
    /// which proves it at this function's own public level). Bodies in
    /// `bodies` and every free point are combined into one temporary
    /// `Vec<RigidBody>` for this call only (`RigidBody` is `Copy`, so this
    /// is a plain value copy, not a persistent restructuring of either);
    /// warm-starting isn't part of this fix (a fresh, empty `ContactCache`
    /// map is passed every sub-step, cold-starting every call exactly as
    /// this function always has) — that remains open follow-up work, the
    /// same way `RB-PHYSICS-001-FR-035` scoped it out for
    /// `resolve_contacts`/`resolve_contacts_between` generally.
    pub fn step(&mut self, bodies: &mut [RigidBody], gravity: Vec3, dt: f32) {
        let sub_dt = dt / NET_SUBSTEPS as f32;
        for _ in 0..NET_SUBSTEPS {
            for (i, point) in self.points.iter_mut().enumerate() {
                if !self.anchored[i] {
                    point.clear_forces();
                }
            }
            self.apply_spring_forces();
            for (i, point) in self.points.iter_mut().enumerate() {
                if self.anchored[i] {
                    continue;
                }
                integrate::apply_gravity(point, gravity);
                integrate::apply_damping(point, sub_dt);
                integrate::integrate_velocities(point, sub_dt);
            }

            let num_bodies = bodies.len();
            let mut manifolds = Vec::new();
            for (i, point) in self.points.iter().enumerate() {
                if self.anchored[i] {
                    continue;
                }
                for (body_index, body) in bodies.iter().enumerate() {
                    let contacts = collision::contacts_between(body, point);
                    if !contacts.is_empty() {
                        manifolds.push((body_index, num_bodies + i, contacts));
                    }
                }
            }
            if !manifolds.is_empty() {
                let mut combined: Vec<RigidBody> =
                    Vec::with_capacity(num_bodies + self.points.len());
                combined.extend_from_slice(bodies);
                combined.extend_from_slice(&self.points);
                solver::resolve_dynamic_manifolds(
                    &mut combined,
                    &manifolds,
                    sub_dt,
                    &mut HashMap::new(),
                );
                bodies.copy_from_slice(&combined[..num_bodies]);
                for (i, point) in self.points.iter_mut().enumerate() {
                    if !self.anchored[i] {
                        *point = combined[num_bodies + i];
                    }
                }
            }

            for (i, point) in self.points.iter_mut().enumerate() {
                if self.anchored[i] {
                    continue;
                }
                let (position, orientation) = integrate::integrate_transform(
                    point.position,
                    point.orientation,
                    point.linear_velocity,
                    point.angular_velocity,
                    sub_dt,
                );
                point.position = position;
                point.orientation = orientation;
                point.update_inertia_tensor();
            }
        }
    }
}

/// Measures the rest length directly from the two points' own current
/// (just-computed, still-undeformed) positions and records the spring.
fn push_spring(springs: &mut Vec<Spring>, points: &[RigidBody], a: usize, b: usize) {
    let rest_length = (points[b].position - points[a].position).length();
    springs.push(Spring { a, b, rest_length });
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    /// `RB-PHYSICS-001-FR-050`'s own root-cause proof: a ball straddling two
    /// net-point-like bodies, exactly left-right symmetric, so the true
    /// answer (by symmetry) has zero net sideways velocity for the ball.
    /// Sequentially resolving each pair fully independently — the pre-FR-050
    /// shape of `NetMesh::step`'s own contact loop — instead leaves the ball
    /// with a *nonzero* sideways velocity whose sign flips depending purely
    /// on which point happened to be resolved first, a purely arbitrary
    /// artifact with no physical basis: order A (`p1` then `p2`) and order B
    /// (`p2` then `p1`) are mirror images of each other, neither matching
    /// the true answer. `solver::resolve_dynamic_manifolds`'s combined
    /// solve — sharing one accumulator across both contacts instead of
    /// fully resolving and applying one before the other's setup even reads
    /// the ball's velocity — lands close to the true symmetric answer
    /// instead, and (being order-independent by construction, since both
    /// manifolds read the same starting accumulator) can't exhibit this
    /// left/right bias at all. Uses `NET_POINT_MASS`/`NET_POINT_RADIUS`/
    /// `NET_RESTITUTION`/`NET_FRICTION` directly so this isn't just a
    /// generic claim about *some* mass ratio (already pinned in general by
    /// `solver::tests::resolve_dynamic_manifolds_keeps_more_of_every_bodys_contact_than_resolving_pairs_independently`)
    /// but a concrete confirmation that this module's own specific
    /// constants — previously assumed "tiny enough" to not matter — do in
    /// fact exhibit the gap.
    #[test]
    fn sequential_net_point_resolution_is_order_dependent_but_the_combined_solve_is_not() {
        let make_scene = || {
            let mut ball = RigidBody::sphere(93.15, 1.0, Vec3::new(0.0, 0.0, 0.0));
            ball.linear_velocity = Vec3::new(0.0, 500.0, 0.0);
            let mut p1 = RigidBody::sphere(
                NET_POINT_RADIUS,
                NET_POINT_MASS,
                Vec3::new(-100.0, 150.0, 0.0),
            );
            p1.restitution = NET_RESTITUTION;
            p1.friction = NET_FRICTION;
            let mut p2 = RigidBody::sphere(
                NET_POINT_RADIUS,
                NET_POINT_MASS,
                Vec3::new(100.0, 150.0, 0.0),
            );
            p2.restitution = NET_RESTITUTION;
            p2.friction = NET_FRICTION;
            (ball, p1, p2)
        };
        let dt = 1.0 / 720.0;

        let (mut ball_a, mut p1_a, mut p2_a) = make_scene();
        let c1 = collision::contacts_between(&ball_a, &p1_a);
        solver::resolve_contacts_between(&mut ball_a, &mut p1_a, &c1, dt);
        let c2 = collision::contacts_between(&ball_a, &p2_a);
        solver::resolve_contacts_between(&mut ball_a, &mut p2_a, &c2, dt);
        let order_a_vx = ball_a.linear_velocity.x;

        let (mut ball_b, mut p1_b, mut p2_b) = make_scene();
        let c2b = collision::contacts_between(&ball_b, &p2_b);
        solver::resolve_contacts_between(&mut ball_b, &mut p2_b, &c2b, dt);
        let c1b = collision::contacts_between(&ball_b, &p1_b);
        solver::resolve_contacts_between(&mut ball_b, &mut p1_b, &c1b, dt);
        let order_b_vx = ball_b.linear_velocity.x;

        let (ball_c, p1_c, p2_c) = make_scene();
        let mut bodies = vec![ball_c, p1_c, p2_c];
        let manifolds = vec![
            (
                0usize,
                1usize,
                collision::contacts_between(&bodies[0], &bodies[1]),
            ),
            (
                0usize,
                2usize,
                collision::contacts_between(&bodies[0], &bodies[2]),
            ),
        ];
        solver::resolve_dynamic_manifolds(&mut bodies, &manifolds, dt, &mut HashMap::new());
        let combined_vx = bodies[0].linear_velocity.x;

        assert!(
            (order_a_vx - order_b_vx).abs() > 10.0,
            "expected resolving the two symmetric points sequentially, in opposite orders, to \
             leave the ball with measurably different (mirror-image) sideways velocities, got \
             order_a_vx={order_a_vx}, order_b_vx={order_b_vx}"
        );
        assert!(
            combined_vx.abs() < order_a_vx.abs().min(order_b_vx.abs()) * 0.5,
            "expected the combined solve to leave the ball's sideways velocity much closer to \
             the true symmetric answer of zero than either sequential order, got \
             combined_vx={combined_vx}, order_a_vx={order_a_vx}, order_b_vx={order_b_vx}"
        );
    }

    /// `RB-PHYSICS-001-FR-050`'s own proof at `NetMesh::step`'s public
    /// level: a ball fired squarely at the net's own center, exactly
    /// straddling two adjacent free interior points (the net built with an
    /// even column count so `x = 0` falls precisely between two columns,
    /// each close enough for `NET_POINT_RADIUS`'s own coverage to reach a
    /// ball at `x = 0`), should stay close to a straight line — no physical
    /// asymmetry exists in this setup to deflect it sideways. Measured
    /// directly: the pre-fix sequential per-point loop left this exact
    /// scenario's ball at a residual sideways speed of ~0.25 units/s (out of
    /// a 2000 units/s impact) after a full second of `step` calls — small
    /// in absolute terms (many small `NET_SUBSTEPS`-sized sub-steps each get
    /// a chance to partially self-correct the previous one's bias via
    /// freshly re-detected contacts, unlike
    /// `sequential_net_point_resolution_is_order_dependent_but_the_combined_solve_is_not`'s
    /// own single-shot, no-self-correction proof of the underlying
    /// mechanism), but not zero, and entirely an artifact of `self.points`'
    /// own construction-time iteration order — physically, this setup has
    /// no way to prefer either side. `solver::resolve_dynamic_manifolds`'s
    /// combined solve reduces that residual roughly 15-fold, to ~0.016
    /// units/s.
    #[test]
    fn a_ball_shot_squarely_into_the_net_stays_close_to_a_straight_line_instead_of_veering_sideways(
    ) {
        let net_center = Vec3::new(0.0, 5000.0, 300.0);
        let mut net = NetMesh::rectangular_grid(
            net_center,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            900.0,
            600.0,
            6,
            3,
        );
        let ball_speed = 2000.0;
        let start = net_center - Vec3::new(0.0, 800.0, 0.0);
        let mut ball = RigidBody::sphere(93.15, 1.0, start);
        ball.linear_velocity = Vec3::new(0.0, ball_speed, 0.0);
        let gravity = Vec3::ZERO; // isolate the net's own catching effect from gravity's fall

        let dt = 1.0 / 120.0;
        for _ in 0..(1.0 / dt) as u32 {
            net.step(std::slice::from_mut(&mut ball), gravity, dt);
            let (position, orientation) = integrate::integrate_transform(
                ball.position,
                ball.orientation,
                ball.linear_velocity,
                ball.angular_velocity,
                dt,
            );
            ball.position = position;
            ball.orientation = orientation;
        }

        assert!(
            ball.linear_velocity.x.abs() < 0.05,
            "expected a squarely-centered, left-right-symmetric net impact to leave the ball's \
             own sideways velocity near zero (the pre-fix sequential loop measured ~0.25 here), \
             got vx={}",
            ball.linear_velocity.x
        );
    }

    fn flat_net(cols: usize, rows: usize) -> NetMesh {
        NetMesh::rectangular_grid(
            Vec3::new(0.0, 100.0, 300.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            900.0,
            600.0,
            cols,
            rows,
        )
    }

    #[test]
    fn perimeter_points_are_anchored_and_interior_points_are_not() {
        let net = flat_net(5, 4);
        for row in 0..4 {
            for col in 0..5 {
                let i = row * 5 + col;
                let expected = row == 0 || row == 3 || col == 0 || col == 4;
                assert_eq!(
                    net.is_anchored(i),
                    expected,
                    "row={row} col={col} anchored mismatch"
                );
            }
        }
    }

    #[test]
    fn springs_start_at_zero_stretch() {
        // Every spring's rest_length is measured from the flat grid itself,
        // so the mesh starts in exact equilibrium — no force should be
        // generated before anything moves.
        let mut net = flat_net(5, 4);
        let before: Vec<Vec3> = net.points.iter().map(|p| p.position).collect();
        net.apply_spring_forces();
        for (i, point) in net.points.iter().enumerate() {
            assert_eq!(
                point.total_force(),
                Vec3::ZERO,
                "point {i} at {:?} accumulated nonzero force at rest",
                before[i]
            );
        }
    }

    #[test]
    fn anchored_points_never_move_under_gravity_alone() {
        let mut net = flat_net(5, 4);
        let anchored_before: Vec<Vec3> = (0..net.points.len())
            .filter(|&i| net.is_anchored(i))
            .map(|i| net.points[i].position)
            .collect();

        let mut ball = RigidBody::sphere(93.15, 1.0, Vec3::new(5000.0, 5000.0, 5000.0));
        let gravity = Vec3::new(0.0, 0.0, -650.0);
        for _ in 0..120 {
            net.step(std::slice::from_mut(&mut ball), gravity, 1.0 / 60.0);
        }

        let anchored_after: Vec<Vec3> = (0..net.points.len())
            .filter(|&i| net.is_anchored(i))
            .map(|i| net.points[i].position)
            .collect();
        assert_eq!(
            anchored_before, anchored_after,
            "expected every anchored point to stay exactly where it started"
        );
    }

    #[test]
    fn an_undisturbed_net_settles_instead_of_oscillating_forever() {
        // Gravity sags the interior points slightly; damping should let
        // that settle to a low residual velocity rather than jiggling
        // indefinitely — the mass-spring analog of
        // `world::tests::resting_ball_stays_at_rest`.
        let mut net = flat_net(5, 4);
        let mut ball = RigidBody::sphere(93.15, 1.0, Vec3::new(5000.0, 5000.0, 5000.0));
        let gravity = Vec3::new(0.0, 0.0, -650.0);
        for _ in 0..600 {
            net.step(std::slice::from_mut(&mut ball), gravity, 1.0 / 60.0);
        }

        let max_speed = net
            .points
            .iter()
            .enumerate()
            .filter(|(i, _)| !net.is_anchored(*i))
            .map(|(_, p)| p.linear_velocity.length())
            .fold(0.0f32, f32::max);
        assert!(
            max_speed < 5.0,
            "expected the undisturbed net to settle to a low residual speed, got {max_speed}"
        );
    }

    #[test]
    fn a_ball_shot_into_the_net_is_measurably_slowed_compared_to_free_flight() {
        // The real "catching" proof: a ball fired at the net's own center
        // loses a large fraction of its speed on impact, unlike a ball
        // fired through open space with no net at all. `NetMesh::step`
        // only mutates the ball's *velocity* (matching `PhysicsWorld`'s own
        // staged pipeline, where a body's transform integrates separately,
        // once, after every contact this step has been resolved) — so this
        // test integrates the ball's position itself between calls, the
        // same way `PhysicsWorld::step` does via
        // `integrate_transform_and_refresh_inertia` right after its own
        // net-step loop.
        let net_center = Vec3::new(0.0, 5000.0, 300.0);
        let ball_speed = 2000.0;
        let start = net_center - Vec3::new(0.0, 800.0, 0.0);

        let mut net = NetMesh::rectangular_grid(
            net_center,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            900.0,
            600.0,
            5,
            5,
        );
        let mut ball = RigidBody::sphere(93.15, 1.0, start);
        ball.linear_velocity = Vec3::new(0.0, ball_speed, 0.0);
        let gravity = Vec3::ZERO; // isolate the net's own catching effect from gravity's fall

        let dt = 1.0 / 120.0;
        for _ in 0..(1.0 / dt) as u32 {
            net.step(std::slice::from_mut(&mut ball), gravity, dt);
            let (position, orientation) = integrate::integrate_transform(
                ball.position,
                ball.orientation,
                ball.linear_velocity,
                ball.angular_velocity,
                dt,
            );
            ball.position = position;
            ball.orientation = orientation;
        }

        assert!(
            ball.linear_velocity.y.abs() < ball_speed * 0.5,
            "expected the net to have caught the ball, losing at least half its speed, \
             start speed={ball_speed}, end vy={}",
            ball.linear_velocity.y
        );
    }

    #[test]
    fn a_car_shot_into_the_net_is_measurably_slowed_compared_to_free_flight() {
        // RB-PHYSICS-001-FR-038: the same proof as
        // `a_ball_shot_into_the_net_is_measurably_slowed_compared_to_free_flight`,
        // for a car (box) instead of a sphere — `collision::contacts_between`
        // already dispatches to `sphere_vs_box` for a box-vs-sphere pair, so
        // no new collision code was needed, only `step`'s own `&mut
        // [RigidBody]` slice replacing its old single-`&mut RigidBody`
        // parameter.
        let net_center = Vec3::new(0.0, 5000.0, 300.0);
        let car_speed = 2000.0;
        let start = net_center - Vec3::new(0.0, 800.0, 0.0);

        let mut net = NetMesh::rectangular_grid(
            net_center,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            900.0,
            600.0,
            5,
            5,
        );
        let mut car = RigidBody::car_box(Vec3::new(60.0, 40.0, 20.0), 1.0, start);
        car.linear_velocity = Vec3::new(0.0, car_speed, 0.0);
        let gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(1.0 / dt) as u32 {
            net.step(std::slice::from_mut(&mut car), gravity, dt);
            let (position, orientation) = integrate::integrate_transform(
                car.position,
                car.orientation,
                car.linear_velocity,
                car.angular_velocity,
                dt,
            );
            car.position = position;
            car.orientation = orientation;
        }

        assert!(
            car.linear_velocity.y.abs() < car_speed * 0.5,
            "expected the net to have caught the car, losing at least half its speed, \
             start speed={car_speed}, end vy={}",
            car.linear_velocity.y
        );
    }

    #[test]
    fn a_ball_and_a_car_are_both_resolved_against_the_same_net_step() {
        // Proves `step`'s own `bodies` slice genuinely resolves every body
        // against the net within one call, not just the first element — a
        // claim this port's earlier single-body-only `step` couldn't even
        // represent, let alone test. The two bodies are offset far enough
        // apart along the net's own width that they never touch each other,
        // isolating each one's own net contact.
        let net_center = Vec3::new(0.0, 5000.0, 300.0);
        let speed = 2000.0;
        let start = net_center - Vec3::new(0.0, 800.0, 0.0);

        let mut net = NetMesh::rectangular_grid(
            net_center,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            900.0,
            600.0,
            5,
            5,
        );
        let mut ball = RigidBody::sphere(93.15, 1.0, start + Vec3::new(-400.0, 0.0, 0.0));
        ball.linear_velocity = Vec3::new(0.0, speed, 0.0);
        let mut car = RigidBody::car_box(
            Vec3::new(60.0, 40.0, 20.0),
            1.0,
            start + Vec3::new(400.0, 0.0, 0.0),
        );
        car.linear_velocity = Vec3::new(0.0, speed, 0.0);
        let gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(1.0 / dt) as u32 {
            let mut bodies = [ball, car];
            net.step(&mut bodies, gravity, dt);
            ball = bodies[0];
            car = bodies[1];
            for body in [&mut ball, &mut car] {
                let (position, orientation) = integrate::integrate_transform(
                    body.position,
                    body.orientation,
                    body.linear_velocity,
                    body.angular_velocity,
                    dt,
                );
                body.position = position;
                body.orientation = orientation;
            }
        }

        assert!(
            ball.linear_velocity.y.abs() < speed * 0.5,
            "expected the net to have caught the ball, got vy={}",
            ball.linear_velocity.y
        );
        assert!(
            car.linear_velocity.y.abs() < speed * 0.5,
            "expected the net to have caught the car, got vy={}",
            car.linear_velocity.y
        );
    }
}
