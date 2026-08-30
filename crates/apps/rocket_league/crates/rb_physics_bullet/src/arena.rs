//! Rocket League's real standard-arena field dimensions
//! (`RB-PHYSICS-001-FR-019`), and a constructor for its octagonal
//! footprint plus a ceiling — built entirely from the existing generic
//! `StaticPlane`/`PhysicsWorld::with_wall` machinery `RB-PHYSICS-001-FR-013`
//! already provides. No new collision code: a ceiling and a corner-cut wall
//! are each just another flat `StaticPlane`, the same as the ground or a
//! side wall.
//!
//! `standard_curves` (`RB-PHYSICS-001-FR-020`/`FR-021`) adds curved
//! wall-to-floor/wall-to-ceiling fillets for all 9 walls — the 4 cardinal
//! (axis-aligned) walls and, since FR-021, the 4 diagonal corner walls too
//! — built from `body::StaticQuarterPipe::between_planes` — still no new
//! collision code of its own here, just composing the same flat planes
//! `standard_walls` already builds. A vertical wall is always perpendicular
//! to the floor/ceiling regardless of its own horizontal rotation, so
//! `between_planes` needs no generalization to cover a diagonal corner
//! wall's floor/ceiling seam — only `axis_direction` (computed via a cross
//! product here, rather than hand-picked, since a corner wall's own
//! "along the wall" direction isn't a coordinate axis) differs from the
//! cardinal-wall case.
//!
//! **Still not modeled**: a curved fillet at the vertical edges themselves
//! — where a corner wall meets its neighboring side/back wall at other than
//! 90 degrees — which is a materially different problem (`between_planes`
//! only handles two *perpendicular* planes, and two arena walls meeting at
//! a corner aren't); the goal structures themselves (no back-net cutout —
//! the back walls here are solid, flat planes spanning the full width); and
//! any geometry finer than a single flat plane or single-radius fillet per
//! boundary segment (the real field mesh's corners and transitions are more
//! complex than this). See `RB-PHYSICS-001`'s Non-goals.

use crate::body::{StaticPlane, StaticQuarterPipe};
use rb_domain::Vec3;

/// Side wall position (the field's half-width along X) — a commonly-cited
/// community-measured Rocket League field dimension (matching the
/// convention already used for `drive::MAX_CAR_SPEED`/`JUMP_SPEED`), not
/// independently confirmed by this project against real field mesh data.
pub const SIDE_WALL_X: f32 = 4096.0;

/// Back wall position (the field's half-length along Y) — same sourcing
/// caveat as `SIDE_WALL_X`.
pub const BACK_WALL_Y: f32 = 5120.0;

/// Ceiling height along Z — same sourcing caveat as `SIDE_WALL_X`.
pub const CEILING_Z: f32 = 2044.0;

/// Uncalibrated placeholder: how far back from the true rectangular corner
/// (where a side wall would meet a back wall at 90 degrees) each of the
/// four diagonal corner walls is inset, along both axes equally. This port
/// has no verified reference for Rocket League's actual corner-wall
/// geometry — the real arena's corners aren't a single flat 45-degree cut
/// at all (they're curved, and blend into the ramps/curves this port
/// doesn't model either) — this value was chosen only to produce a
/// recognizably octagonal footprint for testing, not measured from real
/// field mesh data.
pub const CORNER_LENGTH: f32 = 1152.0;

/// Uncalibrated placeholder: the radius of the curved fillet connecting a
/// cardinal wall to the floor or ceiling (`standard_curves`). This port has
/// no verified reference for Rocket League's actual transition radius —
/// chosen only to be small relative to the field's own dimensions (a
/// visibly local rounding of the corner, not a wall-length-scale ramp), not
/// measured from real field mesh data.
pub const FILLET_RADIUS: f32 = 292.0;

/// The floor: a flat plane at `z = 0`, normal `+Z` (up) — identical to the
/// `flat_ground()` helper this crate's tests have used since v0, just
/// exposed here as part of the standard-arena constructor.
pub fn standard_ground() -> StaticPlane {
    StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0)
}

fn ceiling_plane() -> StaticPlane {
    StaticPlane::new(Vec3::new(0.0, 0.0, -1.0), -CEILING_Z)
}

/// The side wall on the `sign`d side (`1.0` for `+X`, `-1.0` for `-X`).
fn side_wall_plane(sign: f32) -> StaticPlane {
    StaticPlane::new(Vec3::new(-sign, 0.0, 0.0), -SIDE_WALL_X)
}

/// The back wall on the `sign`d side (`1.0` for `+Y`, `-1.0` for `-Y`).
fn back_wall_plane(sign: f32) -> StaticPlane {
    StaticPlane::new(Vec3::new(0.0, -sign, 0.0), -BACK_WALL_Y)
}

/// The diagonal corner wall in quadrant `(sx, sy)` (each `1.0` or `-1.0`).
/// `offset` is `normal.dot(point_on_plane)` for the point where the corner
/// wall meets its side wall, `(SIDE_WALL_X - CORNER_LENGTH, BACK_WALL_Y)`,
/// with `normal = (-1, -1, 0) / sqrt(2)` — this magnitude is shared by all
/// four quadrants since `SIDE_WALL_X`/`BACK_WALL_Y` are used with the same
/// magnitude in every quadrant, only sign differs.
fn corner_wall_plane(sx: f32, sy: f32) -> StaticPlane {
    let normal = Vec3::new(-sx, -sy, 0.0) * std::f32::consts::FRAC_1_SQRT_2;
    let offset = -(SIDE_WALL_X - CORNER_LENGTH + BACK_WALL_Y) * std::f32::consts::FRAC_1_SQRT_2;
    StaticPlane::new(normal, offset)
}

/// The arena's full vertical boundary: 2 side walls (`+-X`), 2 back walls
/// (`+-Y`), a ceiling, and 4 diagonal corner walls (one per quadrant) —
/// 9 `StaticPlane`s total, each with its normal pointing back into the
/// playable volume (matching `StaticPlane`'s own convention — see its doc
/// comment and `RB-PHYSICS-001-FR-013`'s existing wall examples). A corner
/// wall's plane passes through the two points where it meets its
/// neighboring side and back wall, `CORNER_LENGTH` in from the true
/// rectangular corner along each axis; by symmetry all four corner walls
/// share one offset magnitude, only their normal's sign differs per
/// quadrant.
pub fn standard_walls() -> Vec<StaticPlane> {
    let mut walls = Vec::with_capacity(9);

    walls.push(side_wall_plane(1.0));
    walls.push(side_wall_plane(-1.0));
    walls.push(back_wall_plane(1.0));
    walls.push(back_wall_plane(-1.0));
    walls.push(ceiling_plane());

    for &(sx, sy) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        walls.push(corner_wall_plane(sx, sy));
    }

    walls
}

/// Curved wall-to-floor/wall-to-ceiling fillets for all 9 walls — the 4
/// cardinal walls (`RB-PHYSICS-001-FR-020`) and, since `FR-021`, the 4
/// diagonal corner walls too — 16 `StaticQuarterPipe`s total (one
/// floor-side and one ceiling-side fillet per wall), each built by
/// `StaticQuarterPipe::between_planes` from the same flat planes
/// `standard_walls` uses. A corner wall's own "along the wall" direction
/// isn't a coordinate axis, unlike a cardinal wall's, so its
/// `axis_direction` is computed via a cross product (`floor.normal.cross(
/// &wall.normal)`) rather than hand-picked — `between_planes` itself needs
/// no generalization, since a vertical wall's normal is perpendicular to
/// the floor/ceiling's regardless of the wall's own horizontal rotation.
pub fn standard_curves() -> Vec<StaticQuarterPipe> {
    let floor = standard_ground();
    let ceiling = ceiling_plane();
    let mut curves = Vec::with_capacity(16);

    for &sign in &[1.0f32, -1.0] {
        let wall = side_wall_plane(sign);
        let axis_direction = Vec3::new(0.0, 1.0, 0.0);
        curves.push(StaticQuarterPipe::between_planes(
            &floor,
            &wall,
            FILLET_RADIUS,
            axis_direction,
        ));
        curves.push(StaticQuarterPipe::between_planes(
            &ceiling,
            &wall,
            FILLET_RADIUS,
            axis_direction,
        ));
    }

    for &sign in &[1.0f32, -1.0] {
        let wall = back_wall_plane(sign);
        let axis_direction = Vec3::new(1.0, 0.0, 0.0);
        curves.push(StaticQuarterPipe::between_planes(
            &floor,
            &wall,
            FILLET_RADIUS,
            axis_direction,
        ));
        curves.push(StaticQuarterPipe::between_planes(
            &ceiling,
            &wall,
            FILLET_RADIUS,
            axis_direction,
        ));
    }

    for &(sx, sy) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let wall = corner_wall_plane(sx, sy);
        // A vertical wall's normal always has zero Z component while the
        // floor/ceiling's is purely Z, so this cross product is already
        // exactly unit length (the two normals are always perpendicular,
        // regardless of the wall's own horizontal rotation) — no
        // `.normalize()`/`.unwrap()` needed.
        let floor_axis_direction = floor.normal.cross(&wall.normal);
        curves.push(StaticQuarterPipe::between_planes(
            &floor,
            &wall,
            FILLET_RADIUS,
            floor_axis_direction,
        ));
        let ceiling_axis_direction = ceiling.normal.cross(&wall.normal);
        curves.push(StaticQuarterPipe::between_planes(
            &ceiling,
            &wall,
            FILLET_RADIUS,
            ceiling_axis_direction,
        ));
    }

    curves
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_walls_has_nine_planes() {
        assert_eq!(standard_walls().len(), 9);
    }

    #[test]
    fn the_origin_is_inside_every_wall_and_the_ceiling() {
        let origin = Vec3::ZERO;
        for wall in standard_walls() {
            assert!(
                wall.signed_distance(&origin) > 0.0,
                "expected the arena's center to be on the playable side of every wall, got {wall:?}"
            );
        }
    }

    #[test]
    fn side_and_back_walls_are_symmetric() {
        let walls = standard_walls();
        // The first four entries are +-X, +-Y in that order (see
        // standard_walls); each opposing pair shares the same offset
        // magnitude by construction.
        assert_eq!(walls[0].offset, walls[1].offset);
        assert_eq!(walls[2].offset, walls[3].offset);
    }

    #[test]
    fn a_point_just_outside_a_side_wall_is_not_inside() {
        let just_past = Vec3::new(SIDE_WALL_X + 1.0, 0.0, 100.0);
        let walls = standard_walls();
        let side_wall = walls[0]; // normal (-1, 0, 0), the +X side wall
        assert!(side_wall.signed_distance(&just_past) < 0.0);
    }

    #[test]
    fn ceiling_bounds_from_above() {
        let just_below_ceiling = Vec3::new(0.0, 0.0, CEILING_Z - 1.0);
        let just_above_ceiling = Vec3::new(0.0, 0.0, CEILING_Z + 1.0);
        let walls = standard_walls();
        let ceiling = walls[4];
        assert!(ceiling.signed_distance(&just_below_ceiling) > 0.0);
        assert!(ceiling.signed_distance(&just_above_ceiling) < 0.0);
    }

    #[test]
    fn a_corner_wall_cuts_off_the_true_rectangular_corner() {
        // The true (uncut) rectangular corner, (SIDE_WALL_X, BACK_WALL_Y),
        // must be outside every corner wall's playable side — proving the
        // octagon actually removes that corner rather than just adding
        // walls that never bind.
        let true_corner = Vec3::new(SIDE_WALL_X, BACK_WALL_Y, 100.0);
        let walls = standard_walls();
        let corner_wall_for_first_quadrant = walls[5]; // (sx, sy) = (1.0, 1.0)
        assert!(
            corner_wall_for_first_quadrant.signed_distance(&true_corner) < 0.0,
            "expected the true rectangular corner to be cut off by the corner wall"
        );
    }

    #[test]
    fn all_four_corner_walls_share_one_offset_magnitude() {
        let walls = standard_walls();
        let corner_offsets: Vec<f32> = walls[5..9].iter().map(|w| w.offset).collect();
        for offset in &corner_offsets[1..] {
            assert!((offset - corner_offsets[0]).abs() < 1e-4);
        }
    }

    #[test]
    fn standard_curves_has_sixteen_fillets() {
        assert_eq!(standard_curves().len(), 16);
    }

    #[test]
    fn every_standard_curve_bridges_a_wall_to_the_floor_or_ceiling() {
        // Every fillet's axis should sit exactly FILLET_RADIUS above the
        // floor (a floor-side fillet) or FILLET_RADIUS below the ceiling (a
        // ceiling-side fillet) -- never anywhere else.
        for curve in standard_curves() {
            let near_floor = (curve.axis_point.z - FILLET_RADIUS).abs() < 1e-3;
            let near_ceiling = (curve.axis_point.z - (CEILING_Z - FILLET_RADIUS)).abs() < 1e-3;
            assert!(
                near_floor || near_ceiling,
                "expected every curve's axis to sit radius-in from the floor or ceiling, got z={}",
                curve.axis_point.z
            );
        }
    }

    #[test]
    fn every_standard_curve_sits_radius_in_from_a_vertical_wall() {
        // `between_planes` places its axis exactly `radius` from *each*
        // bridged plane (see `StaticQuarterPipe::between_planes`'s doc
        // comment), so every curve's axis must sit exactly `FILLET_RADIUS`
        // from some vertical wall -- a side wall, a back wall, or (since
        // FR-021) a diagonal corner wall.
        let vertical_walls: Vec<StaticPlane> = standard_walls()
            .into_iter()
            .filter(|w| w.normal.z == 0.0)
            .collect();
        for curve in standard_curves() {
            let sits_radius_in_from_some_wall = vertical_walls
                .iter()
                .any(|wall| (wall.signed_distance(&curve.axis_point) - FILLET_RADIUS).abs() < 1e-3);
            assert!(
                sits_radius_in_from_some_wall,
                "expected every curve's axis to sit radius-in from some vertical wall, got {:?}",
                curve.axis_point
            );
        }
    }

    #[test]
    fn a_corner_wall_fillets_axis_sits_radius_in_from_both_the_corner_wall_and_the_floor() {
        let wall = corner_wall_plane(1.0, 1.0);
        let floor = standard_ground();
        let pipe = StaticQuarterPipe::between_planes(
            &floor,
            &wall,
            FILLET_RADIUS,
            Vec3::new(1.0, -1.0, 0.0),
        );
        assert!((wall.signed_distance(&pipe.axis_point) - FILLET_RADIUS).abs() < 1e-3);
        assert!((floor.signed_distance(&pipe.axis_point) - FILLET_RADIUS).abs() < 1e-3);
    }

    #[test]
    fn a_corner_wall_fillets_sector_vectors_are_perpendicular_unit_vectors() {
        let wall = corner_wall_plane(1.0, 1.0);
        let floor = standard_ground();
        let axis_direction = floor.normal.cross(&wall.normal);
        let pipe = StaticQuarterPipe::between_planes(&floor, &wall, FILLET_RADIUS, axis_direction);
        assert!((pipe.sector_start.length() - 1.0).abs() < 1e-4);
        assert!((pipe.sector_end.length() - 1.0).abs() < 1e-4);
        assert!(pipe.sector_start.dot(&pipe.sector_end).abs() < 1e-4);
    }

    #[test]
    fn every_corner_walls_cross_product_axis_direction_is_unit_length() {
        // The invariant the production `.normalize()`-free code in
        // `standard_curves` relies on: a vertical wall's normal is always
        // exactly perpendicular to the floor/ceiling's, so the raw cross
        // product is already unit length, for every quadrant.
        let floor = standard_ground();
        for &(sx, sy) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            let wall = corner_wall_plane(sx, sy);
            let axis_direction = floor.normal.cross(&wall.normal);
            assert!(
                (axis_direction.length() - 1.0).abs() < 1e-4,
                "cross product for quadrant ({sx}, {sy}) was not unit length: {axis_direction:?}"
            );
        }
    }
}
