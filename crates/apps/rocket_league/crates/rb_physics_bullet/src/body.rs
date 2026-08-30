//! Rigid body types: `RigidBody` (dynamic, either sphere- or box-shaped)
//! and `StaticPlane` (immovable). A single `RigidBody` type serves both
//! the ball (sphere) and car (box) — matching Bullet's own architecture,
//! where one `btRigidBody` class carries a polymorphic `btCollisionShape`
//! rather than a separate rigid-body type per shape (see `Shape`). This
//! only became necessary once a second shape (the car box,
//! `RB-PHYSICS-001-FR-004`) existed: v0's sphere-only scope got away with
//! a scalar inverse inertia because a sphere's inertia is isotropic; a
//! box's isn't, so `RigidBody` carries a general 3x3 inverse inertia
//! tensor (`Mat3`) instead — see `update_inertia_tensor`.
//!
//! World convention: +Z is up, matching Unreal Engine (which Rocket League
//! runs on) rather than the +Y-up convention common in some other engines.

use crate::mat3::Mat3;
use rb_domain::{Quat, Vec3};

/// The local-frame collision geometry a `RigidBody` carries — just enough
/// to compute a local inertia tensor (`local_inertia`) and narrow-phase
/// contacts (see `collision.rs`). Sphere and box are the only two shapes
/// any recorded Rocket League match needs (ball, car).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Sphere {
        radius: f32,
    },
    /// `half_extents` are the box's half-widths along its own local X/Y/Z
    /// axes — matching `btBoxShape`'s convention (its constructor also
    /// takes half-extents, not full dimensions).
    Box {
        half_extents: Vec3,
    },
}

impl Shape {
    /// The diagonal of the local-frame inertia tensor (off-diagonal terms
    /// are zero for both shapes about their own center of mass, by
    /// symmetry) — port of `btSphereShape::calculateLocalInertia` /
    /// `btBoxShape::calculateLocalInertia`.
    fn local_inertia(&self, mass: f32) -> Vec3 {
        match *self {
            Shape::Sphere { radius } => {
                // I = 2/5 m r^2 for a solid sphere, same for all three axes.
                let i = 0.4 * mass * radius * radius;
                Vec3::new(i, i, i)
            }
            Shape::Box { half_extents } => {
                // I = m/12 * (full_dimension_a^2 + full_dimension_b^2) per
                // axis, for a solid box — btBoxShape uses `2 * halfExtents`
                // as the full dimensions.
                let lx2 = (2.0 * half_extents.x).powi(2);
                let ly2 = (2.0 * half_extents.y).powi(2);
                let lz2 = (2.0 * half_extents.z).powi(2);
                Vec3::new(
                    mass / 12.0 * (ly2 + lz2),
                    mass / 12.0 * (lx2 + lz2),
                    mass / 12.0 * (lx2 + ly2),
                )
            }
        }
    }
}

/// A dynamic rigid body: either a sphere (the ball) or a box (a car).
/// Mirrors the subset of `bullet3/src/BulletDynamics/Dynamics/btRigidBody.h`'s
/// fields this crate's integration and solver code actually needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidBody {
    pub shape: Shape,
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,

    /// 0 disables damping — matches Bullet's default `m_linearDamping = 0`.
    pub linear_damping: f32,
    pub angular_damping: f32,

    /// Bullet's `m_restitution`/`m_friction`, combined at contact time via
    /// `btManifoldPoint::m_combinedRestitution`/`m_combinedFriction`
    /// (default combine mode: average — see `solver::combine_restitution`).
    pub restitution: f32,
    pub friction: f32,

    inv_mass: f32,
    /// Diagonal principal moments' inverses, in the body's own local
    /// frame — constant for the body's lifetime (shape/mass don't change).
    inv_inertia_local: Vec3,
    /// `R * diag(inv_inertia_local) * R^T` for the body's current
    /// orientation — recomputed by `update_inertia_tensor` whenever
    /// `orientation` changes (matches Bullet's `updateInertiaTensor`,
    /// called once per step after the transform integrates).
    inv_inertia_world: Mat3,

    total_force: Vec3,
    total_torque: Vec3,
}

impl RigidBody {
    /// `mass` must be positive — a zero/negative mass body isn't a
    /// meaningful dynamic body in this scope (Bullet handles mass == 0 as
    /// "static", but that's what `StaticPlane` is for here). Panics if
    /// `shape`'s own dimensions (radius, half-extents) aren't positive.
    pub fn new(shape: Shape, mass: f32, position: Vec3) -> RigidBody {
        assert!(
            mass > 0.0,
            "RigidBody mass must be positive; use a static body for immovable objects"
        );
        match shape {
            Shape::Sphere { radius } => assert!(radius > 0.0, "Sphere radius must be positive"),
            Shape::Box { half_extents } => assert!(
                half_extents.x > 0.0 && half_extents.y > 0.0 && half_extents.z > 0.0,
                "Box half_extents must all be positive"
            ),
        }

        let local_inertia = shape.local_inertia(mass);
        let inv_inertia_local = Vec3::new(
            1.0 / local_inertia.x,
            1.0 / local_inertia.y,
            1.0 / local_inertia.z,
        );

        let mut body = RigidBody {
            shape,
            position,
            orientation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            linear_damping: 0.0,
            angular_damping: 0.0,
            restitution: 0.5,
            friction: 0.5,
            inv_mass: 1.0 / mass,
            inv_inertia_local,
            inv_inertia_world: Mat3::IDENTITY,
            total_force: Vec3::ZERO,
            total_torque: Vec3::ZERO,
        };
        body.update_inertia_tensor();
        body
    }

    pub fn sphere(radius: f32, mass: f32, position: Vec3) -> RigidBody {
        RigidBody::new(Shape::Sphere { radius }, mass, position)
    }

    pub fn car_box(half_extents: Vec3, mass: f32, position: Vec3) -> RigidBody {
        RigidBody::new(Shape::Box { half_extents }, mass, position)
    }

    /// Recomputes `inv_inertia_world` from the body's current `orientation`
    /// — port of `btRigidBody::updateInertiaTensor`
    /// (`m_invInertiaTensorWorld = basis.scaled(invInertiaLocal) * basis.transpose()`).
    /// Must be called after `orientation` changes; `PhysicsWorld::step`
    /// does this once per step, right after integrating the transform.
    pub fn update_inertia_tensor(&mut self) {
        let basis = Mat3::from_quat(&self.orientation);
        self.inv_inertia_world = basis
            .scaled_columns(&self.inv_inertia_local)
            .mul_mat3(&basis.transpose());
    }

    pub fn inv_mass(&self) -> f32 {
        self.inv_mass
    }

    pub fn inv_inertia_world(&self) -> Mat3 {
        self.inv_inertia_world
    }

    pub fn mass(&self) -> f32 {
        1.0 / self.inv_mass
    }

    /// Velocity of the point on the body at world-space offset `rel_pos`
    /// from its center — matches `btRigidBody::getVelocityInLocalPoint`.
    pub fn velocity_at_point(&self, rel_pos: &Vec3) -> Vec3 {
        self.linear_velocity + self.angular_velocity.cross(rel_pos)
    }

    pub fn apply_central_force(&mut self, force: Vec3) {
        self.total_force += force;
    }

    pub fn apply_torque(&mut self, torque: Vec3) {
        self.total_torque += torque;
    }

    pub fn apply_impulse(&mut self, impulse: Vec3, rel_pos: Vec3) {
        self.linear_velocity += impulse * self.inv_mass;
        self.angular_velocity += self.inv_inertia_world.mul_vec3(&rel_pos.cross(&impulse));
    }

    pub fn clear_forces(&mut self) {
        self.total_force = Vec3::ZERO;
        self.total_torque = Vec3::ZERO;
    }

    pub fn total_force(&self) -> Vec3 {
        self.total_force
    }

    pub fn total_torque(&self) -> Vec3 {
        self.total_torque
    }
}

/// An immovable plane, defined as `{ p : dot(normal, p) == offset }`.
/// `normal` must be a unit vector (the ground plane is
/// `StaticPlane { normal: Vec3::new(0.0, 0.0, 1.0), offset: 0.0 }`).
///
/// Static bodies don't get their own `btRigidBody` in this port — Bullet
/// represents them as a zero-mass rigid body, but a plane has no
/// orientation/velocity state worth carrying, so a plain struct is the
/// smaller, equally-faithful representation for the one static shape this
/// crate needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticPlane {
    pub normal: Vec3,
    pub offset: f32,
    pub restitution: f32,
    pub friction: f32,
}

impl StaticPlane {
    pub fn new(normal: Vec3, offset: f32) -> StaticPlane {
        StaticPlane {
            normal,
            offset,
            restitution: 0.5,
            friction: 0.5,
        }
    }

    /// Signed distance from `point` to the plane, positive on the side
    /// `normal` points toward.
    pub fn signed_distance(&self, point: &Vec3) -> f32 {
        self.normal.dot(point) - self.offset
    }
}

/// An immovable partial-cylinder fillet connecting two flat planes (a wall
/// and the floor, a wall and the ceiling, or — since
/// `RB-PHYSICS-001-FR-022` — two walls meeting at a corner that isn't a
/// right angle) — `RB-PHYSICS-001-FR-020`, Rocket League's real curved
/// wall-to-floor and wall-to-ceiling transitions. Like `StaticPlane`,
/// infinite along its own axis (`axis_direction`) — this crate doesn't
/// model a finite wall length any more for the curve than it already does
/// for the flat walls themselves.
///
/// The playable side is the *inside* of the partial cylinder (like riding
/// the concave face of a skateboard quarter-pipe, which is exactly what
/// this shape is named after, even though — since FR-022 — its own sector
/// isn't always literally a quarter-circle) — a point is only governed by
/// this fillet at all when its direction from `axis_point` (projected
/// perpendicular to `axis_direction`) falls within the sector from
/// `sector_start` to `sector_end` (whatever angle, always at most 180
/// degrees, those two vectors happen to subtend — see `between_planes`);
/// outside that sector, whichever flat plane the fillet bridges takes over
/// instead (this shape doesn't know or care about that — see
/// `PhysicsWorld::step`, which resolves the flat planes and this fillet as
/// independent, additive contact sources, same as it already does for the
/// ground and every wall).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticQuarterPipe {
    pub axis_point: Vec3,
    /// Unit vector; the fillet is infinite along this direction. Its sign
    /// matters (see `sector_start`/`sector_end`), not just its line: this
    /// is a genuine 3D direction, not merely "the axis's orientation."
    pub axis_direction: Vec3,
    pub radius: f32,
    /// Unit vector, perpendicular to `axis_direction`: the direction from
    /// `axis_point` toward the fillet's tangent point on the first flat
    /// plane it bridges.
    pub sector_start: Vec3,
    /// Unit vector, perpendicular to `axis_direction`: the direction from
    /// `axis_point` toward the fillet's tangent point on the second flat
    /// plane it bridges. Together with `sector_start` and `axis_direction`,
    /// defines the sector: sweeping from `sector_start` toward `sector_end`
    /// via a *positive* (right-hand-rule) rotation about `axis_direction`
    /// must cover the fillet's own (at most 180-degree) angle — see
    /// `between_planes`, which guarantees this by construction.
    pub sector_end: Vec3,
    pub restitution: f32,
    pub friction: f32,
}

impl StaticQuarterPipe {
    pub fn new(
        axis_point: Vec3,
        axis_direction: Vec3,
        radius: f32,
        sector_start: Vec3,
        sector_end: Vec3,
    ) -> StaticQuarterPipe {
        StaticQuarterPipe {
            axis_point,
            axis_direction,
            radius,
            sector_start,
            sector_end,
            restitution: 0.5,
            friction: 0.5,
        }
    }

    /// Derives a fillet of the given `radius` connecting two flat
    /// `StaticPlane`s meeting along a line — e.g. the floor and a side
    /// wall, or (since `RB-PHYSICS-001-FR-022`) a diagonal corner wall and
    /// its neighboring side wall, which meet at a shallower angle than a
    /// right angle — given a direction along that shared line
    /// (`axis_direction`, perpendicular to both planes' normals; for an
    /// axis-aligned arena wall meeting the floor/ceiling, this is simply
    /// "along the wall," e.g. `(0, 1, 0)` for a wall running along Y).
    ///
    /// Works for *any* two non-parallel planes, not just perpendicular
    /// ones: the fillet's own sector angle always comes out to exactly the
    /// angle between `plane_a.normal` and `plane_b.normal` (which is always
    /// in `[0, 180]` degrees by construction, so there's never an ambiguous
    /// "long way around" to worry about) — a right angle for two
    /// perpendicular planes (every cardinal or diagonal-corner wall's own
    /// floor/ceiling seam, `RB-PHYSICS-001-FR-020`/`FR-021`), or some other
    /// angle for two walls meeting at an actual corner (`FR-022`).
    /// `axis_direction`'s own sign doesn't matter — either of the two
    /// opposite directions along the shared line works — this constructor
    /// detects and corrects it internally so `sector_start`/`sector_end`
    /// always sweep the fillet's own short arc, never the reflex one.
    ///
    /// The axis point sits `radius` units inward from *both* planes along
    /// their own normals (so the fillet's surface is tangent to each plane
    /// exactly `radius` units from where they'd otherwise meet at a sharp
    /// edge) — found by solving two equations, `dot(plane_a.normal, axis)`
    /// equals `plane_a.offset` plus `radius`, and likewise for `plane_b`,
    /// as a 2x2 linear system, expressing `axis` in the (generally
    /// non-orthogonal) basis formed by the two normals themselves. This
    /// reduces to the simpler "just add the two scaled normals together"
    /// shortcut exactly when the normals are perpendicular, but that
    /// shortcut silently gives the wrong point otherwise, which is why this
    /// solves the system directly instead of assuming orthogonality.
    /// `sector_start`/`sector_end` are simply the negation of each plane's
    /// own normal (the direction from the axis back toward that plane's
    /// tangent point) — true regardless of the angle between the planes.
    pub fn between_planes(
        plane_a: &StaticPlane,
        plane_b: &StaticPlane,
        radius: f32,
        axis_direction: Vec3,
    ) -> StaticQuarterPipe {
        let target_a = plane_a.offset + radius;
        let target_b = plane_b.offset + radius;
        let cos_angle = plane_a.normal.dot(&plane_b.normal);
        let denom = 1.0 - cos_angle * cos_angle;
        let coeff_a = (target_a - target_b * cos_angle) / denom;
        let coeff_b = (target_b - target_a * cos_angle) / denom;
        let axis_point = plane_a.normal * coeff_a + plane_b.normal * coeff_b;

        let sector_start = -plane_a.normal;
        let sector_end = -plane_b.normal;
        // Ensure sector_start -> sector_end sweeps the short arc in the
        // *positive* (right-hand-rule) sense about axis_direction, flipping
        // it if the caller happened to pass the opposite of the two
        // directions along the shared line — see the struct's own doc
        // comment on why axis_direction's sign matters for this.
        let axis_direction = if sector_start.cross(&sector_end).dot(&axis_direction) < 0.0 {
            -axis_direction
        } else {
            axis_direction
        };

        StaticQuarterPipe::new(axis_point, axis_direction, radius, sector_start, sector_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_inertia_matches_solid_sphere_formula() {
        let s = RigidBody::sphere(1.0, 2.0, Vec3::ZERO);
        // I = 2/5 * m * r^2 = 0.4 * 2.0 * 1.0 = 0.8
        let v = s.inv_inertia_world().mul_vec3(&Vec3::new(1.0, 0.0, 0.0));
        assert!((v.x - 1.0 / 0.8).abs() < 1e-6);
    }

    #[test]
    fn box_inertia_matches_solid_cuboid_formula() {
        // A 2x4x6 box (half-extents 1x2x3), mass 12: I = m/12*(b^2+c^2) per
        // axis using full dimensions (2,4,6).
        let half_extents = Vec3::new(1.0, 2.0, 3.0);
        let b = RigidBody::car_box(half_extents, 12.0, Vec3::ZERO);
        let expected = Vec3::new(
            12.0 / 12.0 * (16.0 + 36.0), // Ixx = m/12*(ly^2+lz^2) = 1*(16+36)=52
            12.0 / 12.0 * (4.0 + 36.0),  // Iyy = 1*(4+36)=40
            12.0 / 12.0 * (4.0 + 16.0),  // Izz = 1*(4+16)=20
        );
        let inv = b.inv_inertia_world();
        assert!((inv.mul_vec3(&Vec3::new(1.0, 0.0, 0.0)).x - 1.0 / expected.x).abs() < 1e-5);
        assert!((inv.mul_vec3(&Vec3::new(0.0, 1.0, 0.0)).y - 1.0 / expected.y).abs() < 1e-5);
        assert!((inv.mul_vec3(&Vec3::new(0.0, 0.0, 1.0)).z - 1.0 / expected.z).abs() < 1e-5);
    }

    #[test]
    #[should_panic(expected = "mass must be positive")]
    fn zero_mass_sphere_panics() {
        RigidBody::sphere(1.0, 0.0, Vec3::ZERO);
    }

    #[test]
    #[should_panic(expected = "half_extents must all be positive")]
    fn zero_extent_box_panics() {
        RigidBody::car_box(Vec3::new(1.0, 0.0, 1.0), 1.0, Vec3::ZERO);
    }

    #[test]
    fn velocity_at_point_includes_rotational_contribution() {
        let mut s = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        s.angular_velocity = Vec3::new(0.0, 0.0, 1.0); // spinning about +Z
        let rel_pos = Vec3::new(1.0, 0.0, 0.0); // point on +X of the surface
                                                // omega x r = (0,0,1) x (1,0,0) = (0,1,0)
        let v = s.velocity_at_point(&rel_pos);
        assert!((v - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-6);
    }

    #[test]
    fn sphere_inertia_tensor_is_orientation_independent() {
        // A sphere's isotropic inertia shouldn't change under rotation —
        // confirms the Mat3 generalization doesn't alter v0's sphere
        // behavior.
        let mut s = RigidBody::sphere(1.0, 2.0, Vec3::ZERO);
        let before = s.inv_inertia_world().mul_vec3(&Vec3::new(1.0, 2.0, 3.0));
        let half = std::f32::consts::FRAC_PI_4;
        s.orientation = Quat::new(0.0, 0.0, half.sin(), half.cos());
        s.update_inertia_tensor();
        let after = s.inv_inertia_world().mul_vec3(&Vec3::new(1.0, 2.0, 3.0));
        assert!((before - after).length() < 1e-5);
    }

    #[test]
    fn box_inertia_tensor_changes_with_orientation() {
        // Unlike a sphere, a box's anisotropic inertia tensor genuinely
        // depends on orientation — this is exactly why Mat3 (not a scalar)
        // is needed once box bodies exist.
        let mut b = RigidBody::car_box(Vec3::new(1.0, 2.0, 3.0), 12.0, Vec3::ZERO);
        let probe = Vec3::new(1.0, 0.0, 0.0);
        let before = b.inv_inertia_world().mul_vec3(&probe);
        let half = std::f32::consts::FRAC_PI_4;
        b.orientation = Quat::new(0.0, 0.0, half.sin(), half.cos());
        b.update_inertia_tensor();
        let after = b.inv_inertia_world().mul_vec3(&probe);
        assert!(
            (before - after).length() > 1e-3,
            "expected the box's inertia response to change with orientation"
        );
    }

    #[test]
    fn plane_signed_distance_is_zero_on_the_plane() {
        let ground = StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0);
        assert_eq!(ground.signed_distance(&Vec3::new(5.0, -3.0, 0.0)), 0.0);
    }

    #[test]
    fn plane_signed_distance_is_positive_above() {
        let ground = StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0);
        assert_eq!(ground.signed_distance(&Vec3::new(0.0, 0.0, 4.0)), 4.0);
    }

    /// A floor (z=0) meeting a +X side wall at x=100, as if a wall-jump
    /// test wall — used to check `between_planes`' derived geometry against
    /// hand-computed expectations.
    fn floor_and_side_wall() -> (StaticPlane, StaticPlane) {
        let floor = StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0);
        let wall = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -100.0);
        (floor, wall)
    }

    #[test]
    fn quarter_pipe_axis_sits_radius_units_in_from_both_planes() {
        let (floor, wall) = floor_and_side_wall();
        let pipe = StaticQuarterPipe::between_planes(&floor, &wall, 20.0, Vec3::new(0.0, 1.0, 0.0));
        // radius=20: axis at z=20 (in from the floor) and x=100-20=80 (in
        // from the wall at x=100).
        assert!((pipe.axis_point.x - 80.0).abs() < 1e-4);
        assert!((pipe.axis_point.z - 20.0).abs() < 1e-4);
    }

    #[test]
    fn quarter_pipe_sector_vectors_point_toward_each_planes_tangent_point() {
        let (floor, wall) = floor_and_side_wall();
        let pipe = StaticQuarterPipe::between_planes(&floor, &wall, 20.0, Vec3::new(0.0, 1.0, 0.0));
        // sector_start (toward the floor's tangent point) should point
        // down (-Z); sector_end (toward the wall's tangent point) should
        // point toward the wall (+X).
        assert!((pipe.sector_start - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5);
        assert!((pipe.sector_end - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn quarter_pipe_tangent_points_lie_exactly_on_each_plane() {
        let (floor, wall) = floor_and_side_wall();
        let radius = 20.0;
        let pipe =
            StaticQuarterPipe::between_planes(&floor, &wall, radius, Vec3::new(0.0, 1.0, 0.0));
        let floor_tangent = pipe.axis_point + pipe.sector_start * radius;
        let wall_tangent = pipe.axis_point + pipe.sector_end * radius;
        assert!(floor.signed_distance(&floor_tangent).abs() < 1e-4);
        assert!(wall.signed_distance(&wall_tangent).abs() < 1e-4);
    }

    #[test]
    fn quarter_pipe_sector_vectors_are_perpendicular_unit_vectors() {
        let (floor, wall) = floor_and_side_wall();
        let pipe = StaticQuarterPipe::between_planes(&floor, &wall, 20.0, Vec3::new(0.0, 1.0, 0.0));
        assert!((pipe.sector_start.length() - 1.0).abs() < 1e-5);
        assert!((pipe.sector_end.length() - 1.0).abs() < 1e-5);
        assert!(pipe.sector_start.dot(&pipe.sector_end).abs() < 1e-5);
    }

    /// A wall at `x = 0` meeting a second wall through the origin at 45
    /// degrees -- a *non-perpendicular* pair (unlike `floor_and_side_wall`),
    /// the same angle a diagonal corner wall's own vertical edge meets its
    /// neighboring side/back wall at (`RB-PHYSICS-001-FR-022`). Used to
    /// check `between_planes`' generalization to any two non-parallel
    /// planes, not just perpendicular ones.
    fn non_perpendicular_walls() -> (StaticPlane, StaticPlane) {
        let wall = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), 0.0);
        let diagonal = StaticPlane::new(
            Vec3::new(-1.0, -1.0, 0.0) * std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        );
        (wall, diagonal)
    }

    #[test]
    fn between_non_perpendicular_planes_still_sits_the_axis_radius_in_from_both() {
        let (wall, diagonal) = non_perpendicular_walls();
        let radius = 20.0;
        let pipe =
            StaticQuarterPipe::between_planes(&wall, &diagonal, radius, Vec3::new(0.0, 0.0, 1.0));
        assert!((wall.signed_distance(&pipe.axis_point) - radius).abs() < 1e-3);
        assert!((diagonal.signed_distance(&pipe.axis_point) - radius).abs() < 1e-3);
    }

    #[test]
    fn between_non_perpendicular_planes_tangent_points_lie_exactly_on_each_plane() {
        let (wall, diagonal) = non_perpendicular_walls();
        let radius = 20.0;
        let pipe =
            StaticQuarterPipe::between_planes(&wall, &diagonal, radius, Vec3::new(0.0, 0.0, 1.0));
        let wall_tangent = pipe.axis_point + pipe.sector_start * radius;
        let diagonal_tangent = pipe.axis_point + pipe.sector_end * radius;
        assert!(wall.signed_distance(&wall_tangent).abs() < 1e-3);
        assert!(diagonal.signed_distance(&diagonal_tangent).abs() < 1e-3);
    }

    #[test]
    fn between_non_perpendicular_planes_sector_angle_matches_the_angle_between_normals() {
        // Not literally a "quarter" pipe here: the two planes meet at 45
        // degrees (not 90), so the fillet's own sector -- the angle between
        // sector_start and sector_end -- comes out to 45 degrees too, not
        // 90. This is the real proof `between_planes` derives whatever
        // sector angle actually results, rather than assuming a right angle.
        let (wall, diagonal) = non_perpendicular_walls();
        let pipe =
            StaticQuarterPipe::between_planes(&wall, &diagonal, 1.0, Vec3::new(0.0, 0.0, 1.0));
        let angle = pipe.sector_start.dot(&pipe.sector_end).acos();
        assert!(
            (angle - std::f32::consts::FRAC_PI_4).abs() < 1e-3,
            "expected a 45-degree sector, got {} radians",
            angle
        );
    }

    #[test]
    fn between_non_perpendicular_planes_sector_faces_the_sharp_corner_it_replaces() {
        // The real proof the generalized sector orientation is correct: the
        // sharp corner this fillet rounds off (where the two flat planes
        // would otherwise meet exactly, both passing through the origin
        // here) must sit outside the fillet's own radius (the fillet cuts
        // the corner off, not past it) and within its sector (the fillet
        // actually faces the missing material it's replacing, not away from
        // it) -- using the same containment test `collision::
        // sphere_vs_quarter_pipe` uses.
        let (wall, diagonal) = non_perpendicular_walls();
        let radius = 1.0;
        let pipe =
            StaticQuarterPipe::between_planes(&wall, &diagonal, radius, Vec3::new(0.0, 0.0, 1.0));

        let corner = Vec3::ZERO;
        let rel = corner - pipe.axis_point;
        let dist = rel.length();
        let dir = rel * (1.0 / dist);

        assert!(
            dist > radius,
            "expected the sharp corner to sit outside the fillet's own radius, got dist={dist}"
        );
        assert!(
            pipe.sector_start.cross(&dir).dot(&pipe.axis_direction) >= 0.0
                && dir.cross(&pipe.sector_end).dot(&pipe.axis_direction) >= 0.0,
            "expected the fillet's sector to face the sharp corner it replaces"
        );
    }

    #[test]
    fn between_planes_self_corrects_a_backwards_axis_direction() {
        // Regardless of which of the two opposite directions along the
        // shared edge line the caller passes in, sector_start -> sector_end
        // must sweep the short arc in the *positive* sense around the
        // resulting axis_direction -- between_planes flips a backwards input
        // internally to guarantee this (see its own doc comment).
        let (wall, diagonal) = non_perpendicular_walls();
        let forward =
            StaticQuarterPipe::between_planes(&wall, &diagonal, 1.0, Vec3::new(0.0, 0.0, 1.0));
        let backward =
            StaticQuarterPipe::between_planes(&wall, &diagonal, 1.0, Vec3::new(0.0, 0.0, -1.0));
        for pipe in [forward, backward] {
            assert!(
                pipe.sector_start.cross(&pipe.sector_end).dot(&pipe.axis_direction) >= 0.0,
                "expected sector_start -> sector_end to sweep the positive way around axis_direction, got {pipe:?}"
            );
        }
    }
}
