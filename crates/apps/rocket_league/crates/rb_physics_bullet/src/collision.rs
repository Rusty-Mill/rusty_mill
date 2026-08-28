//! Narrow-phase collision detection: body-vs-static-plane (ground contact)
//! and sphere-vs-box (ball-vs-car). Bullet's real algorithms for these
//! (`btSphereBoxCollisionAlgorithm`, the generic convex-vs-plane path) are
//! more general than this needs to be — for an infinite plane, and for a
//! sphere against an axis-aligned-in-its-own-frame box, both reduce to
//! exact analytic/closed-form tests, which is what's ported here as the
//! direct equivalent, not a simplification of Bullet's result.
//!
//! Not implemented: box-vs-box collision (two cars against each other) —
//! this scope has exactly one car, so the pairing never arises. Adding it
//! for real would need a general convex narrow-phase algorithm (SAT or
//! GJK/EPA), not a small extension of `sphere_vs_box` below.

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

/// Analytic sphere-vs-box contact: the box's closest point to the sphere
/// center is found by clamping the sphere center (transformed into the
/// box's local frame) to `[-half_extents, half_extents]` per axis — a
/// closed form of the closest-point-on-OBB query
/// `btBoxSphereCollisionAlgorithm` runs via a general support-mapping
/// search, since a box's closest point to an external point never needs
/// one.
///
/// A second case — the sphere's center already inside the box (deep
/// penetration, e.g. a fast-moving ball tunnelling a frame's worth into
/// the car) — has no such "closest point" (the clamp is a no-op, so
/// `local_center - clamped` is the zero vector, not a direction to push
/// along). Bullet's own box-sphere algorithm handles this via the box's
/// general SAT machinery; here it's simpler, because the *other* shape is
/// a single point: the box's three face axes are the only separating axes
/// that can possibly apply, so picking whichever face is nearest (least
/// negative penetration) is exact, not approximate.
fn sphere_vs_box(
    sphere_position: Vec3,
    radius: f32,
    box_position: Vec3,
    box_orientation: Quat,
    half_extents: Vec3,
) -> Option<Contact> {
    let local_center = box_orientation
        .conjugate()
        .rotate(&(sphere_position - box_position));

    let clamped = Vec3::new(
        local_center.x.clamp(-half_extents.x, half_extents.x),
        local_center.y.clamp(-half_extents.y, half_extents.y),
        local_center.z.clamp(-half_extents.z, half_extents.z),
    );
    let outside_offset = local_center - clamped;
    let outside_distance = outside_offset.length();

    let (local_point, local_normal, gap) = if outside_distance > 1e-6 {
        (
            clamped,
            outside_offset * (1.0 / outside_distance),
            outside_distance - radius,
        )
    } else {
        let margin = Vec3::new(
            half_extents.x - local_center.x.abs(),
            half_extents.y - local_center.y.abs(),
            half_extents.z - local_center.z.abs(),
        );
        let sign = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
        let (normal, point, depth) = if margin.x <= margin.y && margin.x <= margin.z {
            let s = sign(local_center.x);
            (
                Vec3::new(s, 0.0, 0.0),
                Vec3::new(s * half_extents.x, local_center.y, local_center.z),
                margin.x,
            )
        } else if margin.y <= margin.z {
            let s = sign(local_center.y);
            (
                Vec3::new(0.0, s, 0.0),
                Vec3::new(local_center.x, s * half_extents.y, local_center.z),
                margin.y,
            )
        } else {
            let s = sign(local_center.z);
            (
                Vec3::new(0.0, 0.0, s),
                Vec3::new(local_center.x, local_center.y, s * half_extents.z),
                margin.z,
            )
        };
        (point, normal, -depth - radius)
    };

    if gap > CONTACT_PROCESSING_THRESHOLD {
        return None;
    }

    Some(Contact {
        normal: box_orientation.rotate(&local_normal),
        point: box_position + box_orientation.rotate(&local_point),
        penetration_depth: -gap,
    })
}

/// Dispatches a contact test between two dynamic bodies, for the one pair
/// this scope needs: a sphere (the ball) against a box (a car). `normal`
/// always points from `b` toward `a` (matching `contacts_vs_plane`'s
/// convention with `b` playing the plane's "reference" role), so the
/// two-body solver can apply `+impulse` to `a` and `-impulse` to `b` along
/// it without needing to know which argument was the sphere.
///
/// Sphere-vs-sphere and box-vs-box return `None`: this scope has exactly
/// one ball and one car, so neither pairing ever actually occurs — not a
/// simplification of a real case, just one this codebase has no caller for.
pub fn contact_between(a: &RigidBody, b: &RigidBody) -> Option<Contact> {
    match (a.shape, b.shape) {
        (Shape::Sphere { radius }, Shape::Box { half_extents }) => {
            sphere_vs_box(a.position, radius, b.position, b.orientation, half_extents)
        }
        (Shape::Box { half_extents }, Shape::Sphere { radius }) => {
            sphere_vs_box(b.position, radius, a.position, a.orientation, half_extents).map(|c| {
                Contact {
                    normal: -c.normal,
                    ..c
                }
            })
        }
        (Shape::Sphere { .. }, Shape::Sphere { .. }) | (Shape::Box { .. }, Shape::Box { .. }) => {
            None
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

    fn stationary_car() -> RigidBody {
        RigidBody::car_box(Vec3::new(60.0, 30.0, 18.0), 180.0, Vec3::ZERO)
    }

    #[test]
    fn ball_far_from_car_has_no_contact() {
        let ball = RigidBody::sphere(92.75, 1.0, Vec3::new(1000.0, 0.0, 0.0));
        assert!(contact_between(&ball, &stationary_car()).is_none());
    }

    #[test]
    fn ball_touching_car_face_has_zero_penetration() {
        // The ball's surface exactly meets the car's +X face.
        let ball = RigidBody::sphere(92.75, 1.0, Vec3::new(60.0 + 92.75, 0.0, 0.0));
        let contact = contact_between(&ball, &stationary_car()).unwrap();
        assert!(contact.penetration_depth.abs() < 1e-4);
        assert!((contact.normal - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn ball_overlapping_car_face_has_positive_penetration() {
        let ball = RigidBody::sphere(92.75, 1.0, Vec3::new(60.0 + 50.0, 0.0, 0.0));
        let contact = contact_between(&ball, &stationary_car()).unwrap();
        assert!((contact.penetration_depth - (92.75 - 50.0)).abs() < 1e-4);
    }

    #[test]
    fn ball_center_embedded_in_car_pushes_out_the_nearest_face() {
        // Ball center sits inside the car box, closest to the +Z (roof)
        // face (margin 2.0) rather than +X (margin 40.0) or +Y (margin
        // 10.0) — the deep-penetration branch must pick +Z.
        let ball = RigidBody::sphere(5.0, 1.0, Vec3::new(20.0, 20.0, 16.0));
        let contact = contact_between(&ball, &stationary_car()).unwrap();
        assert!((contact.normal - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-5);
        assert!(contact.penetration_depth > 0.0);
    }

    #[test]
    fn contact_between_is_antisymmetric_in_argument_order() {
        let ball = RigidBody::sphere(92.75, 1.0, Vec3::new(60.0 + 50.0, 0.0, 0.0));
        let car = stationary_car();
        let ball_car = contact_between(&ball, &car).unwrap();
        let car_ball = contact_between(&car, &ball).unwrap();
        assert!((ball_car.normal + car_ball.normal).length() < 1e-5);
        assert!((ball_car.penetration_depth - car_ball.penetration_depth).abs() < 1e-4);
    }

    #[test]
    fn contact_between_two_spheres_is_none() {
        let a = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        let b = RigidBody::sphere(1.0, 1.0, Vec3::new(0.5, 0.0, 0.0));
        assert!(contact_between(&a, &b).is_none());
    }

    #[test]
    fn contact_between_two_boxes_is_none() {
        let a = stationary_car();
        let b = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(0.5, 0.0, 0.0));
        assert!(contact_between(&a, &b).is_none());
    }
}
