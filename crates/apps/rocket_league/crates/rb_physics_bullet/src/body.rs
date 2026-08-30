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

/// An immovable partial-cylinder fillet connecting two perpendicular flat
/// planes (a wall and the floor, or a wall and the ceiling) —
/// `RB-PHYSICS-001-FR-020`, Rocket League's real curved wall-to-floor and
/// wall-to-ceiling transitions. Like `StaticPlane`, infinite along its own
/// axis (`axis_direction`) — this crate doesn't model a finite wall length
/// any more for the curve than it already does for the flat walls
/// themselves.
///
/// The playable side is the *inside* of the partial cylinder (like riding
/// the concave face of a skateboard quarter-pipe, which is exactly what
/// this shape is named after) — a point is only governed by this fillet at
/// all when its direction from `axis_point` (projected perpendicular to
/// `axis_direction`) falls within the 90-degree sector from `sector_start`
/// to `sector_end`; outside that sector, whichever flat plane the fillet
/// bridges takes over instead (this shape doesn't know or care about that —
/// see `PhysicsWorld::step`, which resolves the flat planes and this fillet
/// as independent, additive contact sources, same as it already does for
/// the ground and every wall).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticQuarterPipe {
    pub axis_point: Vec3,
    /// Unit vector; the fillet is infinite along this direction.
    pub axis_direction: Vec3,
    pub radius: f32,
    /// Unit vector, perpendicular to `axis_direction`: the direction from
    /// `axis_point` toward the fillet's tangent point on the first flat
    /// plane it bridges.
    pub sector_start: Vec3,
    /// Unit vector, perpendicular to `axis_direction` and to
    /// `sector_start` (a 90-degree sector): the direction from
    /// `axis_point` toward the fillet's tangent point on the second flat
    /// plane it bridges.
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

    /// Derives a fillet of the given `radius` connecting two perpendicular
    /// `StaticPlane`s — e.g. the floor and a side wall, or the ceiling and
    /// a side wall — given a direction along which the fillet should run
    /// (`axis_direction`, which must be perpendicular to both planes'
    /// normals; for an axis-aligned arena wall, this is simply "along the
    /// wall," e.g. `(0, 1, 0)` for a wall running along Y).
    ///
    /// Only exact for two *perpendicular* planes (`plane_a.normal` and
    /// `plane_b.normal` themselves perpendicular, and both perpendicular to
    /// `axis_direction`) — true for every cardinal (axis-aligned) arena
    /// wall's own floor/ceiling seam, not for a diagonal corner wall's (see
    /// `RB-PHYSICS-001-FR-020`'s Non-goals; this port doesn't attempt a
    /// fillet there yet).
    ///
    /// The axis point sits `radius` units inward from *both* planes along
    /// their own normals (so the fillet's surface is tangent to each plane
    /// exactly `radius` units from where they'd otherwise meet at a sharp
    /// edge); `sector_start`/`sector_end` are simply the negation of each
    /// plane's own normal (the direction from the axis back toward that
    /// plane's tangent point).
    pub fn between_planes(
        plane_a: &StaticPlane,
        plane_b: &StaticPlane,
        radius: f32,
        axis_direction: Vec3,
    ) -> StaticQuarterPipe {
        let axis_point =
            plane_a.normal * (plane_a.offset + radius) + plane_b.normal * (plane_b.offset + radius);
        StaticQuarterPipe::new(
            axis_point,
            axis_direction,
            radius,
            -plane_a.normal,
            -plane_b.normal,
        )
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
}
