//! Narrow-phase collision detection against a static plane. Bullet's real
//! sphere/box-vs-plane paths (`btSphereBoxCollisionAlgorithm`, the generic
//! convex-vs-plane path) are more general than this needs to be — for an
//! actual infinite plane, both shapes' contact tests reduce to exact
//! analytic forms, which is what's ported here as the direct equivalent,
//! not a simplification of Bullet's result.
//!
//! Not implemented: box-vs-sphere (car-vs-ball) collision. Both bodies
//! currently only collide with the ground plane, never each other — see
//! `RB-PHYSICS-001-FR-004`'s open items. Adding it needs a real convex
//! narrow-phase algorithm (SAT or GJK/EPA), not a small extension of the
//! plane-specific analytic tests below.

use crate::body::{RigidBody, Shape};
use rb_domain::{Quat, Vec3};

/// A single contact point between a dynamic body and a static plane.
/// Field names/meaning match `btManifoldPoint`: `normal` points from the
/// plane toward the body (matching `m_normalWorldOnB` when the plane is
/// treated as body B), `point` is the contact point in world space, and
/// `penetration_depth` is positive when overlapping (Bullet stores the
/// negative of this as `getDistance()`).
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

use crate::body::StaticPlane;

/// Analytic sphere-vs-plane contact: the sphere's closest point to the
/// plane is always `position - normal * radius`, so the general
/// closest-point search Bullet's narrow phase would otherwise run reduces
/// to a single distance comparison.
fn sphere_vs_plane(position: Vec3, radius: f32, plane: &StaticPlane) -> Option<Contact> {
    let center_distance = plane.signed_distance(&position);
    let gap = center_distance - radius;

    if gap > CONTACT_PROCESSING_THRESHOLD {
        return None;
    }

    let point = position - plane.normal * center_distance;
    Some(Contact {
        normal: plane.normal,
        point,
        penetration_depth: -gap,
    })
}

/// Analytic box-vs-plane contact: a box's extreme points against any plane
/// are exactly its 8 corners, so testing each corner's signed distance is
/// an exact result, not an approximation — up to 4 corners survive (a box
/// resting flat), 2 (resting on an edge), or 1 (resting on a corner).
///
/// Each surviving corner's own world position is used as `point` (not
/// projected onto the plane, unlike `sphere_vs_plane`'s convention) —
/// for a box this matters even at rest, since a tilted box's corner isn't
/// generally directly "below" the body's center along the plane normal,
/// and the solver needs the true contact-to-center offset (`rel_pos`) to
/// compute torque correctly.
fn box_vs_plane(
    position: Vec3,
    orientation: Quat,
    half_extents: Vec3,
    plane: &StaticPlane,
) -> Vec<Contact> {
    let mut contacts = Vec::with_capacity(4);
    for &sx in &[-1.0f32, 1.0] {
        for &sy in &[-1.0f32, 1.0] {
            for &sz in &[-1.0f32, 1.0] {
                let local_corner = Vec3::new(
                    sx * half_extents.x,
                    sy * half_extents.y,
                    sz * half_extents.z,
                );
                let world_corner = position + orientation.rotate(&local_corner);
                let gap = plane.signed_distance(&world_corner);
                if gap <= CONTACT_PROCESSING_THRESHOLD {
                    contacts.push(Contact {
                        normal: plane.normal,
                        point: world_corner,
                        penetration_depth: -gap,
                    });
                }
            }
        }
    }
    contacts
}

/// Dispatches to the shape-appropriate plane contact test, returning every
/// contact point found (0 to 4, depending on shape and orientation).
pub fn contacts_vs_plane(body: &RigidBody, plane: &StaticPlane) -> Vec<Contact> {
    match body.shape {
        Shape::Sphere { radius } => sphere_vs_plane(body.position, radius, plane)
            .into_iter()
            .collect(),
        Shape::Box { half_extents } => {
            box_vs_plane(body.position, body.orientation, half_extents, plane)
        }
    }
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
        let s = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 10.0));
        assert!(contacts_vs_plane(&s, &ground()).is_empty());
    }

    #[test]
    fn sphere_resting_exactly_on_ground_has_zero_penetration() {
        let s = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        let contacts = contacts_vs_plane(&s, &ground());
        assert_eq!(contacts.len(), 1);
        assert!((contacts[0].penetration_depth).abs() < 1e-6);
        assert_eq!(contacts[0].normal, Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn sphere_embedded_in_ground_has_positive_penetration() {
        let s = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 0.5));
        let contacts = contacts_vs_plane(&s, &ground());
        assert!((contacts[0].penetration_depth - 0.5).abs() < 1e-6);
    }

    #[test]
    fn sphere_contact_point_lies_on_the_plane() {
        let s = RigidBody::sphere(1.0, 1.0, Vec3::new(2.0, -3.0, 0.5));
        let contacts = contacts_vs_plane(&s, &ground());
        assert!(ground().signed_distance(&contacts[0].point).abs() < 1e-6);
    }

    #[test]
    fn box_far_above_ground_has_no_contact() {
        let b = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(0.0, 0.0, 10.0));
        assert!(contacts_vs_plane(&b, &ground()).is_empty());
    }

    #[test]
    fn box_resting_flat_has_four_contacts() {
        let b = RigidBody::car_box(Vec3::new(1.0, 2.0, 0.5), 1.0, Vec3::new(0.0, 0.0, 0.5));
        let contacts = contacts_vs_plane(&b, &ground());
        assert_eq!(contacts.len(), 4);
        for c in &contacts {
            assert!(c.penetration_depth.abs() < 1e-5);
            assert_eq!(c.normal, Vec3::new(0.0, 0.0, 1.0));
        }
    }

    #[test]
    fn box_tilted_forty_five_degrees_about_an_axis_has_two_contacts() {
        // A 45-degree tilt about Y puts the box on one bottom edge (two
        // corners), lifting the other two corners of that face into the
        // air — the classic "resting on an edge" case. Quaternion angle is
        // half-angle (Quat::new(axis * sin(angle/2), cos(angle/2))), so a
        // 45-degree *full* rotation needs half = 22.5 degrees.
        let half = std::f32::consts::FRAC_PI_8;
        let tilt = Quat::new(0.0, half.sin(), 0.0, half.cos());
        let half_extents = Vec3::new(1.0, 1.0, 1.0);
        // Half-diagonal of the box's cross-section touches the ground.
        let resting_z = (half_extents.x * half_extents.x + half_extents.z * half_extents.z).sqrt();
        let mut b = RigidBody::car_box(half_extents, 1.0, Vec3::new(0.0, 0.0, resting_z));
        b.orientation = tilt;
        let contacts = contacts_vs_plane(&b, &ground());
        assert_eq!(
            contacts.len(),
            2,
            "expected exactly one bottom edge (2 corners) touching"
        );
    }

    #[test]
    fn box_embedded_in_ground_has_positive_penetration() {
        let b = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(0.0, 0.0, 0.5));
        let contacts = contacts_vs_plane(&b, &ground());
        assert_eq!(contacts.len(), 4);
        for c in &contacts {
            assert!((c.penetration_depth - 0.5).abs() < 1e-5);
        }
    }
}
