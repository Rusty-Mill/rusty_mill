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
//! typical single-step `dt` values, e.g. 1/60s) and resolves the ball's
//! contact against every free point it currently overlaps via
//! `collision::sphere_vs_sphere`/`solver::resolve_contacts_between` — the
//! exact same two-dynamic-body sequential-impulse path every other
//! dynamic-vs-dynamic contact in this crate already uses (ball-vs-car,
//! car-vs-car), not a special-cased shortcut. This is why a net point is a
//! real (if artificially tiny and light) `RigidBody` rather than a plain
//! `Vec3` position/velocity pair: it lets this module add zero new solver
//! code, only the spring-force accumulation Hooke's law itself needs and
//! the sphere-vs-sphere contact test this crate had no prior caller for
//! (see `collision::sphere_vs_sphere`'s own doc comment).
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
//! Explicitly out of scope (tracked in `RB-PHYSICS-001`, not silently
//! dropped): a car's own contact against a net — a car still passes
//! straight through a `NetMesh`'s spatial footprint untouched, exactly as
//! it did before this module existed, and continues to be stopped by the
//! pre-existing rigid `StaticBoundedWall`/back-of-net `StaticPlane`
//! machinery `RB-PHYSICS-001-FR-029` already built (a `NetMesh` panel sits
//! well in front of that machinery — see `arena::NET_DEPTH` — so it's
//! always the ball's real backstop regardless of how the net itself
//! behaves); manifold richness beyond one contact per overlapping point
//! (no clipped-face-style manifold the way `box_vs_box` builds one); a full
//! 3D "sock" shape billowing backward from the goal mouth (this models a
//! single flat rest-shape panel instead, which still deforms backward
//! dynamically under a real ball impact via its own springs — just not a
//! pre-shaped pocket); and bending stiffness (only structural + shear
//! springs, no springs resisting the mesh folding along a diagonal) — none
//! of this crate's existing cloth-adjacent shapes need rendering-quality
//! draping, only enough structure to catch a ball believably.

use crate::body::RigidBody;
use crate::collision;
use crate::integrate;
use crate::solver;
use rb_domain::Vec3;

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
    /// resolves `ball`'s contact against every free point it currently
    /// overlaps. Each sub-step: accumulate spring forces, apply gravity
    /// (a real net does sag a little under its own weight) and damping,
    /// integrate every free point's velocity (mirroring
    /// `PhysicsWorld::apply_forces_and_integrate_velocities`'s own
    /// gravity-damping-integrate sequence), resolve `ball` against every
    /// free point within contact range (via
    /// `collision::contacts_between`/`solver::resolve_contacts_between`,
    /// the exact two-dynamic-body path every other dynamic-vs-dynamic
    /// contact in this crate already uses — this mutates `ball`'s own
    /// velocity too, progressively across sub-steps, not just the net's),
    /// then integrate every free point's transform. An anchored point never
    /// accumulates force, never integrates, and is skipped by every one of
    /// these phases — its position is simply whatever `rectangular_grid`
    /// built it at, forever.
    pub fn step(&mut self, ball: &mut RigidBody, gravity: Vec3, dt: f32) {
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
            for (i, point) in self.points.iter_mut().enumerate() {
                if self.anchored[i] {
                    continue;
                }
                let contacts = collision::contacts_between(ball, point);
                if !contacts.is_empty() {
                    solver::resolve_contacts_between(ball, point, &contacts, sub_dt);
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

        let mut ball = RigidBody::sphere(92.75, 1.0, Vec3::new(5000.0, 5000.0, 5000.0));
        let gravity = Vec3::new(0.0, 0.0, -650.0);
        for _ in 0..120 {
            net.step(&mut ball, gravity, 1.0 / 60.0);
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
        let mut ball = RigidBody::sphere(92.75, 1.0, Vec3::new(5000.0, 5000.0, 5000.0));
        let gravity = Vec3::new(0.0, 0.0, -650.0);
        for _ in 0..600 {
            net.step(&mut ball, gravity, 1.0 / 60.0);
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
        let mut ball = RigidBody::sphere(92.75, 1.0, start);
        ball.linear_velocity = Vec3::new(0.0, ball_speed, 0.0);
        let gravity = Vec3::ZERO; // isolate the net's own catching effect from gravity's fall

        let dt = 1.0 / 120.0;
        for _ in 0..(1.0 / dt) as u32 {
            net.step(&mut ball, gravity, dt);
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
}
