//! Narrow-phase collision detection, scoped to sphere-vs-static-plane for
//! v0. Bullet's real sphere/plane path (`btSphereBoxCollisionAlgorithm` for
//! box, or the generic convex-vs-plane path) is more general than this
//! needs to be — for an actual infinite plane, the contact test reduces to
//! comparing signed distance against the sphere radius, which is what's
//! ported here as the direct analytic equivalent, not a simplification of
//! Bullet's result.

use crate::body::{Sphere, StaticPlane};
use rb_domain::Vec3;

/// A single contact point between a dynamic sphere and a static plane.
/// Field names/meaning match `btManifoldPoint`: `normal` points from the
/// plane toward the sphere (matching `m_normalWorldOnB` when the plane is
/// treated as body B), `point` is the contact point on the sphere's
/// surface, and `penetration_depth` is positive when overlapping (Bullet
/// stores the negative of this as `getDistance()`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    pub normal: Vec3,
    pub point: Vec3,
    pub penetration_depth: f32,
}

/// `contact_processing_threshold`, matching
/// `btManifoldPoint::getContactProcessingThreshold()`: contacts aren't
/// generated until the gap is this small (or overlapping), so resting
/// bodies don't jitter between "touching" and "not touching" every frame.
const CONTACT_PROCESSING_THRESHOLD: f32 = 0.01;

pub fn sphere_vs_plane(sphere: &Sphere, plane: &StaticPlane) -> Option<Contact> {
    let center_distance = plane.signed_distance(&sphere.position);
    let gap = center_distance - sphere.radius;

    if gap > CONTACT_PROCESSING_THRESHOLD {
        return None;
    }

    let point = sphere.position - plane.normal * center_distance;
    Some(Contact {
        normal: plane.normal,
        point,
        penetration_depth: -gap,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ground() -> StaticPlane {
        StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0)
    }

    #[test]
    fn sphere_far_above_ground_has_no_contact() {
        let s = Sphere::new(1.0, 1.0, Vec3::new(0.0, 0.0, 10.0));
        assert!(sphere_vs_plane(&s, &ground()).is_none());
    }

    #[test]
    fn sphere_resting_exactly_on_ground_has_zero_penetration() {
        let s = Sphere::new(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        let c = sphere_vs_plane(&s, &ground()).unwrap();
        assert!((c.penetration_depth).abs() < 1e-6);
        assert_eq!(c.normal, Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn sphere_embedded_in_ground_has_positive_penetration() {
        let s = Sphere::new(1.0, 1.0, Vec3::new(0.0, 0.0, 0.5));
        let c = sphere_vs_plane(&s, &ground()).unwrap();
        assert!((c.penetration_depth - 0.5).abs() < 1e-6);
    }

    #[test]
    fn contact_point_lies_on_the_plane() {
        let s = Sphere::new(1.0, 1.0, Vec3::new(2.0, -3.0, 0.5));
        let c = sphere_vs_plane(&s, &ground()).unwrap();
        assert!(ground().signed_distance(&c.point).abs() < 1e-6);
    }
}
