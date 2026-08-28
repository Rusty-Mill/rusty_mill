//! Rigid-body integration, ported from Bullet3
//! (`bullet3/src/BulletDynamics/Dynamics/btRigidBody.cpp` and
//! `bullet3/src/LinearMath/btTransformUtil.h`, zlib license — see
//! `THIRD_PARTY_NOTICES.md`). Scalar-only (no SIMD), since this is a
//! from-scratch Rust translation, not a binding — see ADR-0004.

use crate::body::Sphere;
use rb_domain::{Quat, Vec3};

/// `MAX_ANGVEL` in `btRigidBody.cpp`: collision calculations become
/// unreliable above this angular speed, so integration clamps to it.
const MAX_ANGVEL: f32 = std::f32::consts::FRAC_PI_2;

/// `ANGULAR_MOTION_THRESHOLD` in `btTransformUtil.h`: `0.5 * SIMD_HALF_PI`.
const ANGULAR_MOTION_THRESHOLD: f32 = 0.5 * std::f32::consts::FRAC_PI_2;

/// Port of `btRigidBody::applyGravity` + `applyCentralForce`: accumulates a
/// gravity force (`mass * gravity_accel`) into the body's force
/// accumulator. A no-op contract-wise for anything with zero inverse mass
/// would apply here too, but `Sphere` is always dynamic in this scope (see
/// `body.rs`), so that branch from Bullet's `isStaticOrKinematicObject()`
/// check doesn't apply.
pub fn apply_gravity(body: &mut Sphere, gravity_accel: Vec3) {
    body.apply_central_force(gravity_accel * body.mass());
}

/// Port of `btRigidBody::applyDamping`, exponential-decay branch (the
/// `#else` of `BT_USE_OLD_DAMPING_METHOD`, which is Bullet's default). The
/// `m_additionalDamping` extra-stability branch is intentionally omitted —
/// it's an opt-in stability hack in upstream Bullet, off by default, and
/// not part of the core algorithm this port targets.
pub fn apply_damping(body: &mut Sphere, dt: f32) {
    body.linear_velocity *= (1.0 - body.linear_damping).max(0.0).powf(dt);
    body.angular_velocity *= (1.0 - body.angular_damping).max(0.0).powf(dt);
}

/// Port of `btRigidBody::integrateVelocities`: semi-implicit Euler update
/// of linear/angular velocity from the accumulated force/torque, with
/// Bullet's angular-velocity clamp.
pub fn integrate_velocities(body: &mut Sphere, dt: f32) {
    let inv_mass = body.inv_mass();
    let inv_inertia = body.inv_inertia();
    body.linear_velocity += body.total_force() * (inv_mass * dt);
    body.angular_velocity += body.total_torque() * (inv_inertia * dt);

    let angvel = body.angular_velocity.length();
    if angvel * dt > MAX_ANGVEL {
        body.angular_velocity *= (MAX_ANGVEL / dt) / angvel;
    }
}

/// Port of `btTransformUtil::integrateTransform`'s exponential-map
/// orientation update (F. Sebastian Grassia, "Practical Parameterization of
/// Rotations Using the Exponential Map") plus the linear position update.
/// Bullet's alternative `QUATERNION_DERIVATIVE` branch is `#ifdef`'d out
/// upstream too — the exponential map is what Bullet actually ships.
pub fn integrate_transform(
    position: Vec3,
    orientation: Quat,
    linear_velocity: Vec3,
    angular_velocity: Vec3,
    dt: f32,
) -> (Vec3, Quat) {
    let new_position = position + linear_velocity * dt;

    let angle_sq = angular_velocity.length_squared();
    let mut angle = if angle_sq > 1e-12 {
        angle_sq.sqrt()
    } else {
        0.0
    };

    if angle * dt > ANGULAR_MOTION_THRESHOLD {
        angle = ANGULAR_MOTION_THRESHOLD / dt;
    }

    let axis = if angle < 0.001 {
        // Taylor expansion of sinc(angle) for small angles, matching
        // Bullet's small-angle branch exactly.
        angular_velocity * (0.5 * dt - (dt * dt * dt) * 0.020_833_333 * angle * angle)
    } else {
        angular_velocity * ((0.5 * angle * dt).sin() / angle)
    };

    let delta_orientation = Quat::new(axis.x, axis.y, axis.z, (angle * dt * 0.5).cos());
    let predicted = delta_orientation.mul(&orientation);

    let new_orientation = if predicted.length_squared() > 1e-12 {
        predicted.normalize()
    } else {
        orientation
    };

    (new_position, new_orientation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gravity_accumulates_force_proportional_to_mass() {
        let mut s = Sphere::new(1.0, 2.0, Vec3::ZERO);
        apply_gravity(&mut s, Vec3::new(0.0, 0.0, -10.0));
        assert_eq!(s.total_force(), Vec3::new(0.0, 0.0, -20.0));
    }

    #[test]
    fn zero_damping_leaves_velocity_unchanged() {
        let mut s = Sphere::new(1.0, 1.0, Vec3::ZERO);
        s.linear_velocity = Vec3::new(3.0, 0.0, 0.0);
        apply_damping(&mut s, 1.0 / 60.0);
        assert_eq!(s.linear_velocity, Vec3::new(3.0, 0.0, 0.0));
    }

    #[test]
    fn full_damping_zeroes_velocity_immediately() {
        let mut s = Sphere::new(1.0, 1.0, Vec3::ZERO);
        s.linear_velocity = Vec3::new(3.0, 0.0, 0.0);
        s.linear_damping = 1.0;
        apply_damping(&mut s, 1.0 / 60.0);
        assert_eq!(s.linear_velocity, Vec3::ZERO);
    }

    #[test]
    fn integrate_velocities_applies_semi_implicit_euler_step() {
        let mut s = Sphere::new(1.0, 2.0, Vec3::ZERO);
        apply_gravity(&mut s, Vec3::new(0.0, 0.0, -10.0));
        integrate_velocities(&mut s, 0.5);
        // dv = F/m * dt = (0,0,-20)/2 * 0.5 = (0,0,-5)
        assert!((s.linear_velocity - Vec3::new(0.0, 0.0, -5.0)).length() < 1e-6);
    }

    #[test]
    fn angular_velocity_is_clamped_to_max_angvel() {
        let mut s = Sphere::new(1.0, 1.0, Vec3::ZERO);
        s.apply_torque(Vec3::new(0.0, 0.0, 1_000_000.0));
        integrate_velocities(&mut s, 1.0);
        assert!(s.angular_velocity.length() * 1.0 <= MAX_ANGVEL + 1e-4);
    }

    #[test]
    fn integrate_transform_moves_position_by_velocity_times_dt() {
        let (pos, _) = integrate_transform(
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::ZERO,
            0.5,
        );
        assert!((pos - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-6);
    }

    #[test]
    fn integrate_transform_with_zero_angular_velocity_keeps_orientation() {
        let (_, orn) = integrate_transform(Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO, Vec3::ZERO, 0.5);
        assert_eq!(orn, Quat::IDENTITY);
    }

    #[test]
    fn integrate_transform_produces_normalized_orientation() {
        let (_, orn) = integrate_transform(
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 3.0),
            1.0 / 60.0,
        );
        assert!((orn.length_squared() - 1.0).abs() < 1e-5);
    }
}
