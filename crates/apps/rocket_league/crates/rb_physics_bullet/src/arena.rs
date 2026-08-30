//! Rocket League's real standard-arena field dimensions
//! (`RB-PHYSICS-001-FR-019`), and a constructor for its octagonal
//! footprint plus a ceiling — built entirely from the existing generic
//! `StaticPlane`/`PhysicsWorld::with_wall` machinery `RB-PHYSICS-001-FR-013`
//! already provides. No new collision code: a ceiling and a corner-cut wall
//! are each just another flat `StaticPlane`, the same as the ground or a
//! side wall.
//!
//! `standard_curves` (`RB-PHYSICS-001-FR-020`) adds curved
//! wall-to-floor/wall-to-ceiling fillets for the 4 cardinal (axis-aligned)
//! walls, built from `body::StaticQuarterPipe::between_planes` — still no
//! new collision code of its own here, just composing the same flat planes
//! `standard_walls` already builds.
//!
//! **Still not modeled**: fillets at the 4 diagonal corner walls (their
//! normals aren't axis-aligned, so `between_planes`' orthonormal-basis
//! assumption doesn't hold there — see `StaticQuarterPipe::between_planes`'s
//! own doc comment), the goal structures themselves (no back-net cutout —
//! the back walls here are solid, flat planes spanning the full width), and
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

    // Corner offset: normal.dot(point_on_plane) for the point where the
    // corner wall meets its side wall, (SIDE_WALL_X - CORNER_LENGTH,
    // BACK_WALL_Y), with normal (-1, -1, 0) / sqrt(2) — this magnitude is
    // shared by all four quadrants since SIDE_WALL_X/BACK_WALL_Y are used
    // with the same magnitude in every quadrant, only sign differs.
    let corner_offset =
        -(SIDE_WALL_X - CORNER_LENGTH + BACK_WALL_Y) * std::f32::consts::FRAC_1_SQRT_2;
    for &(sx, sy) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let normal = Vec3::new(-sx, -sy, 0.0) * std::f32::consts::FRAC_1_SQRT_2;
        walls.push(StaticPlane::new(normal, corner_offset));
    }

    walls
}

/// Curved wall-to-floor/wall-to-ceiling fillets (`RB-PHYSICS-001-FR-020`)
/// for the 4 cardinal walls only — 8 `StaticQuarterPipe`s total (one
/// floor-side and one ceiling-side fillet per cardinal wall), each built by
/// `StaticQuarterPipe::between_planes` from the same flat planes
/// `standard_walls` uses. The 4 diagonal corner walls get no fillet here —
/// see the module doc for why (`between_planes`' orthonormal-basis
/// assumption doesn't hold for a non-axis-aligned wall).
pub fn standard_curves() -> Vec<StaticQuarterPipe> {
    let floor = standard_ground();
    let ceiling = ceiling_plane();
    let mut curves = Vec::with_capacity(8);

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
    fn standard_curves_has_eight_fillets() {
        assert_eq!(standard_curves().len(), 8);
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
    fn every_standard_curve_sits_radius_in_from_a_side_or_back_wall() {
        for curve in standard_curves() {
            let near_side_wall = (curve.axis_point.x.abs() - (SIDE_WALL_X - FILLET_RADIUS)).abs()
                < 1e-3
                && curve.axis_point.y == 0.0;
            let near_back_wall = (curve.axis_point.y.abs() - (BACK_WALL_Y - FILLET_RADIUS)).abs()
                < 1e-3
                && curve.axis_point.x == 0.0;
            assert!(
                near_side_wall || near_back_wall,
                "expected every curve's axis to sit radius-in from a cardinal wall, got {:?}",
                curve.axis_point
            );
        }
    }
}
