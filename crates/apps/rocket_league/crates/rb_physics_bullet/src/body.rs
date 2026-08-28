//! Rigid body types for v0's scope: a dynamic sphere (the ball) and a
//! static plane (the ground). See `RB-PHYSICS-001` for why this scope —
//! spheres have isotropic inertia, which lets this increment port Bullet's
//! real integration/solver math faithfully without also needing a general
//! 3x3 inertia tensor (that's required once car boxes are added, tracked
//! as the next `RB-PHYSICS-001` increment).
//!
//! World convention: +Z is up, matching Unreal Engine (which Rocket League
//! runs on) rather than the +Y-up convention common in some other engines.

use rb_domain::{Quat, Vec3};

/// A dynamic rigid body shaped as a sphere. Mirrors the subset of
/// `bullet3/src/BulletDynamics/Dynamics/btRigidBody.h`'s fields this crate's
/// integration and solver code actually needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sphere {
    pub radius: f32,
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
    inv_inertia: f32,

    total_force: Vec3,
    total_torque: Vec3,
}

impl Sphere {
    /// `mass` must be positive — a zero/negative mass sphere isn't a
    /// meaningful dynamic body in this scope (Bullet handles mass == 0 as
    /// "static", but that's what `StaticPlane` is for here).
    pub fn new(radius: f32, mass: f32, position: Vec3) -> Sphere {
        assert!(
            mass > 0.0,
            "Sphere mass must be positive; use a static body for immovable objects"
        );
        assert!(radius > 0.0, "Sphere radius must be positive");
        // I = 2/5 m r^2 for a solid sphere (isotropic, so a scalar suffices
        // in place of Bullet's general `m_invInertiaTensorWorld` 3x3).
        let inertia = 0.4 * mass * radius * radius;
        Sphere {
            radius,
            position,
            orientation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            linear_damping: 0.0,
            angular_damping: 0.0,
            restitution: 0.5,
            friction: 0.5,
            inv_mass: 1.0 / mass,
            inv_inertia: 1.0 / inertia,
            total_force: Vec3::ZERO,
            total_torque: Vec3::ZERO,
        }
    }

    pub fn inv_mass(&self) -> f32 {
        self.inv_mass
    }

    pub fn inv_inertia(&self) -> f32 {
        self.inv_inertia
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
        self.angular_velocity += rel_pos.cross(&impulse) * self.inv_inertia;
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
/// smaller, equally-faithful representation for v0's one static shape.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_inertia_matches_solid_sphere_formula() {
        let s = Sphere::new(1.0, 2.0, Vec3::ZERO);
        // I = 2/5 * m * r^2 = 0.4 * 2.0 * 1.0 = 0.8
        assert!((1.0 / s.inv_inertia() - 0.8).abs() < 1e-6);
    }

    #[test]
    #[should_panic(expected = "mass must be positive")]
    fn zero_mass_sphere_panics() {
        Sphere::new(1.0, 0.0, Vec3::ZERO);
    }

    #[test]
    fn velocity_at_point_includes_rotational_contribution() {
        let mut s = Sphere::new(1.0, 1.0, Vec3::ZERO);
        s.angular_velocity = Vec3::new(0.0, 0.0, 1.0); // spinning about +Z
        let rel_pos = Vec3::new(1.0, 0.0, 0.0); // point on +X of the surface
                                                // omega x r = (0,0,1) x (1,0,0) = (0,1,0)
        let v = s.velocity_at_point(&rel_pos);
        assert!((v - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-6);
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
}
