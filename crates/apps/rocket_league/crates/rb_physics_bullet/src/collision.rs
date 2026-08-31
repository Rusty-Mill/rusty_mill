//! Narrow-phase collision detection: body-vs-static-plane (ground contact),
//! sphere-vs-box (ball-vs-car), box-vs-box (car-vs-car), and, since
//! `RB-PHYSICS-001-FR-033`, sphere-vs-sphere (the ball against a
//! `net::NetMesh`'s free point masses). The plane, sphere-vs-box, and
//! sphere-vs-sphere tests are all closed-form (see their own doc comments);
//! box-vs-box needs a real separating-axis test (`box_vs_box`), since two
//! arbitrarily-oriented boxes have no such shortcut. `PhysicsWorld` now
//! calls `box_vs_box` (via `contacts_between`) for every pair of cars in
//! its scene, not just as a unit-tested-in-isolation capability.
//!
//! Since `RB-PHYSICS-001-FR-047`, every closed-form test here
//! (`sphere_vs_plane`, `box_vs_plane`, `sphere_vs_box`, `sphere_vs_sphere`)
//! has been checked directly against Bullet's own real
//! `btConvexPlaneCollisionAlgorithm`/`btSphereBoxCollisionAlgorithm`/
//! `btSphereSphereCollisionAlgorithm` source — see each function's own doc
//! comment for its finding, and `box_vs_plane`'s in particular for the one
//! genuine, deliberate divergence found (already the case for `box_vs_box`
//! since `RB-PHYSICS-001-FR-042`).

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

use crate::body::{
    StaticBoundedWall, StaticCornerFillet, StaticGoalWall, StaticPlane, StaticQuarterPipe,
};

/// Analytic sphere-vs-plane contact: the sphere's closest point to the
/// plane is always `position - normal * radius`, so the general
/// closest-point search Bullet's narrow phase would otherwise run reduces
/// to a single distance comparison.
///
/// Confirmed against real `btConvexPlaneCollisionAlgorithm::processCollision`
/// (`RB-PHYSICS-001-FR-047`): for a sphere, its GJK support vertex along
/// `-planeNormal` is exactly `center - radius * planeNormal`, so this
/// closed form is Bullet's own real algorithm reduced analytically, not an
/// approximation of it — same `distance`/`pOnB`/`normalOnSurfaceB`
/// (`= plane.getWorldTransform().getBasis() * planeNormal`, matching this
/// function's own `normal: plane.normal`) conventions confirmed exact.
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
///
/// **One genuine, deliberate divergence from real Bullet, found and not
/// adopted (`RB-PHYSICS-001-FR-047`).** Real
/// `btConvexPlaneCollisionAlgorithm::processCollision` does NOT compute
/// every extreme corner in one pass: it calls a single GJK
/// `localGetSupportingVertex` query along `-planeNormal`, producing
/// exactly *one* contact point per frame — a box resting flat on a plane
/// gets a persistent-manifold-accumulated set of up to 4 points only
/// gradually, as numerical jitter shifts which corner the single support
/// query happens to return frame to frame (its own optional "perturbation"
/// multi-point path — `m_numPerturbationIterations` re-querying the
/// support vertex at several rotated orientations — is configured off by
/// default: `btConvexPlaneCollisionAlgorithm::CreateFunc`'s own real
/// default is `m_numPerturbationIterations = 1`,
/// `m_minimumPointsPerturbationThreshold = 0`, so the perturbation loop's
/// own `getNumContacts() < m_minimumPointsPerturbationThreshold` guard is
/// never true). This function's own instantaneous, exact 4-corner
/// computation is a deliberate, more rigorous simplification of that real
/// single-vertex-plus-persistence dance — same favorable divergence
/// already established for `box_vs_box` against `dBoxBox`
/// (`RB-PHYSICS-001-FR-042`) — not adopted, since replicating Bullet's own
/// frame-by-frame settling behavior here would only reintroduce several
/// frames of a box "sinking in" before all 4 corners register, with no
/// compensating benefit.
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

/// Like `sphere_vs_plane`, but against a `StaticGoalWall`'s window
/// (`RB-PHYSICS-001-FR-024`): a sphere whose *center* falls inside the
/// window gets no contact at all, letting it pass straight through into
/// the goal, rather than colliding with the wall material that isn't
/// there. Checking only the center (not the sphere's full silhouette)
/// is a documented simplification -- a ball can clip a few units of the
/// window's own edge before this stops registering contact there, the
/// same "check the flat contact point, not the full swept volume"
/// approximation every other static shape in this crate already makes
/// (see e.g. `sphere_vs_quarter_pipe`'s sector test, which has the same
/// property at a sector boundary).
fn sphere_vs_goal_wall(position: Vec3, radius: f32, wall: &StaticGoalWall) -> Option<Contact> {
    if wall.contains_in_window(&position) {
        return None;
    }
    sphere_vs_plane(position, radius, &wall.plane)
}

/// Like `box_vs_plane`, but against a `StaticGoalWall`'s window
/// (`RB-PHYSICS-001-FR-028`): each of the box's 8 corners is tested
/// individually against `contains_in_window` — a corner whose own
/// projection onto the plane's `u_axis`/`v_axis` falls inside the window
/// contributes no contact at all (that corner passes straight through),
/// exactly `sphere_vs_goal_wall`'s pass-through rule applied once per
/// corner instead of once for the sphere's single center point. A corner
/// outside the window behaves exactly like an ordinary `box_vs_plane`
/// corner. This means a car driving squarely through the goal mouth (every
/// corner inside the window) gets no contact at all and sails through,
/// while a car only partly lined up with the window still catches a real
/// contact on whichever corners are still outside it — the same "some
/// corners collide, some don't" partial-block behavior a real car easing
/// into a goal at an angle would produce, and the same per-corner
/// approximation technique `box_vs_quarter_pipe`/`box_vs_corner_fillet`
/// (`RB-PHYSICS-001-FR-027`) already established for curved geometry.
fn box_vs_goal_wall(
    position: Vec3,
    orientation: Quat,
    half_extents: Vec3,
    wall: &StaticGoalWall,
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
                if wall.contains_in_window(&world_corner) {
                    continue;
                }
                let gap = wall.plane.signed_distance(&world_corner);
                if gap <= CONTACT_PROCESSING_THRESHOLD {
                    contacts.push(Contact {
                        normal: wall.plane.normal,
                        point: world_corner,
                        penetration_depth: -gap,
                    });
                }
            }
        }
    }
    contacts
}

/// Dispatches a `StaticGoalWall`'s contact generation by shape: a sphere
/// (the ball) gets the windowed treatment (`sphere_vs_goal_wall`), and,
/// since `RB-PHYSICS-001-FR-028`, a box (a car) gets the equivalent
/// per-corner windowed treatment (`box_vs_goal_wall`) instead of falling
/// straight through to an unwindowed `contacts_vs_plane` the way it did
/// through FR-027 — a car can now actually drive into a goal through the
/// same window the ball already could pass through.
pub fn contacts_vs_goal_wall(body: &RigidBody, wall: &StaticGoalWall) -> Vec<Contact> {
    match body.shape {
        Shape::Sphere { radius } => sphere_vs_goal_wall(body.position, radius, wall)
            .into_iter()
            .collect(),
        Shape::Box { half_extents } => {
            box_vs_goal_wall(body.position, body.orientation, half_extents, wall)
        }
    }
}

/// Like `sphere_vs_plane`, but against a `StaticBoundedWall`'s own bound
/// (`RB-PHYSICS-001-FR-029`) — the opposite gate from `sphere_vs_goal_wall`:
/// a sphere whose *center* falls outside the bound gets no contact at all,
/// rather than one whose center falls inside it.
fn sphere_vs_bounded_wall(
    position: Vec3,
    radius: f32,
    wall: &StaticBoundedWall,
) -> Option<Contact> {
    if !wall.contains_in_bound(&position) {
        return None;
    }
    sphere_vs_plane(position, radius, &wall.plane)
}

/// Like `box_vs_goal_wall`, but against a `StaticBoundedWall`'s own bound
/// (`RB-PHYSICS-001-FR-029`) instead of a `StaticGoalWall`'s window — each
/// of the box's 8 corners is tested individually against
/// `contains_in_bound`, and a corner *outside* the bound contributes no
/// contact (the opposite gate from `box_vs_goal_wall`'s own per-corner
/// window test). A corner inside the bound falls through to an ordinary
/// `box_vs_plane`-style corner test.
fn box_vs_bounded_wall(
    position: Vec3,
    orientation: Quat,
    half_extents: Vec3,
    wall: &StaticBoundedWall,
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
                if !wall.contains_in_bound(&world_corner) {
                    continue;
                }
                let gap = wall.plane.signed_distance(&world_corner);
                if gap <= CONTACT_PROCESSING_THRESHOLD {
                    contacts.push(Contact {
                        normal: wall.plane.normal,
                        point: world_corner,
                        penetration_depth: -gap,
                    });
                }
            }
        }
    }
    contacts
}

/// Dispatches a `StaticBoundedWall`'s contact generation by shape
/// (`RB-PHYSICS-001-FR-029`) — used for the goal box's own side walls and
/// roof, each only solid within a rectangular bound immediately behind the
/// goal-mouth window (see `StaticBoundedWall`'s own doc comment for why an
/// unbounded plane there would be wrong).
pub fn contacts_vs_bounded_wall(body: &RigidBody, wall: &StaticBoundedWall) -> Vec<Contact> {
    match body.shape {
        Shape::Sphere { radius } => sphere_vs_bounded_wall(body.position, radius, wall)
            .into_iter()
            .collect(),
        Shape::Box { half_extents } => {
            box_vs_bounded_wall(body.position, body.orientation, half_extents, wall)
        }
    }
}

/// Analytic sphere-vs-quarter-pipe contact (`RB-PHYSICS-001-FR-020`): the
/// sphere lives *inside* the partial cylinder's concave face (see
/// `StaticQuarterPipe`'s own doc comment for why), so — unlike
/// `sphere_vs_plane`, where the sphere's closest point is always
/// `position - normal * radius` on the *outside* of a solid half-space —
/// contact fires as the sphere's surface approaches or crosses the
/// cylinder's own radius from the inside, and the correction pushes the
/// sphere back *toward* the axis, not away from it.
fn sphere_vs_quarter_pipe(
    position: Vec3,
    radius: f32,
    pipe: &StaticQuarterPipe,
) -> Option<Contact> {
    let rel = position - pipe.axis_point;
    let along_axis = rel.dot(&pipe.axis_direction);
    let perp = rel - pipe.axis_direction * along_axis;
    let dist = perp.length();

    // The sphere center sitting exactly on the pipe's axis has no
    // well-defined direction to push along — an unlikely exact
    // singularity, the same category as the landing-auto-orientation
    // assist's exactly-upside-down case (RB-PHYSICS-001-FR-018).
    if dist < 1e-6 {
        return None;
    }
    let dir = perp * (1.0 / dist);

    // Only this fillet's own sector is governed by it — outside that
    // range, whichever flat plane it bridges takes over instead (see the
    // struct's own doc comment). `sector_start`/`sector_end` can subtend
    // any angle up to 180 degrees (not just 90 — see
    // `StaticQuarterPipe::between_planes`, `RB-PHYSICS-001-FR-022`), so
    // membership needs a general "is dir within the wedge" test rather
    // than the old two-dot-products shortcut (which only happened to work
    // because a 90-degree sector's two edges are perpendicular): `dir` is
    // in the sector iff sweeping from `sector_start` toward it, and from it
    // toward `sector_end`, both go the *positive* way around
    // `axis_direction` (by at most a half turn) — exactly what these two
    // signed cross products, both non-negative, mean.
    if pipe.sector_start.cross(&dir).dot(&pipe.axis_direction) < 0.0
        || dir.cross(&pipe.sector_end).dot(&pipe.axis_direction) < 0.0
    {
        return None;
    }

    let gap = (pipe.radius - radius) - dist;
    if gap > CONTACT_PROCESSING_THRESHOLD {
        return None;
    }

    let point = pipe.axis_point + pipe.axis_direction * along_axis + dir * pipe.radius;
    Some(Contact {
        normal: -dir,
        point,
        penetration_depth: -gap,
    })
}

/// Analytic box-vs-quarter-pipe contact (`RB-PHYSICS-001-FR-027`): reduces
/// to the same "test every corner" technique `box_vs_plane` already uses
/// for a flat plane — each of a box's 8 corners is checked as a
/// zero-radius sphere via `sphere_vs_quarter_pipe` (exact for that single
/// point), and every corner that reports a contact contributes one to the
/// manifold.
///
/// Unlike `box_vs_plane` (where the analogous corner test is exact because
/// a plane's signed distance is *linear*), this technique's exactness for
/// *this* shape isn't a coincidence either, despite the curved surface:
/// `RB-PHYSICS-001-FR-032` set out to build a genuine GJK-based
/// convex-vs-curved-surface narrow phase specifically to close a
/// once-suspected gap here (a face resting flush against a shallow curve
/// under-detecting because none of its own corners individually register)
/// — and, in doing so, proved that gap doesn't actually exist for this
/// containment-style contact. A quarter-pipe's contact test is "is the
/// box's farthest point from `axis_point`/`axis_direction` at or beyond
/// `radius`" (see `sphere_vs_quarter_pipe`'s own doc comment for why it's
/// a *farthest*-point, containment question, not a nearest-point one);
/// distance-from-an-axis is a *convex* function of position, and the
/// maximum of a convex function over a convex polytope (the box) is
/// always attained at one of its extreme points — its 8 corners — never
/// in a face's interior. So corner-testing isn't approximating anything
/// here: it's computing the exact same maximum a full box-vs-cylinder
/// narrow phase (support mapping / GJK-EPA) would, just via simple
/// enumeration instead of an iterative solver. Confirmed both by this
/// argument and empirically (`RB-PHYSICS-001-FR-032`'s own verification
/// plan) before this doc comment was corrected — the previous wording
/// here claimed a real under-detection bug that further investigation
/// found to be unfounded, not a limitation this project chose to accept.
///
/// A genuinely different remaining approximation (not a detection bug):
/// when 2+ corners simultaneously violate the radius, each is resolved as
/// its own independent contact point (its own local radial normal),
/// rather than as a single unified manifold a full convex-vs-convex
/// narrow phase might produce — a manifold-*richness* question, not a
/// detection one, and out of this requirement's scope. Each surviving
/// corner's own world position is used as `point`, not the fillet-surface
/// point `sphere_vs_quarter_pipe` itself would compute, for the same
/// rel_pos/torque-accuracy reason `box_vs_plane`'s own doc comment
/// already gives.
fn box_vs_quarter_pipe(
    position: Vec3,
    orientation: Quat,
    half_extents: Vec3,
    pipe: &StaticQuarterPipe,
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
                if let Some(mut contact) = sphere_vs_quarter_pipe(world_corner, 0.0, pipe) {
                    contact.point = world_corner;
                    contacts.push(contact);
                }
            }
        }
    }
    contacts
}

/// Dispatches a contact test against a quarter-pipe fillet — a sphere (the
/// ball) via `sphere_vs_quarter_pipe`, a box (a car, since
/// `RB-PHYSICS-001-FR-027`) via `box_vs_quarter_pipe`'s per-corner test —
/// exact, not an approximation, for this containment-style contact (see
/// its own doc comment, corrected by `RB-PHYSICS-001-FR-032`'s
/// investigation).
pub fn contacts_vs_quarter_pipe(body: &RigidBody, pipe: &StaticQuarterPipe) -> Vec<Contact> {
    match body.shape {
        Shape::Sphere { radius } => sphere_vs_quarter_pipe(body.position, radius, pipe)
            .into_iter()
            .collect(),
        Shape::Box { half_extents } => {
            box_vs_quarter_pipe(body.position, body.orientation, half_extents, pipe)
        }
    }
}

/// Analytic sphere-vs-corner-fillet contact (`RB-PHYSICS-001-FR-023`): the
/// same "ride the concave inside" convention as `sphere_vs_quarter_pipe`,
/// generalized from a cylinder to a sphere — contact fires as the sphere's
/// surface approaches or crosses `fillet.radius` from inside `fillet`'s
/// own sphere, pushing back toward `fillet.center`.
fn sphere_vs_corner_fillet(
    position: Vec3,
    radius: f32,
    fillet: &StaticCornerFillet,
) -> Option<Contact> {
    let rel = position - fillet.center;
    let dist = rel.length();

    // The sphere center sitting exactly on the fillet's own center has no
    // well-defined direction to push along -- the same unlikely exact
    // singularity `sphere_vs_quarter_pipe` guards against.
    if dist < 1e-6 {
        return None;
    }
    let dir = rel * (1.0 / dist);

    // Only this fillet's own spherical triangle is governed by it --
    // outside that region, whichever edge fillet or flat plane actually
    // borders that direction takes over instead (see the struct's own doc
    // comment).
    if fillet.bounds.iter().any(|b| dir.dot(b) < 0.0) {
        return None;
    }

    let gap = (fillet.radius - radius) - dist;
    if gap > CONTACT_PROCESSING_THRESHOLD {
        return None;
    }

    let point = fillet.center + dir * fillet.radius;
    Some(Contact {
        normal: -dir,
        point,
        penetration_depth: -gap,
    })
}

/// Analytic box-vs-corner-fillet contact (`RB-PHYSICS-001-FR-027`): the
/// same corner-testing technique `box_vs_quarter_pipe` uses, generalized
/// from a cylinder to a sphere — each of a box's 8 corners is checked as a
/// zero-radius sphere via `sphere_vs_corner_fillet`, and every corner that
/// reports a contact contributes one to the manifold. Exact, not an
/// approximation, for the same reason `box_vs_quarter_pipe`'s own doc
/// comment gives (distance-from-`fillet.center` is convex, so its maximum
/// over the box is always at a corner) — see that comment for the full
/// `RB-PHYSICS-001-FR-032` investigation this generalizes from a line to
/// a point. Same world-position (not fillet-surface) `point` convention
/// for correct torque.
fn box_vs_corner_fillet(
    position: Vec3,
    orientation: Quat,
    half_extents: Vec3,
    fillet: &StaticCornerFillet,
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
                if let Some(mut contact) = sphere_vs_corner_fillet(world_corner, 0.0, fillet) {
                    contact.point = world_corner;
                    contacts.push(contact);
                }
            }
        }
    }
    contacts
}

/// Dispatches a contact test against a corner fillet — a sphere (the ball)
/// via `sphere_vs_corner_fillet`, a box (a car, since
/// `RB-PHYSICS-001-FR-027`) via `box_vs_corner_fillet`'s per-corner test —
/// exact, not an approximation (see its own doc comment).
pub fn contacts_vs_corner_fillet(body: &RigidBody, fillet: &StaticCornerFillet) -> Vec<Contact> {
    match body.shape {
        Shape::Sphere { radius } => sphere_vs_corner_fillet(body.position, radius, fillet)
            .into_iter()
            .collect(),
        Shape::Box { half_extents } => {
            box_vs_corner_fillet(body.position, body.orientation, half_extents, fillet)
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
///
/// Both cases confirmed against real
/// `btSphereBoxCollisionAlgorithm::getSphereDistance`/`getSpherePenetration`
/// (`RB-PHYSICS-001-FR-047`): the outside case's clamp-then-normalize is
/// the same closest-point construction, and the deep-penetration case's
/// per-axis-margin-then-sign selection below is confirmed to reproduce
/// `getSpherePenetration`'s own face-checking order exactly, not just a
/// mathematically-equivalent alternative — that function initializes to
/// the `+x` face and only overrides on a *strictly* smaller distance,
/// checking `+x, -x, +y, -y, +z, -z` in that order, so an exact tie always
/// resolves to whichever of those is checked earliest. Comparing per-axis
/// margins with `<=` (below) reproduces the same axis preference
/// (`x` over `y` over `z`), and `sign(local_center.<axis>) >= 0.0`
/// reproduces the same within-axis preference (`+` over `-`) — see
/// `sphere_embedded_at_an_axis_tie_prefers_the_lower_axis_like_bullets_own_face_check_order`
/// for a worked, non-symmetric case pinning this exactly. One numeric
/// difference found and not adopted: this function's outside/inside
/// branch threshold is `outside_distance > 1e-6` (linear), while real
/// Bullet's is `dist2 <= SIMD_EPSILON` (squared, ~1.19e-7 — a linear
/// distance of ~3.45e-4, roughly 2.5 orders of magnitude looser). Harmless
/// either way: the only consequence is which branch runs in an
/// astronomically narrow band right at the box's surface, and unlike a
/// quaternion normalize (`RB-PHYSICS-001-FR-045`), dividing by a small but
/// genuinely nonzero `outside_distance` here is numerically stable at any
/// magnitude a `f32` can represent.
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

/// Analytic sphere-vs-sphere contact — trivially closed-form, unlike either
/// shape pairing above: the closest points are always along the line
/// connecting the two centers, so this reduces to a single distance
/// comparison exactly like `sphere_vs_plane` does. `point` sits on `b`'s own
/// surface facing `a`, and `normal` points from `b` toward `a`, matching
/// `contacts_between`'s general convention. Needed since
/// `RB-PHYSICS-001-FR-033`, for the ball's contact against a `net::NetMesh`'s
/// free point masses (each a tiny sphere `RigidBody` — see `net.rs`'s own
/// doc comment for why a real net's mass-spring points reuse this crate's
/// existing sphere shape and two-body solver path rather than a bespoke
/// penalty-force system).
///
/// Confirmed byte-for-byte accurate against real
/// `btSphereSphereCollisionAlgorithm::processCollision`
/// (`RB-PHYSICS-001-FR-047`): `diff = posA - posB`,
/// `normalOnSurfaceB = diff / len`, `pos1 = posB + radius1 * normalOnSurfaceB`,
/// and `dist = len - (radius0 + radius1)` all match this function's
/// `delta`/`normal`/`point`/`gap` exactly. Two harmless, non-adopted
/// numeric differences: the degenerate-coincident-centers threshold here
/// is `dist > 1e-6` vs. real Bullet's `len > SIMD_EPSILON` (~1.19e-7 —
/// same one-order-of-magnitude-tighter pattern as `sphere_vs_box`, above,
/// and `integrate_transform`, `RB-PHYSICS-001-FR-045`), and the arbitrary
/// fallback direction for that unreachable-in-practice case is `(0, 0, 1)`
/// here vs. Bullet's own `(1, 0, 0)` default — an arbitrary choice on both
/// sides, so no behavioral divergence either way.
fn sphere_vs_sphere(pos_a: Vec3, radius_a: f32, pos_b: Vec3, radius_b: f32) -> Option<Contact> {
    let delta = pos_a - pos_b;
    let dist = delta.length();
    let gap = dist - radius_a - radius_b;

    if gap > CONTACT_PROCESSING_THRESHOLD {
        return None;
    }

    // Degenerate case (exactly coincident centers) picks an arbitrary
    // separating direction rather than dividing by zero — vanishingly
    // unlikely in practice (it would mean two sphere centers landed on
    // literally the same point), but still a well-defined result rather
    // than a NaN normal.
    let normal = if dist > 1e-6 {
        delta * (1.0 / dist)
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let point = pos_b + normal * radius_b;
    Some(Contact {
        normal,
        point,
        penetration_depth: -gap,
    })
}

fn get_axis(v: Vec3, index: usize) -> f32 {
    match index {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

fn set_axis(mut v: Vec3, index: usize, value: f32) -> Vec3 {
    match index {
        0 => v.x = value,
        1 => v.y = value,
        _ => v.z = value,
    }
    v
}

/// The two axis indices other than `axis_index`, in ascending order — the
/// two directions tangent to a box face perpendicular to `axis_index`.
fn tangent_indices(axis_index: usize) -> [usize; 2] {
    match axis_index {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    }
}

/// A box's three local axes, in world space, matching `box_vs_plane`'s and
/// `sphere_vs_box`'s convention of `half_extents.x/y/z` along the box's own
/// local X/Y/Z.
fn box_axes(orientation: &Quat) -> [Vec3; 3] {
    [
        orientation.rotate(&Vec3::new(1.0, 0.0, 0.0)),
        orientation.rotate(&Vec3::new(0.0, 1.0, 0.0)),
        orientation.rotate(&Vec3::new(0.0, 0.0, 1.0)),
    ]
}

/// The overlap of both boxes' projections onto `axis` (assumed normalized):
/// positive means they overlap by that much along this axis, negative means
/// `axis` is a genuine separating axis (the boxes don't collide). Port of
/// the per-axis test in `btBoxBoxDetector::dBoxBox`'s (ODE-derived)
/// separating-axis loop.
#[allow(clippy::too_many_arguments)]
fn axis_overlap(
    axis: &Vec3,
    axes_a: &[Vec3; 3],
    half_a: Vec3,
    axes_b: &[Vec3; 3],
    half_b: Vec3,
    center_diff: &Vec3,
) -> f32 {
    let dist = center_diff.dot(axis).abs();
    let radius_a: f32 = (0..3)
        .map(|k| (axes_a[k].dot(axis) * get_axis(half_a, k)).abs())
        .sum();
    let radius_b: f32 = (0..3)
        .map(|k| (axes_b[k].dot(axis) * get_axis(half_b, k)).abs())
        .sum();
    radius_a + radius_b - dist
}

/// Closest points between two line segments `p1`-`q1` and `p2`-`q2` — the
/// standard closest-point-between-segments construction (e.g. Ericson,
/// *Real-Time Collision Detection*, section 5.1.9), needed for an
/// edge-edge box contact's single point. `RB-PHYSICS-001-FR-042` confirmed
/// directly against `btBoxBoxDetector::dBoxBox`'s own real source that this
/// is strictly more rigorous than Bullet's own reference: `dBoxBox`'s
/// `dLineClosestApproach` computes closest approach between two *infinite
/// lines* and applies the resulting offsets with no clamping to the finite
/// edge length at all, while this function correctly stays within both
/// finite segments — a genuine improvement over the algorithm this port is
/// otherwise based on, not merely an equivalent restatement of it.
fn closest_points_on_segments(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(&d1);
    let e = d2.dot(&d2);
    let f = d2.dot(&r);

    if a <= 1e-8 && e <= 1e-8 {
        return (p1, p2);
    }

    let (s, t);
    if a <= 1e-8 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(&r);
        if e <= 1e-8 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(&d2);
            let denom = a * e - b * b;
            let mut s2 = if denom.abs() > 1e-8 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t2 = (b * s2 + f) / e;
            if t2 < 0.0 {
                t2 = 0.0;
                s2 = (-c / a).clamp(0.0, 1.0);
            } else if t2 > 1.0 {
                t2 = 1.0;
                s2 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s2;
            t = t2;
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// A face contact between two boxes: `ref_*` is the reference box (whose
/// face normal is the chosen separating axis) and `inc_*` the incident box.
/// Clips the incident box's face nearest to facing into the reference
/// box's face (found by whichever of its 6 face normals is most
/// anti-parallel to the reference face's) against the reference face's own
/// extent — a box-specific closed form of the general polygon clipping
/// `btBoxBoxDetector::dBoxBox` runs, since both faces are always
/// axis-aligned rectangles in their own box's local frame.
#[allow(clippy::too_many_arguments)]
fn face_contact(
    ref_pos: Vec3,
    ref_orient: Quat,
    ref_half: Vec3,
    ref_axis_index: usize,
    ref_sign: f32,
    inc_pos: Vec3,
    inc_orient: Quat,
    inc_half: Vec3,
    normal: Vec3,
    depth_hint: f32,
) -> Vec<Contact> {
    let ref_axes = box_axes(&ref_orient);
    let ref_face_normal_world = ref_axes[ref_axis_index] * ref_sign;
    let ref_tangents = tangent_indices(ref_axis_index);

    let inc_axes = box_axes(&inc_orient);
    let mut inc_axis_index = 0;
    let mut inc_sign = 1.0f32;
    let mut most_antiparallel = f32::INFINITY;
    for (k, &axis) in inc_axes.iter().enumerate() {
        for &s in &[1.0f32, -1.0] {
            let dot = (axis * s).dot(&ref_face_normal_world);
            if dot < most_antiparallel {
                most_antiparallel = dot;
                inc_axis_index = k;
                inc_sign = s;
            }
        }
    }
    let inc_tangents = tangent_indices(inc_axis_index);

    let mut corners = [Vec3::ZERO; 4];
    let mut n = 0;
    for &ta in &[-1.0f32, 1.0] {
        for &tb in &[-1.0f32, 1.0] {
            let mut local = Vec3::ZERO;
            local = set_axis(
                local,
                inc_axis_index,
                inc_sign * get_axis(inc_half, inc_axis_index),
            );
            local = set_axis(
                local,
                inc_tangents[0],
                ta * get_axis(inc_half, inc_tangents[0]),
            );
            local = set_axis(
                local,
                inc_tangents[1],
                tb * get_axis(inc_half, inc_tangents[1]),
            );
            corners[n] = inc_pos + inc_orient.rotate(&local);
            n += 1;
        }
    }

    let mut contacts = Vec::with_capacity(4);
    for corner in corners {
        let local = ref_orient.conjugate().rotate(&(corner - ref_pos));
        let t0 = get_axis(local, ref_tangents[0]);
        let t1 = get_axis(local, ref_tangents[1]);
        let limit0 = get_axis(ref_half, ref_tangents[0]) + CONTACT_PROCESSING_THRESHOLD;
        let limit1 = get_axis(ref_half, ref_tangents[1]) + CONTACT_PROCESSING_THRESHOLD;
        if t0.abs() > limit0 || t1.abs() > limit1 {
            continue;
        }
        let face_coord = get_axis(local, ref_axis_index) * ref_sign;
        let depth = get_axis(ref_half, ref_axis_index) - face_coord;
        if depth < -CONTACT_PROCESSING_THRESHOLD {
            continue;
        }
        let projected = set_axis(
            local,
            ref_axis_index,
            ref_sign * get_axis(ref_half, ref_axis_index),
        );
        contacts.push(Contact {
            normal,
            point: ref_pos + ref_orient.rotate(&projected),
            penetration_depth: depth.max(0.0),
        });
    }

    if contacts.is_empty() {
        // Safety net: SAT confirmed real overlap along this axis, but every
        // incident corner clipped outside the reference face's extent (a
        // grazing/marginal configuration) — report one contact at the
        // incident box's center, clamped onto the reference face, rather
        // than silently dropping a genuine collision. `RB-PHYSICS-001-FR-042`
        // confirmed directly against `btBoxBoxDetector::dBoxBox`'s own real
        // source that this branch's "shouldn't normally happen" framing
        // matches the reference author's own — it contains the exact same
        // undocumented judgment call, twice (after its own polygon-clipping
        // step and again after filtering to penetrating points), with zero
        // geometric proof given either time. Where this port deliberately
        // diverges is policy, not correctness: `dBoxBox`'s own fallback
        // there is `return 0` (drop the collision entirely), while this one
        // synthesizes a contact instead — since SAT has already confirmed
        // real geometric overlap by this point, silently dropping it risks
        // a body tunneling through in a rare grazing configuration.
        let local_center = ref_orient.conjugate().rotate(&(inc_pos - ref_pos));
        let mut clamped = local_center;
        for &k in &ref_tangents {
            clamped = set_axis(
                clamped,
                k,
                get_axis(local_center, k).clamp(-get_axis(ref_half, k), get_axis(ref_half, k)),
            );
        }
        clamped = set_axis(
            clamped,
            ref_axis_index,
            ref_sign * get_axis(ref_half, ref_axis_index),
        );
        contacts.push(Contact {
            normal,
            point: ref_pos + ref_orient.rotate(&clamped),
            penetration_depth: depth_hint.max(0.0),
        });
    }

    contacts
}

/// An edge-edge contact between two boxes: offsets each box's centerline
/// along axis `i`/`j` to the specific one of its 4 parallel edges nearest
/// the other box (by choosing the other two local axes' sign to move
/// toward it, using the center-to-center vector `d` as the "which side is
/// nearer" proxy), then finds the closest points between the resulting
/// finite segments. `RB-PHYSICS-001-FR-042` investigated swapping `d` for
/// the already-available SAT-resolved `normal` instead (matching
/// `btBoxBoxDetector::dBoxBox`'s own reference approach, which uses the
/// resolved collision-normal direction rather than a raw center-to-center
/// proxy) and empirically tested both against a brute-force ground truth
/// (all 16 sign combinations, minimum segment-to-segment distance) across
/// 50,000 randomized two-box configurations: neither heuristic reliably
/// picks the true nearest edge pair, and which one does better depends on
/// the regime — `d` wins for large/arbitrary penetration depths (~11.6% vs.
/// ~8.7% optimal-match rate), `normal` wins for realistic near-first-contact
/// depths under 0.5 units (~93% vs. ~77%), and both have occasional
/// individual outliers tens of units off the true optimum. Not adopted:
/// swapping one imperfect heuristic for a different imperfect one isn't a
/// justified change without real recorded car-vs-car contact data to know
/// which regime actually matters here. A genuinely rigorous fix would need
/// a non-heuristic nearest-edge-pair selection (e.g. the brute-force search
/// used only as this investigation's own throwaway ground-truth oracle),
/// left as a still-open item.
#[allow(clippy::too_many_arguments)]
fn edge_contact(
    pos_a: Vec3,
    axes_a: &[Vec3; 3],
    half_a: Vec3,
    i: usize,
    pos_b: Vec3,
    axes_b: &[Vec3; 3],
    half_b: Vec3,
    j: usize,
    normal: Vec3,
    depth: f32,
) -> Contact {
    let d = pos_b - pos_a;

    let mut edge_a_center = pos_a;
    for &k in &tangent_indices(i) {
        let sign = if axes_a[k].dot(&d) >= 0.0 { 1.0 } else { -1.0 };
        edge_a_center += axes_a[k] * (sign * get_axis(half_a, k));
    }
    let mut edge_b_center = pos_b;
    for &k in &tangent_indices(j) {
        let sign = if axes_b[k].dot(&d) <= 0.0 { 1.0 } else { -1.0 };
        edge_b_center += axes_b[k] * (sign * get_axis(half_b, k));
    }

    let (pa, pb) = closest_points_on_segments(
        edge_a_center - axes_a[i] * get_axis(half_a, i),
        edge_a_center + axes_a[i] * get_axis(half_a, i),
        edge_b_center - axes_b[j] * get_axis(half_b, j),
        edge_b_center + axes_b[j] * get_axis(half_b, j),
    );

    Contact {
        normal,
        point: (pa + pb) * 0.5,
        penetration_depth: depth.max(0.0),
    }
}

#[derive(Clone, Copy)]
enum SatFeature {
    FaceA(usize),
    FaceB(usize),
    Edge(usize, usize),
}

/// Separating-axis test between two oriented boxes, the same overall
/// structure as `btBoxBoxDetector::dBoxBox` (itself derived from ODE's
/// `dBoxBox`, which Bullet's implementation credits): test all 3 of A's
/// face axes, all 3 of B's, and all 9 `Ai × Bj` edge-pair axes; if every
/// one shows overlap, the minimum-overlap axis is the collision normal,
/// and its *kind* (face or edge) decides whether the contact is a clipped
/// face manifold (up to 4 points) or a single edge-edge point.
fn box_vs_box(
    pos_a: Vec3,
    orient_a: Quat,
    half_a: Vec3,
    pos_b: Vec3,
    orient_b: Quat,
    half_b: Vec3,
) -> Vec<Contact> {
    let axes_a = box_axes(&orient_a);
    let axes_b = box_axes(&orient_b);
    let d = pos_b - pos_a;

    let mut best_overlap = f32::INFINITY;
    let mut best_axis = Vec3::ZERO;
    let mut best_feature = SatFeature::FaceA(0);

    for i in 0..3 {
        let overlap = axis_overlap(&axes_a[i], &axes_a, half_a, &axes_b, half_b, &d);
        if overlap < 0.0 {
            return Vec::new();
        }
        if overlap < best_overlap {
            best_overlap = overlap;
            best_axis = axes_a[i];
            best_feature = SatFeature::FaceA(i);
        }
    }
    for j in 0..3 {
        let overlap = axis_overlap(&axes_b[j], &axes_a, half_a, &axes_b, half_b, &d);
        if overlap < 0.0 {
            return Vec::new();
        }
        if overlap < best_overlap {
            best_overlap = overlap;
            best_axis = axes_b[j];
            best_feature = SatFeature::FaceB(j);
        }
    }
    for i in 0..3 {
        for j in 0..3 {
            let raw = axes_a[i].cross(&axes_b[j]);
            let len = raw.length();
            if len < 1e-6 {
                // Near-parallel edges: this axis is numerically unstable
                // and, for two boxes, never the *only* separating axis
                // (a face axis always covers this configuration too), so
                // skipping it costs no correctness.
                continue;
            }
            let axis = raw * (1.0 / len);
            let overlap = axis_overlap(&axis, &axes_a, half_a, &axes_b, half_b, &d);
            if overlap < 0.0 {
                return Vec::new();
            }
            // Edge axes are noisier than face axes (a near-parallel pair
            // just barely above the skip threshold can slightly
            // under-report separation), so an edge axis only overrides a
            // face axis when it's a genuinely tighter fit, not by noise —
            // the same face-biased tie-break `dBoxBox`-style detectors use.
            if overlap < best_overlap - 1e-4 {
                best_overlap = overlap;
                best_axis = axis;
                best_feature = SatFeature::Edge(i, j);
            }
        }
    }

    // Sign-fix so `normal` points from B toward A, matching
    // `contacts_between`'s convention.
    let normal = if best_axis.dot(&d) > 0.0 {
        -best_axis
    } else {
        best_axis
    };

    match best_feature {
        SatFeature::FaceA(i) => {
            let sign = if axes_a[i].dot(&d) >= 0.0 { 1.0 } else { -1.0 };
            face_contact(
                pos_a,
                orient_a,
                half_a,
                i,
                sign,
                pos_b,
                orient_b,
                half_b,
                normal,
                best_overlap,
            )
        }
        SatFeature::FaceB(j) => {
            let sign = if axes_b[j].dot(&(-d)) >= 0.0 {
                1.0
            } else {
                -1.0
            };
            face_contact(
                pos_b,
                orient_b,
                half_b,
                j,
                sign,
                pos_a,
                orient_a,
                half_a,
                normal,
                best_overlap,
            )
        }
        SatFeature::Edge(i, j) => vec![edge_contact(
            pos_a,
            &axes_a,
            half_a,
            i,
            pos_b,
            &axes_b,
            half_b,
            j,
            normal,
            best_overlap,
        )],
    }
}

/// Dispatches a contact test between two dynamic bodies, covering every
/// shape pairing this crate has: sphere-vs-box (the ball-vs-car case,
/// always 0 or 1 points) and box-vs-box (the car-vs-car case, 0 to 4 points
/// for a face contact or 1 for an edge contact — see `box_vs_box`).
/// `normal` always points from `b` toward `a` (matching `contacts_vs_plane`'s
/// convention with `b` playing the plane's "reference" role), so the
/// two-body solver can apply `+impulse` to `a` and `-impulse` to `b` along
/// it without needing to know which argument was which shape.
///
/// Sphere-vs-sphere (`sphere_vs_sphere`, since `RB-PHYSICS-001-FR-033`) is
/// implemented too — needed for the ball's contact against a
/// `net::NetMesh`'s free point masses, each represented as a tiny sphere
/// `RigidBody` rather than this scope's one real ball; two actual balls
/// never collide in this port, but the shape pairing itself is real now.
pub fn contacts_between(a: &RigidBody, b: &RigidBody) -> Vec<Contact> {
    match (a.shape, b.shape) {
        (Shape::Sphere { radius }, Shape::Box { half_extents }) => {
            sphere_vs_box(a.position, radius, b.position, b.orientation, half_extents)
                .into_iter()
                .collect()
        }
        (Shape::Box { half_extents }, Shape::Sphere { radius }) => {
            sphere_vs_box(b.position, radius, a.position, a.orientation, half_extents)
                .map(|c| Contact {
                    normal: -c.normal,
                    ..c
                })
                .into_iter()
                .collect()
        }
        (Shape::Sphere { radius: radius_a }, Shape::Sphere { radius: radius_b }) => {
            sphere_vs_sphere(a.position, radius_a, b.position, radius_b)
                .into_iter()
                .collect()
        }
        (
            Shape::Box {
                half_extents: half_a,
            },
            Shape::Box {
                half_extents: half_b,
            },
        ) => box_vs_box(
            a.position,
            a.orientation,
            half_a,
            b.position,
            b.orientation,
            half_b,
        ),
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
        let ball = RigidBody::sphere(93.15, 1.0, Vec3::new(1000.0, 0.0, 0.0));
        assert!(contacts_between(&ball, &stationary_car()).is_empty());
    }

    #[test]
    fn ball_touching_car_face_has_zero_penetration() {
        // The ball's surface exactly meets the car's +X face.
        let ball = RigidBody::sphere(93.15, 1.0, Vec3::new(60.0 + 93.15, 0.0, 0.0));
        let contacts = contacts_between(&ball, &stationary_car());
        assert_eq!(contacts.len(), 1);
        assert!(contacts[0].penetration_depth.abs() < 1e-4);
        assert!((contacts[0].normal - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn ball_overlapping_car_face_has_positive_penetration() {
        let ball = RigidBody::sphere(93.15, 1.0, Vec3::new(60.0 + 50.0, 0.0, 0.0));
        let contacts = contacts_between(&ball, &stationary_car());
        assert!((contacts[0].penetration_depth - (93.15 - 50.0)).abs() < 1e-4);
    }

    #[test]
    fn ball_center_embedded_in_car_pushes_out_the_nearest_face() {
        // Ball center sits inside the car box, closest to the +Z (roof)
        // face (margin 2.0) rather than +X (margin 40.0) or +Y (margin
        // 10.0) — the deep-penetration branch must pick +Z.
        let ball = RigidBody::sphere(5.0, 1.0, Vec3::new(20.0, 20.0, 16.0));
        let contacts = contacts_between(&ball, &stationary_car());
        assert!((contacts[0].normal - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-5);
        assert!(contacts[0].penetration_depth > 0.0);
    }

    #[test]
    fn sphere_embedded_at_an_axis_tie_prefers_the_lower_axis_like_bullets_own_face_check_order() {
        // RB-PHYSICS-001-FR-047: half_extents/position chosen so the box's
        // own -x face and +y face are *exactly* tied at margin 3.0 (the z
        // faces sit at margin 100.0, never in contention). Real Bullet's
        // `getSpherePenetration` checks faces in a fixed
        // +x, -x, +y, -y, +z, -z order, only overriding its running
        // minimum on a *strictly* smaller distance — so on this exact tie
        // it settles on -x (checked before +y). A naive "first satisfying
        // axis" scan in a different order, or one that didn't also
        // prioritize the correct sign within an axis, could just as
        // plausibly have picked +y here instead.
        let car = RigidBody::car_box(Vec3::new(5.0, 5.0, 100.0), 180.0, Vec3::ZERO);
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(-2.0, 2.0, 0.0));
        let contacts = contacts_between(&ball, &car);
        assert_eq!(contacts.len(), 1);
        assert!((contacts[0].normal - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5);
        // depth = margin (3.0) + radius (1.0): see the deep-penetration
        // branch's own `(point, normal, -depth - radius)` gap construction.
        assert!((contacts[0].penetration_depth - 4.0).abs() < 1e-5);
    }

    #[test]
    fn sphere_vs_box_contact_is_antisymmetric_in_argument_order() {
        let ball = RigidBody::sphere(93.15, 1.0, Vec3::new(60.0 + 50.0, 0.0, 0.0));
        let car = stationary_car();
        let ball_car = &contacts_between(&ball, &car)[0];
        let car_ball = &contacts_between(&car, &ball)[0];
        assert!((ball_car.normal + car_ball.normal).length() < 1e-5);
        assert!((ball_car.penetration_depth - car_ball.penetration_depth).abs() < 1e-4);
    }

    #[test]
    fn overlapping_spheres_produce_a_contact_pointing_from_b_toward_a() {
        // RB-PHYSICS-001-FR-033: sphere-vs-sphere is a real shape pairing
        // now (needed for the ball's contact against a `net::NetMesh`'s own
        // point masses), replacing the old "always empty" placeholder this
        // codebase carried while it had no caller for it at all.
        let a = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        let b = RigidBody::sphere(1.0, 1.0, Vec3::new(0.5, 0.0, 0.0));
        let contacts = contacts_between(&a, &b);
        assert_eq!(contacts.len(), 1);
        assert!((contacts[0].normal - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5);
        assert!((contacts[0].penetration_depth - 1.5).abs() < 1e-5);
    }

    #[test]
    fn far_apart_spheres_have_no_contact() {
        let a = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        let b = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 0.0));
        assert!(contacts_between(&a, &b).is_empty());
    }

    #[test]
    fn boxes_far_apart_have_no_contact() {
        let a = stationary_car();
        let b = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(1000.0, 0.0, 0.0));
        assert!(contacts_between(&a, &b).is_empty());
    }

    #[test]
    fn boxes_overlapping_face_to_face_have_a_four_point_manifold() {
        // Two boxes overlapping symmetrically along X: A's +X face pushes
        // into B's -X face, both large enough that the full overlap
        // rectangle survives clipping — the classic 4-point flat contact.
        let a = RigidBody::car_box(Vec3::new(10.0, 10.0, 10.0), 1.0, Vec3::ZERO);
        let b = RigidBody::car_box(
            Vec3::new(10.0, 10.0, 10.0),
            1.0,
            Vec3::new(15.0, 0.0, 0.0), // 5 units of overlap along X
        );
        let contacts = contacts_between(&a, &b);
        assert_eq!(contacts.len(), 4);
        for c in &contacts {
            assert!((c.penetration_depth - 5.0).abs() < 1e-3);
            // Normal points from b toward a, i.e. -X here.
            assert!((c.normal - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5);
        }
    }

    #[test]
    fn box_vs_box_contact_is_antisymmetric_in_argument_order() {
        let a = RigidBody::car_box(Vec3::new(10.0, 10.0, 10.0), 1.0, Vec3::ZERO);
        let b = RigidBody::car_box(Vec3::new(10.0, 10.0, 10.0), 1.0, Vec3::new(15.0, 0.0, 0.0));
        let ab = contacts_between(&a, &b);
        let ba = contacts_between(&b, &a);
        assert_eq!(ab.len(), ba.len());
        assert!((ab[0].normal + ba[0].normal).length() < 1e-5);
        assert!((ab[0].penetration_depth - ba[0].penetration_depth).abs() < 1e-3);
    }

    #[test]
    fn box_rotated_forty_five_degrees_pokes_into_a_face_with_fewer_than_four_contacts() {
        // B (a diamond in cross-section, rotated 45 degrees about Z) pokes
        // into A's +X region along an edge or corner rather than flat
        // face-to-face — depending on which axis SAT picks (a face axis or
        // one of the edge-edge axes), clipping degenerates to fewer than
        // the flat case's 4 points. Not pinning an exact axis or count
        // here (the rotated geometry's minimum-penetration axis isn't
        // obvious by hand) — just that a real, positive-depth,
        // unit-normal, B-to-A-pointing contact comes out, and it's not the
        // full flat manifold.
        let a = RigidBody::car_box(Vec3::new(50.0, 50.0, 50.0), 1.0, Vec3::ZERO);
        let half = std::f32::consts::FRAC_PI_8;
        let mut b = RigidBody::car_box(
            Vec3::new(10.0, 10.0, 10.0),
            1.0,
            Vec3::new(50.0 + 10.0, 0.0, 0.0),
        );
        b.orientation = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let contacts = contacts_between(&a, &b);
        assert!(
            !contacts.is_empty() && contacts.len() < 4,
            "expected a partial (non-flat) manifold, got {contacts:?}"
        );
        for c in &contacts {
            assert!(c.penetration_depth > 0.0);
            assert!((c.normal.length() - 1.0).abs() < 1e-4);
            assert!(
                c.normal.x < 0.0,
                "expected the normal to point from b toward a, i.e. roughly -X here, got {:?}",
                c.normal
            );
        }
    }

    /// A floor (z=0) meeting a +X side wall at x=100, fillet radius 20 —
    /// the axis sits at (80, y, 20), matching `body::tests::
    /// quarter_pipe_axis_sits_radius_units_in_from_both_planes`.
    fn floor_wall_pipe() -> StaticQuarterPipe {
        let floor = StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0);
        let wall = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -100.0);
        crate::body::StaticQuarterPipe::between_planes(
            &floor,
            &wall,
            20.0,
            Vec3::new(0.0, 1.0, 0.0),
        )
    }

    #[test]
    fn sphere_deep_inside_the_pipe_has_no_contact() {
        // Sitting right on the axis's own perpendicular-plane origin plus
        // a tiny nudge (avoiding the exact-on-axis singularity) is deep
        // inside a 20-unit-radius pipe for a 1-unit sphere.
        let pipe = floor_wall_pipe();
        let s = RigidBody::sphere(1.0, 1.0, pipe.axis_point + Vec3::new(0.0, 0.0, 0.1));
        assert!(contacts_vs_quarter_pipe(&s, &pipe).is_empty());
    }

    #[test]
    fn sphere_touching_the_pipe_surface_has_zero_penetration() {
        let pipe = floor_wall_pipe();
        let radius = 1.0;
        // Exactly radius+sphere_radius from the axis, along the sector
        // bisector (halfway between sector_start and sector_end) — well
        // inside the 90-degree sector.
        let bisector = ((pipe.sector_start + pipe.sector_end) * 0.5)
            .normalize()
            .unwrap();
        let position = pipe.axis_point + bisector * (pipe.radius - radius);
        let s = RigidBody::sphere(radius, 1.0, position);
        let contacts = contacts_vs_quarter_pipe(&s, &pipe);
        assert_eq!(contacts.len(), 1);
        assert!(contacts[0].penetration_depth.abs() < 1e-4);
    }

    #[test]
    fn sphere_pushed_past_the_pipe_surface_has_positive_penetration_pushing_toward_the_axis() {
        let pipe = floor_wall_pipe();
        let radius = 1.0;
        let bisector = ((pipe.sector_start + pipe.sector_end) * 0.5)
            .normalize()
            .unwrap();
        // 5 units further out than the resting distance -- overlapping the
        // fillet's own material by 5 units.
        let position = pipe.axis_point + bisector * (pipe.radius - radius + 5.0);
        let s = RigidBody::sphere(radius, 1.0, position);
        let contacts = contacts_vs_quarter_pipe(&s, &pipe);
        assert_eq!(contacts.len(), 1);
        assert!((contacts[0].penetration_depth - 5.0).abs() < 1e-4);
        // Correction pushes back toward the axis, i.e. opposite the
        // bisector direction (unlike a flat plane, which always pushes
        // away from the plane).
        assert!((contacts[0].normal + bisector).length() < 1e-4);
    }

    #[test]
    fn sphere_outside_the_pipes_sector_has_no_contact() {
        // Directly "above" the axis, i.e. deep into the room along the
        // diagonal away from the corner -- outside the 90-degree sector
        // even though it might be close in absolute distance.
        let pipe = floor_wall_pipe();
        let away_from_corner = -((pipe.sector_start + pipe.sector_end) * 0.5)
            .normalize()
            .unwrap();
        let s = RigidBody::sphere(1.0, 1.0, pipe.axis_point + away_from_corner * pipe.radius);
        assert!(contacts_vs_quarter_pipe(&s, &pipe).is_empty());
    }

    #[test]
    fn box_embedded_in_the_quarter_pipes_footprint_has_contact() {
        // The real proof of RB-PHYSICS-001-FR-027: a car's own box is no
        // longer always empty against a curved fillet. Centering the car
        // directly on the pipe's own surface (along its sector bisector)
        // pushes at least one of its 8 corners past the curve into what
        // used to be untouchable material.
        let pipe = floor_wall_pipe();
        let bisector = ((pipe.sector_start + pipe.sector_end) * 0.5)
            .normalize()
            .unwrap();
        let deeply_overlapping_position = pipe.axis_point + bisector * pipe.radius;
        let car = RigidBody::car_box(
            Vec3::new(60.0, 30.0, 18.0),
            180.0,
            deeply_overlapping_position,
        );
        let contacts = contacts_vs_quarter_pipe(&car, &pipe);
        assert!(
            !contacts.is_empty(),
            "expected at least one of the car's 8 corners to be embedded in the pipe's footprint"
        );
        for contact in &contacts {
            // Every contact should push back toward the axis (this
            // crate's universal "ride the concave inside" convention --
            // see sphere_vs_quarter_pipe's own doc comment), not away
            // from it.
            let rel = contact.point - pipe.axis_point;
            let along_axis = rel.dot(&pipe.axis_direction);
            let radial = rel - pipe.axis_direction * along_axis;
            assert!(radial.dot(&contact.normal) < 0.0);
        }
    }

    #[test]
    fn box_far_from_the_quarter_pipe_has_no_contact() {
        // Placed deep on the *opposite* side of the sector from the pipe's
        // own wedge (the room's ordinary interior, not the rounded
        // corner) -- clearly outside the sector regardless of distance,
        // unlike moving further along the sector's own bisector (still
        // "inside" the wedge angularly at any radius, which isn't the
        // "far away and clearly unaffected" case this test wants).
        let pipe = floor_wall_pipe();
        let bisector = ((pipe.sector_start + pipe.sector_end) * 0.5)
            .normalize()
            .unwrap();
        let car = RigidBody::car_box(
            Vec3::new(60.0, 30.0, 18.0),
            180.0,
            pipe.axis_point - bisector * 1000.0,
        );
        assert!(contacts_vs_quarter_pipe(&car, &pipe).is_empty());
    }

    #[test]
    fn no_point_on_a_boxs_face_is_ever_farther_from_a_quarter_pipes_axis_than_its_own_corners() {
        // RB-PHYSICS-001-FR-032's own investigation, made concrete: a
        // once-suspected bug claimed a box's flat face resting against a
        // *shallow* (large-radius) curve could have its own middle overlap
        // the fillet while all 8 corners stayed clear, under-detecting the
        // contact. That's mathematically impossible for this shape —
        // `box_vs_quarter_pipe`'s contact question is "is the box's
        // farthest point from the axis line at or beyond `radius`", and
        // distance-from-a-line is a *convex* function of position, whose
        // maximum over a convex polytope (the box) is always attained at
        // one of its extreme points (corners), never a face's interior.
        // This test proves that concretely rather than just arguing it:
        // for a large-radius pipe and a car positioned exactly the way
        // FR-032's own investigation found (resting flat on the floor,
        // close enough to the wall that its corners straddle the curve),
        // every densely-sampled point across each of the box's 6 faces has
        // a distance-from-axis no greater than the box's own 8 corners'
        // maximum — corner-testing already finds the true worst case.
        let floor = StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0);
        let wall = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -1000.0);
        let pipe = crate::body::StaticQuarterPipe::between_planes(
            &floor,
            &wall,
            292.0,
            Vec3::new(0.0, 1.0, 0.0),
        );

        let half_extents = Vec3::new(60.0, 30.0, 18.0);
        let position = Vec3::new(900.0, 0.0, half_extents.z);

        let dist_from_axis = |p: Vec3| -> f32 {
            let rel = p - pipe.axis_point;
            let along = rel.dot(&pipe.axis_direction);
            (rel - pipe.axis_direction * along).length()
        };

        let corners: Vec<Vec3> = [-1.0f32, 1.0]
            .iter()
            .flat_map(|&sx| {
                [-1.0f32, 1.0].iter().flat_map(move |&sy| {
                    [-1.0f32, 1.0].iter().map(move |&sz| {
                        position
                            + Vec3::new(
                                sx * half_extents.x,
                                sy * half_extents.y,
                                sz * half_extents.z,
                            )
                    })
                })
            })
            .collect();
        let corner_max = corners
            .iter()
            .map(|&c| dist_from_axis(c))
            .fold(f32::MIN, f32::max);

        // Densely sample every face's own interior on a fine grid (not
        // just its 4 corners), covering all 6 faces.
        const STEPS: i32 = 50;
        let lerp = |lo: f32, hi: f32, t: f32| lo + (hi - lo) * t;
        let mut face_sample_max = f32::MIN;
        for i in 0..=STEPS {
            for j in 0..=STEPS {
                let u = i as f32 / STEPS as f32;
                let v = j as f32 / STEPS as f32;
                let a = lerp(-half_extents.x, half_extents.x, u);
                let b = lerp(-half_extents.y, half_extents.y, v);
                let c = lerp(-half_extents.z, half_extents.z, u);
                let samples = [
                    position + Vec3::new(a, b, -half_extents.z), // z- face
                    position + Vec3::new(a, b, half_extents.z),  // z+ face
                    position + Vec3::new(a, -half_extents.y, c), // y- face
                    position + Vec3::new(a, half_extents.y, c),  // y+ face
                    position + Vec3::new(-half_extents.x, b, c), // x- face
                    position + Vec3::new(half_extents.x, b, c),  // x+ face
                ];
                for s in samples {
                    face_sample_max = face_sample_max.max(dist_from_axis(s));
                }
            }
        }

        assert!(
            face_sample_max <= corner_max + 1e-3,
            "expected no face-interior point to exceed the corners' own maximum distance from \
             the axis, corner_max={corner_max}, face_sample_max={face_sample_max}"
        );
    }

    /// A floor (z=0) meeting a +X side wall at x=100 and a +Y back wall at
    /// y=100, fillet radius 20 -- the compound corner where all three
    /// pairwise edge fillets (each built the way `floor_wall_pipe` builds
    /// one of them) would otherwise meet at a single sharp point. The
    /// center sits radius-in from all three planes, at (80, 80, 20).
    fn floor_two_walls_corner() -> StaticCornerFillet {
        let floor = StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0);
        let wall_x = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -100.0);
        let wall_y = StaticPlane::new(Vec3::new(0.0, -1.0, 0.0), -100.0);
        StaticCornerFillet::between_three_planes(&floor, &wall_x, &wall_y, 20.0)
    }

    #[test]
    fn sphere_deep_inside_the_corner_fillet_has_no_contact() {
        // Sitting right on the fillet's own center plus a tiny nudge
        // (avoiding the exact-on-center singularity) is deep inside a
        // 20-unit-radius fillet for a 1-unit sphere.
        let fillet = floor_two_walls_corner();
        let s = RigidBody::sphere(1.0, 1.0, fillet.center + Vec3::new(0.0, 0.0, 0.1));
        assert!(contacts_vs_corner_fillet(&s, &fillet).is_empty());
    }

    #[test]
    fn sphere_touching_the_corner_fillet_surface_has_zero_penetration() {
        let fillet = floor_two_walls_corner();
        let radius = 1.0;
        // Exactly radius+sphere_radius from the center, toward the sharp
        // corner (100, 100, 0) that this fillet replaces -- well inside
        // the fillet's spherical-triangle bounds.
        let toward_corner = Vec3::new(1.0, 1.0, -1.0).normalize().unwrap();
        let position = fillet.center + toward_corner * (fillet.radius - radius);
        let s = RigidBody::sphere(radius, 1.0, position);
        let contacts = contacts_vs_corner_fillet(&s, &fillet);
        assert_eq!(contacts.len(), 1);
        assert!(contacts[0].penetration_depth.abs() < 1e-4);
    }

    #[test]
    fn sphere_pushed_past_the_corner_fillet_surface_has_positive_penetration_pushing_toward_the_center(
    ) {
        let fillet = floor_two_walls_corner();
        let radius = 1.0;
        let toward_corner = Vec3::new(1.0, 1.0, -1.0).normalize().unwrap();
        // 5 units further out than the resting distance -- overlapping the
        // fillet's own material by 5 units.
        let position = fillet.center + toward_corner * (fillet.radius - radius + 5.0);
        let s = RigidBody::sphere(radius, 1.0, position);
        let contacts = contacts_vs_corner_fillet(&s, &fillet);
        assert_eq!(contacts.len(), 1);
        assert!((contacts[0].penetration_depth - 5.0).abs() < 1e-4);
        // Correction pushes back toward the center, i.e. opposite the
        // corner-ward direction (unlike a flat plane, which always pushes
        // away from the plane).
        assert!((contacts[0].normal + toward_corner).length() < 1e-4);
    }

    #[test]
    fn sphere_outside_the_corner_fillets_bounds_has_no_contact() {
        // Directly away from the sharp corner this fillet replaces --
        // outside the spherical-triangle bounds even though it might be
        // close in absolute distance to the center.
        let fillet = floor_two_walls_corner();
        let away_from_corner = -Vec3::new(1.0, 1.0, -1.0).normalize().unwrap();
        let s = RigidBody::sphere(1.0, 1.0, fillet.center + away_from_corner * fillet.radius);
        assert!(contacts_vs_corner_fillet(&s, &fillet).is_empty());
    }

    #[test]
    fn box_embedded_in_the_corner_fillets_footprint_has_contact() {
        // The real proof of RB-PHYSICS-001-FR-027: a car's own box is no
        // longer always empty against a compound-corner fillet either.
        // Centering the car directly on the fillet's own surface (toward
        // the sharp corner it replaces) pushes at least one of its 8
        // corners past the sphere into what used to be untouchable
        // material.
        let fillet = floor_two_walls_corner();
        let toward_corner = Vec3::new(1.0, 1.0, -1.0).normalize().unwrap();
        let deeply_overlapping_position = fillet.center + toward_corner * fillet.radius;
        let car = RigidBody::car_box(
            Vec3::new(60.0, 30.0, 18.0),
            180.0,
            deeply_overlapping_position,
        );
        let contacts = contacts_vs_corner_fillet(&car, &fillet);
        assert!(
            !contacts.is_empty(),
            "expected at least one of the car's 8 corners to be embedded in the fillet's footprint"
        );
        for contact in &contacts {
            // Every contact should push back toward the fillet's own
            // center, the same "ride the concave inside" convention
            // `sphere_vs_corner_fillet` already documents.
            let rel = contact.point - fillet.center;
            assert!(rel.dot(&contact.normal) < 0.0);
        }
    }

    #[test]
    fn box_outside_the_corner_fillets_bounds_has_no_contact() {
        // Same "directly away from the sharp corner" direction
        // `sphere_outside_the_corner_fillets_bounds_has_no_contact` uses --
        // clearly outside the spherical-triangle bounds regardless of
        // distance, unlike moving further toward the corner (still
        // "inside" the bounds at any radius).
        let fillet = floor_two_walls_corner();
        let away_from_corner = -Vec3::new(1.0, 1.0, -1.0).normalize().unwrap();
        let car = RigidBody::car_box(
            Vec3::new(60.0, 30.0, 18.0),
            180.0,
            fillet.center + away_from_corner * 1000.0,
        );
        assert!(contacts_vs_corner_fillet(&car, &fillet).is_empty());
    }

    /// A back wall at y=100 (normal (0,-1,0)), with a 20-wide, 30-tall
    /// goal-mouth window centered at (0, 100, 30) -- the same fixture
    /// `body::tests::goal_wall_with_window` uses.
    fn goal_wall() -> StaticGoalWall {
        let plane = StaticPlane::new(Vec3::new(0.0, -1.0, 0.0), -100.0);
        StaticGoalWall::new(
            plane,
            Vec3::new(0.0, 100.0, 30.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            20.0,
            30.0,
        )
    }

    #[test]
    fn sphere_embedded_in_the_goal_window_has_no_contact() {
        let wall = goal_wall();
        let s = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 100.5, 30.0));
        assert!(contacts_vs_goal_wall(&s, &wall).is_empty());
    }

    #[test]
    fn sphere_outside_the_goal_window_behaves_like_an_ordinary_plane() {
        let wall = goal_wall();
        let s = RigidBody::sphere(1.0, 1.0, Vec3::new(25.0, 99.5, 30.0));
        let contacts = contacts_vs_goal_wall(&s, &wall);
        assert_eq!(contacts.len(), 1);
        assert!((contacts[0].penetration_depth - 0.5).abs() < 1e-5);
        assert_eq!(contacts[0].normal, wall.plane.normal);
    }

    #[test]
    fn sphere_resting_exactly_on_the_wall_outside_the_window_has_zero_penetration() {
        let wall = goal_wall();
        let s = RigidBody::sphere(1.0, 1.0, Vec3::new(25.0, 99.0, 30.0));
        let contacts = contacts_vs_goal_wall(&s, &wall);
        assert_eq!(contacts.len(), 1);
        assert!(contacts[0].penetration_depth.abs() < 1e-6);
    }

    #[test]
    fn box_squarely_inside_the_goal_window_has_no_contact() {
        // A car small enough that every corner falls inside the window
        // sails straight through, the box equivalent of
        // `sphere_embedded_in_the_goal_window_has_no_contact` --
        // `RB-PHYSICS-001-FR-028` closes the Non-goal `box_vs_goal_wall`'s
        // own doc comment used to describe here.
        let wall = goal_wall();
        let car = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(0.0, 99.5, 30.0));
        assert!(contacts_vs_goal_wall(&car, &wall).is_empty());
    }

    #[test]
    fn box_straddling_the_goal_window_edge_only_collides_on_the_corners_still_outside_it() {
        // A car centered on the window's own right edge (x=20) has half
        // its corners (x=21) outside the window and half (x=19) inside --
        // exactly the "some corners collide, some don't" partial block
        // `box_vs_goal_wall`'s own doc comment describes, unlike a sphere's
        // single all-or-nothing center-point test.
        let wall = goal_wall();
        let car = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(20.0, 99.5, 30.0));
        let contacts = contacts_vs_goal_wall(&car, &wall);
        assert_eq!(
            contacts.len(),
            2,
            "only the two outside-the-window corners on the x=21 face should register a contact"
        );
        for contact in &contacts {
            assert!(contact.point.x > 20.0);
        }
    }

    #[test]
    fn box_entirely_outside_the_goal_window_behaves_like_an_ordinary_plane() {
        // A car nowhere near the window (x=60, well past the 20-wide
        // window's own edge) collides exactly like plain `box_vs_plane`
        // against the wrapped plane -- the box equivalent of
        // `sphere_outside_the_goal_window_behaves_like_an_ordinary_plane`.
        let wall = goal_wall();
        let car = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(60.0, 99.5, 30.0));
        let windowed = contacts_vs_goal_wall(&car, &wall);
        let unwindowed = contacts_vs_plane(&car, &wall.plane);
        assert_eq!(windowed, unwindowed);
        assert!(!windowed.is_empty());
    }

    /// A wall at x=20 (normal (-1,0,0)), bounded to a 10-wide (y), 30-tall
    /// (z) rectangle centered at (20, 110, 30) -- the same fixture
    /// `body::tests::bounded_wall` uses.
    fn bounded_wall() -> StaticBoundedWall {
        let plane = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -20.0);
        StaticBoundedWall::new(
            plane,
            Vec3::new(20.0, 110.0, 30.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            10.0,
            30.0,
        )
    }

    #[test]
    fn sphere_inside_the_bound_behaves_like_an_ordinary_plane() {
        let wall = bounded_wall();
        let s = RigidBody::sphere(1.0, 1.0, Vec3::new(19.5, 110.0, 30.0));
        let contacts = contacts_vs_bounded_wall(&s, &wall);
        assert_eq!(contacts.len(), 1);
        assert!((contacts[0].penetration_depth - 0.5).abs() < 1e-5);
        assert_eq!(contacts[0].normal, wall.plane.normal);
    }

    #[test]
    fn sphere_outside_the_bound_has_no_contact() {
        // Same position relative to the plane (0.5 units embedded) as the
        // test above, but outside the bound's own y range -- would collide
        // against a plain, unbounded `StaticPlane`, but not here.
        let wall = bounded_wall();
        let s = RigidBody::sphere(1.0, 1.0, Vec3::new(19.5, 130.0, 30.0));
        assert!(contacts_vs_bounded_wall(&s, &wall).is_empty());
    }

    #[test]
    fn box_squarely_inside_the_bound_behaves_like_an_ordinary_plane() {
        let wall = bounded_wall();
        let car = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(19.5, 110.0, 30.0));
        let bounded = contacts_vs_bounded_wall(&car, &wall);
        let unbounded = contacts_vs_plane(&car, &wall.plane);
        assert_eq!(bounded, unbounded);
        assert!(!bounded.is_empty());
    }

    #[test]
    fn box_straddling_the_bounds_edge_only_collides_on_the_corners_still_inside_it() {
        // A car centered exactly on the bound's own right edge (y=120) has
        // half its corners (y=121) outside the bound and half (y=119)
        // inside -- the opposite gate from `box_vs_goal_wall`'s own
        // straddling test, but the same partial-collision shape.
        let wall = bounded_wall();
        let car = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(19.5, 120.0, 30.0));
        let contacts = contacts_vs_bounded_wall(&car, &wall);
        assert_eq!(
            contacts.len(),
            2,
            "only the two inside-the-bound corners on the y=119 face should register a contact"
        );
        for contact in &contacts {
            assert!(contact.point.y < 120.0);
        }
    }

    #[test]
    fn box_entirely_outside_the_bound_has_no_contact() {
        let wall = bounded_wall();
        let car = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(19.5, 200.0, 30.0));
        assert!(contacts_vs_bounded_wall(&car, &wall).is_empty());
    }
}
