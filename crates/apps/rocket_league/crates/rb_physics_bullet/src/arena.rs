//! Rocket League's real standard-arena field dimensions
//! (`RB-PHYSICS-001-FR-019`), and a constructor for its octagonal
//! footprint plus a ceiling — built entirely from the existing generic
//! `StaticPlane`/`PhysicsWorld::with_wall` machinery `RB-PHYSICS-001-FR-013`
//! already provides. No new collision code: a ceiling and a corner-cut wall
//! are each just another flat `StaticPlane`, the same as the ground or a
//! side wall.
//!
//! `standard_curves` (`RB-PHYSICS-001-FR-020`/`FR-021`/`FR-022`) adds curved
//! fillets throughout the arena's vertical boundary — wall-to-floor/
//! wall-to-ceiling seams for all 9 walls (the 4 cardinal walls and, since
//! FR-021, the 4 diagonal corner walls too), and, since FR-022, a fillet at
//! each of the 8 vertical edges where a corner wall meets its neighboring
//! side/back wall — all built from `body::StaticQuarterPipe::between_planes`
//! — still no new collision code of its own here, just composing the same
//! flat planes `standard_walls` already builds. `between_planes` itself
//! generalized (as part of FR-022) to handle any two non-parallel planes,
//! not just perpendicular ones, so this module never needed its own
//! geometry code for the corner walls' shallower (135-degree) vertical
//! edges — only which two planes to bridge, and a plain `(0, 0, 1)` axis
//! direction (the edge itself is vertical), differ from the floor/
//! ceiling-seam case.
//!
//! `standard_corner_fillets` (`RB-PHYSICS-001-FR-023`) adds a small
//! spherical patch at each of the 16 compound-corner vertices where a
//! vertical-edge fillet meets a floor- or ceiling-seam fillet, near a
//! corner wall's own top/bottom endpoint — built from
//! `body::StaticCornerFillet::between_three_planes` on the same three flat
//! planes (floor/ceiling, side/back wall, corner wall) that meet there,
//! rather than the two `standard_curves`' own fillets each bridge.
//!
//! Since `RB-PHYSICS-001-FR-025`, a corner wall's own floor/ceiling-seam
//! arches (the 8 of `standard_curves`' 24 fillets bridging a corner wall to
//! the floor or ceiling) use the distinctly larger `CORNER_ARCH_RADIUS`
//! rather than the cardinal walls' `FILLET_RADIUS`, matching real Rocket
//! League's bigger, more swept corner-boost curve. All 16
//! `standard_corner_fillets` also switch to `CORNER_ARCH_RADIUS`, since each
//! one touches one of those arches and `between_three_planes` needs one
//! shared radius across all three planes it blends to still meet the arch
//! exactly where their axes cross (see `CORNER_ARCH_RADIUS`'s own doc
//! comment). The 8 vertical-edge fillets where a corner wall meets its
//! neighboring side/back wall are unaffected — they're independent,
//! additive contact sources next to these arches, not blended with them,
//! same as every other adjoining-fillet pair in this module — and keep
//! `FILLET_RADIUS`.
//!
//! `standard_goal_walls`/`standard_goal_cutout_fillets`
//! (`RB-PHYSICS-001-FR-024`) open an actual goal-mouth window in each back
//! wall — until now, `standard_walls`' two back walls were solid, flat
//! planes spanning the full width, with no opening at all. `standard_walls`
//! itself now returns 7 planes instead of 9 (the back walls move out of
//! it, replaced by `standard_goal_walls`' `StaticGoalWall`s), and
//! `standard_goal_cutout_fillets` rounds the window's three edges (two
//! posts and a crossbar) per goal — 6 `StaticQuarterPipe`s, built the same
//! way every other fillet here is, from a pair of flat planes, one of them
//! (the post's or crossbar's own inward-facing surface) a purely-geometric
//! construction used only to derive the fillet, never added as a real wall
//! itself (see `goal_post_plane`/`goal_crossbar_plane`'s own doc comments
//! for why that would be wrong).
//!
//! `standard_goal_corner_fillets` (`RB-PHYSICS-001-FR-026`) closes the gap
//! `standard_goal_cutout_fillets`' own doc comment flagged: the two compound
//! corners per goal where a post's own vertical fillet meets the crossbar's
//! own horizontal fillet, one per post per goal (4 total). Same approach
//! `RB-PHYSICS-001-FR-023` used for the arena's own compound corners —
//! `body::StaticCornerFillet::between_three_planes` directly on the three
//! real flat planes that meet there (the back wall, that post's plane, and
//! the crossbar) — reusing `FILLET_RADIUS` unchanged, since (unlike
//! `FR-025`'s arena corners) both edge fillets meeting here already share
//! one radius. The goal's other two corners, where a post meets the floor,
//! aren't compound corners needing this treatment: the window's own bottom
//! edge sits exactly at floor level, so a post fillet there simply ends
//! flush with the ground, the same as any other fillet meeting the floor.
//!
//! Since `RB-PHYSICS-001-FR-027`, a car (box) is actually deflected by
//! every fillet in this module too — `collision::contacts_vs_quarter_pipe`/
//! `contacts_vs_corner_fillet` test a box's own 8 corners against the
//! curved surface (the same technique `contacts_vs_plane`'s box path
//! already used for a flat plane), an approximation of the box as a whole,
//! not a full convex-vs-curved-surface narrow phase — see
//! `collision::box_vs_quarter_pipe`'s own doc comment for exactly what
//! that does and doesn't catch.
//!
//! Since `RB-PHYSICS-001-FR-028`, a car can drive into a goal too —
//! `collision::contacts_vs_goal_wall`'s box path now tests each of the
//! box's own 8 corners against the window (the same per-corner approach
//! FR-027 established for curved geometry), rather than falling straight
//! through to an unwindowed plane contact the way it did through FR-027.
//!
//! `standard_goal_back_walls`/`standard_goal_side_walls`/`standard_goal_roofs`
//! (`RB-PHYSICS-001-FR-029`) model a bounded interior behind each
//! goal-mouth window, closing the "a ball or car passes into open,
//! unbounded space" gap FR-024 through FR-028's own doc comments flagged —
//! 2 plain back-of-net planes (`goal_back_wall_plane`, `GOAL_DEPTH` behind
//! the real back wall, reachable only through the window so an unbounded
//! plane there is exact, not an approximation) plus 4 bounded side walls
//! and 2 bounded roofs (`goal_side_wall`/`goal_roof`, each a
//! `body::StaticBoundedWall` reusing `goal_post_plane`/`goal_crossbar_plane`
//! unchanged but bounded to the goal's own depth/width/height footprint —
//! an unbounded plane at either position would incorrectly wall off the
//! *entire* main field, the same problem those planes' own doc comments
//! already documented for their original, purely-geometric role).
//!
//! Since `RB-PHYSICS-001-FR-033`, each goal also gets a real mass-spring net
//! panel (`standard_nets`, a `net::NetMesh` each, `NET_DEPTH` behind the
//! real back wall — well in front of `goal_back_wall_plane`'s own rigid
//! backstop, unchanged) catching the *ball* — see `net::NetMesh`'s own doc
//! comment for the design and for what's still explicitly out of scope
//! (a car's own contact against the net, a full 3D "sock" shape, bending
//! stiffness).
//!
//! **Still not modeled**: any geometry finer than a single flat plane,
//! single-radius edge fillet, or single-radius corner fillet per boundary
//! segment (the real field mesh's corners and transitions are more complex
//! than this). See `RB-PHYSICS-001`'s Non-goals.

use crate::body::{
    StaticBoundedWall, StaticCornerFillet, StaticGoalWall, StaticPlane, StaticQuarterPipe,
};
use crate::net::NetMesh;
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

/// Uncalibrated placeholder: the radius of the curved arch connecting a
/// diagonal *corner* wall to the floor or ceiling — distinctly larger than
/// the cardinal walls' own `FILLET_RADIUS` transition, since real Rocket
/// League's corner-boost area is a noticeably bigger, more swept curve than
/// a cardinal wall's small rounding, not just a scaled-down version of the
/// same shape. This port has no verified reference for the real arch's
/// actual radius either — chosen only to read as visibly larger than
/// `FILLET_RADIUS` in tests, not measured from real field mesh data. Also
/// governs the 16 compound-corner fillets (`standard_corner_fillets`),
/// since every one of them touches a corner wall's own floor- or
/// ceiling-seam arch and needs to share its radius to still meet it exactly
/// where their axes cross (see `StaticCornerFillet::between_three_planes`'s
/// own doc comment for why a mismatched radius there wouldn't blend
/// cleanly).
pub const CORNER_ARCH_RADIUS: f32 = 750.0;

// The whole point of RB-PHYSICS-001-FR-025: a corner wall's own
// floor/ceiling arch should read as visibly bigger than a cardinal wall's
// small rounding, not just a scaled-down copy of the same shape. Enforced at
// compile time rather than as a runtime test, since it's a relationship
// between two constants.
const _: () = assert!(CORNER_ARCH_RADIUS > FILLET_RADIUS);

/// Half-width of the goal-mouth window cut into each back wall — a
/// commonly-cited community-measured Rocket League dimension (same
/// sourcing caveat as `SIDE_WALL_X`), not independently confirmed by this
/// project against real field mesh data.
pub const GOAL_HALF_WIDTH: f32 = 892.755;

/// Height of the goal-mouth window — same sourcing caveat as
/// `GOAL_HALF_WIDTH`.
pub const GOAL_HEIGHT: f32 = 642.775;

/// Uncalibrated placeholder: how far behind the back wall the goal box's
/// own interior extends (`RB-PHYSICS-001-FR-029`) — this port has no
/// verified reference for Rocket League's actual net depth at all, unlike
/// `GOAL_HALF_WIDTH`/`GOAL_HEIGHT`; chosen only to be a visibly real
/// interior volume (comparable in scale to the goal mouth's own
/// dimensions), not measured from real field mesh data.
pub const GOAL_DEPTH: f32 = 880.0;

/// How far behind the real back wall a goal's `net::NetMesh` panel sits
/// (`RB-PHYSICS-001-FR-033`) — deliberately less than `GOAL_DEPTH`, so a
/// ball entering the goal always meets the springy net well before it could
/// ever reach `goal_back_wall_plane`'s own rigid backstop (still there,
/// completely unchanged, as a safety net *behind* the net for the
/// vanishingly unlikely case the mesh's own solve lets the ball slip past
/// it — see `net::NetMesh`'s own doc comment for what a car, which isn't
/// tested against the mesh at all, still collides with instead). Another
/// uncalibrated placeholder, same category as `GOAL_DEPTH` itself.
pub const NET_DEPTH: f32 = GOAL_DEPTH * 0.5;

/// Column count for `standard_nets`' own grid — see `net::NetMesh::
/// rectangular_grid`'s own doc comment for what a "column" means here.
pub const NET_COLS: usize = 7;
/// Row count for `standard_nets`' own grid.
pub const NET_ROWS: usize = 5;

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

/// The arena's flat vertical boundary, minus the two back walls: 2 side
/// walls (`+-X`), a ceiling, and 4 diagonal corner walls (one per
/// quadrant) — 7 `StaticPlane`s total, each with its normal pointing back
/// into the playable volume (matching `StaticPlane`'s own convention —
/// see its doc comment and `RB-PHYSICS-001-FR-013`'s existing wall
/// examples). A corner wall's plane passes through the two points where
/// it meets its neighboring side and back wall, `CORNER_LENGTH` in from
/// the true rectangular corner along each axis; by symmetry all four
/// corner walls share one offset magnitude, only their normal's sign
/// differs per quadrant.
///
/// The back walls themselves moved out of this list as of
/// `RB-PHYSICS-001-FR-024`: each now has a goal-mouth window cut into it,
/// which a plain `StaticPlane` has no way to represent, so they live in
/// `standard_goal_walls` (`StaticGoalWall`s) instead — `PhysicsWorld::
/// standard_arena` wires both lists in together, and a car's own collision
/// with a back wall is unaffected either way (see `collision::
/// contacts_vs_goal_wall`'s own doc comment for why).
pub fn standard_walls() -> Vec<StaticPlane> {
    let mut walls = Vec::with_capacity(7);

    walls.push(side_wall_plane(1.0));
    walls.push(side_wall_plane(-1.0));
    walls.push(ceiling_plane());

    for &(sx, sy) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        walls.push(corner_wall_plane(sx, sy));
    }

    walls
}

/// The goal-mouth window's own vertical post plane on the `sign`d side
/// (`1.0` for `+X`, `-1.0` for `-X`) — the post's flat, inward-facing
/// surface, positioned `GOAL_HALF_WIDTH` in from center exactly like
/// `side_wall_plane` positions a real side wall `SIDE_WALL_X` in from
/// center (same formula, a narrower constant). Used only to derive a
/// post's own rounding fillet via `StaticQuarterPipe::between_planes` in
/// `standard_goal_cutout_fillets` — unlike `side_wall_plane`/
/// `corner_wall_plane`, this is never added to `standard_walls` as a real
/// collision wall itself: a real, infinite plane perpendicular to X at
/// this position would incorrectly wall off the *entire* rest of the
/// field at that X coordinate (a corner wall's own diagonal orientation
/// keeps it non-binding everywhere except right at the true corner; a
/// plane facing straight along X has no such saving grace).
fn goal_post_plane(sign: f32) -> StaticPlane {
    StaticPlane::new(Vec3::new(-sign, 0.0, 0.0), -GOAL_HALF_WIDTH)
}

/// The goal-mouth window's own crossbar plane — the crossbar's flat,
/// downward-facing surface, positioned `GOAL_HEIGHT` up from the floor
/// exactly like `ceiling_plane` positions the real ceiling `CEILING_Z` up
/// (same formula, a lower constant). Same purely-geometric role as
/// `goal_post_plane`: feeds `StaticQuarterPipe::between_planes` in
/// `standard_goal_cutout_fillets`, never added to `standard_walls` itself
/// (it would incorrectly cap the entire field's height at `GOAL_HEIGHT`
/// rather than just the goal mouth's own opening).
fn goal_crossbar_plane() -> StaticPlane {
    StaticPlane::new(Vec3::new(0.0, 0.0, -1.0), -GOAL_HEIGHT)
}

/// The goal-mouth window cut into the back wall on the `sign`d side
/// (`1.0` for `+Y`, `-1.0` for `-Y`) — centered on the wall at half the
/// goal's own height, `GOAL_HALF_WIDTH` wide each way and `GOAL_HEIGHT`
/// tall (from the floor up), wrapping the same `back_wall_plane` this
/// wall used before it had a window at all.
fn goal_wall(sign: f32) -> StaticGoalWall {
    StaticGoalWall::new(
        back_wall_plane(sign),
        Vec3::new(0.0, sign * BACK_WALL_Y, GOAL_HEIGHT * 0.5),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        GOAL_HALF_WIDTH,
        GOAL_HEIGHT * 0.5,
    )
}

/// Both goals' windowed back walls (`RB-PHYSICS-001-FR-024`) — 2
/// `StaticGoalWall`s, one per `+-Y` back wall, each with a `GOAL_HALF_WIDTH`
/// by `GOAL_HEIGHT` window centered on it. Both the ball and, since
/// `RB-PHYSICS-001-FR-028`, a car too are let through the window (see
/// `StaticGoalWall`'s and `collision::contacts_vs_goal_wall`'s own doc
/// comments).
pub fn standard_goal_walls() -> Vec<StaticGoalWall> {
    vec![goal_wall(1.0), goal_wall(-1.0)]
}

/// Fillets rounding the three edges of each goal-mouth window
/// (`RB-PHYSICS-001-FR-024`) — two vertical posts and a horizontal
/// crossbar, per goal, 6 `StaticQuarterPipe`s total. Each is built by
/// `StaticQuarterPipe::between_planes` from the real back-wall plane and a
/// purely-geometric post/crossbar plane (`goal_post_plane`/
/// `goal_crossbar_plane`) positioned at exactly the window's own edge, so
/// the fillet's own tangent point lands exactly on the window boundary —
/// the ball transitions smoothly from the flat wall, through the rounded
/// edge, into the open window, with no gap or overlap between them, the
/// same property every other fillet/window pairing in this crate already
/// has (e.g. a corner wall's own edge fillet sitting exactly on the corner
/// wall's real position). The two compound corners per goal where a post's
/// fillet meets the crossbar's are deliberately not blended into a single
/// smooth vertex — see this module's own doc comment.
pub fn standard_goal_cutout_fillets() -> Vec<StaticQuarterPipe> {
    let crossbar = goal_crossbar_plane();
    let mut fillets = Vec::with_capacity(6);

    for &back_sign in &[1.0f32, -1.0] {
        let wall = back_wall_plane(back_sign);
        for &post_sign in &[1.0f32, -1.0] {
            let post = goal_post_plane(post_sign);
            fillets.push(StaticQuarterPipe::between_planes(
                &wall,
                &post,
                FILLET_RADIUS,
                Vec3::new(0.0, 0.0, 1.0),
            ));
        }
        fillets.push(StaticQuarterPipe::between_planes(
            &wall,
            &crossbar,
            FILLET_RADIUS,
            Vec3::new(1.0, 0.0, 0.0),
        ));
    }

    fillets
}

/// Compound-corner fillets at each goal's two top corners
/// (`RB-PHYSICS-001-FR-026`) — the vertices where a post's own vertical
/// fillet (`standard_goal_cutout_fillets`) meets the crossbar's own
/// horizontal fillet, one per post per goal (4 total: 2 posts times 2
/// goals). `standard_goal_cutout_fillets`' own doc comment flagged these as
/// deliberately left as a sharp, unblended vertex; this closes that gap the
/// same way `RB-PHYSICS-001-FR-023` closed the arena's own compound
/// corners — via `StaticCornerFillet::between_three_planes` directly on the
/// three real flat planes that meet there (the back wall, that post's own
/// plane, and the crossbar), rather than from the two edge fillets
/// `standard_goal_cutout_fillets` builds at that vertex, since a corner
/// fillet's center is already exactly their common axis intersection (see
/// `between_three_planes`'s own doc comment). Reuses `FILLET_RADIUS`
/// unchanged — unlike the arena's own diagonal-corner fillets
/// (`RB-PHYSICS-001-FR-025`), both edge fillets meeting here already share
/// one radius, so there's no mismatched-radius concern requiring a
/// dedicated constant. The goal's other two corners, where a post meets the
/// floor, aren't compound corners at all: the window's own bottom edge
/// sits exactly at floor level, so nothing here is any different from an
/// ordinary post fillet ending flush with the ground the ball already
/// rolls on.
pub fn standard_goal_corner_fillets() -> Vec<StaticCornerFillet> {
    let crossbar = goal_crossbar_plane();
    let mut fillets = Vec::with_capacity(4);

    for &back_sign in &[1.0f32, -1.0] {
        let wall = back_wall_plane(back_sign);
        for &post_sign in &[1.0f32, -1.0] {
            let post = goal_post_plane(post_sign);
            fillets.push(StaticCornerFillet::between_three_planes(
                &wall,
                &post,
                &crossbar,
                FILLET_RADIUS,
            ));
        }
    }

    fillets
}

/// The goal box's own back-of-net wall on the `sign`d side (`1.0` for
/// `+Y`, `-1.0` for `-Y`), `GOAL_DEPTH` behind the real back wall
/// (`RB-PHYSICS-001-FR-029`) — a plain, unbounded `StaticPlane` like
/// `back_wall_plane` itself, not a `StaticBoundedWall`: nothing can ever
/// reach this plane except by first passing through the goal-mouth
/// window (`GOAL_HALF_WIDTH` wide, `GOAL_HEIGHT` tall — see
/// `StaticGoalWall`'s own doc comment), since the real back wall is solid
/// everywhere else, so an unbounded plane here is exact, not an
/// approximation the way one at the goal's own side/roof position would
/// be (see `goal_side_wall`/`goal_roof`'s own doc comments).
fn goal_back_wall_plane(sign: f32) -> StaticPlane {
    StaticPlane::new(Vec3::new(0.0, -sign, 0.0), -(BACK_WALL_Y + GOAL_DEPTH))
}

/// Both goals' own back-of-net walls (`RB-PHYSICS-001-FR-029`) — 2 plain
/// `StaticPlane`s, one per goal, added to `PhysicsWorld.walls` alongside
/// the rest of the arena's flat walls (unlike the goal's side walls and
/// roof, this needs no bound — see `goal_back_wall_plane`'s own doc
/// comment).
pub fn standard_goal_back_walls() -> Vec<StaticPlane> {
    vec![goal_back_wall_plane(1.0), goal_back_wall_plane(-1.0)]
}

/// One of a goal box's own two side walls, on goal `back_sign` (`1.0` for
/// `+Y`, `-1.0` for `-Y`) and post `post_sign` (`1.0` for `+X`, `-1.0` for
/// `-X`) (`RB-PHYSICS-001-FR-029`). Reuses `goal_post_plane(post_sign)` as
/// its own flat plane unchanged — the post's own inward-facing surface at
/// `x = post_sign * GOAL_HALF_WIDTH` is exactly where the goal box's own
/// side wall needs to sit too — but wraps it in a `StaticBoundedWall`
/// bounded to the goal's own depth (`y` from the real back wall out to
/// `GOAL_DEPTH` behind it) and height (`z` from the floor up to
/// `GOAL_HEIGHT`) range: unlike a post's own fillet-deriving role (see
/// `goal_post_plane`'s own doc comment), this plane needs to actually
/// collide, and an unbounded one at this `x` position would incorrectly
/// wall off the *entire* main field at that `x` coordinate, the same
/// problem `goal_post_plane`'s own doc comment already documents for a
/// different purpose.
fn goal_side_wall(back_sign: f32, post_sign: f32) -> StaticBoundedWall {
    StaticBoundedWall::new(
        goal_post_plane(post_sign),
        Vec3::new(
            post_sign * GOAL_HALF_WIDTH,
            back_sign * (BACK_WALL_Y + GOAL_DEPTH * 0.5),
            GOAL_HEIGHT * 0.5,
        ),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        GOAL_DEPTH * 0.5,
        GOAL_HEIGHT * 0.5,
    )
}

/// Both goals' own two side walls each (`RB-PHYSICS-001-FR-029`) — 4
/// `StaticBoundedWall`s total, one per post per goal.
pub fn standard_goal_side_walls() -> Vec<StaticBoundedWall> {
    let mut walls = Vec::with_capacity(4);
    for &back_sign in &[1.0f32, -1.0] {
        for &post_sign in &[1.0f32, -1.0] {
            walls.push(goal_side_wall(back_sign, post_sign));
        }
    }
    walls
}

/// A goal box's own roof, on goal `sign` (`1.0` for `+Y`, `-1.0` for
/// `-Y`) (`RB-PHYSICS-001-FR-029`). Reuses `goal_crossbar_plane()`
/// unchanged — the crossbar's own downward-facing surface at
/// `z = GOAL_HEIGHT` is exactly where the goal box's own roof needs to
/// sit too — but wraps it in a `StaticBoundedWall` bounded to the goal's
/// own width (`x` within `GOAL_HALF_WIDTH` either way) and depth (`y`
/// from the real back wall out to `GOAL_DEPTH` behind it) range, for the
/// same "an unbounded plane here would wall off the whole field" reason
/// `goal_side_wall`'s own doc comment gives.
fn goal_roof(sign: f32) -> StaticBoundedWall {
    StaticBoundedWall::new(
        goal_crossbar_plane(),
        Vec3::new(0.0, sign * (BACK_WALL_Y + GOAL_DEPTH * 0.5), GOAL_HEIGHT),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        GOAL_HALF_WIDTH,
        GOAL_DEPTH * 0.5,
    )
}

/// Both goals' own roofs (`RB-PHYSICS-001-FR-029`) — 2 `StaticBoundedWall`s
/// total, one per goal.
pub fn standard_goal_roofs() -> Vec<StaticBoundedWall> {
    vec![goal_roof(1.0), goal_roof(-1.0)]
}

/// A goal box's own net panel, on goal `sign` (`1.0` for `+Y`, `-1.0` for
/// `-Y`) (`RB-PHYSICS-001-FR-033`) — a `net::NetMesh::rectangular_grid`
/// spanning the same `GOAL_HALF_WIDTH`/`GOAL_HEIGHT` footprint as the
/// goal-mouth window itself (`standard_goal_walls`), so the net's own
/// perimeter lines up with the window's rim rather than leaving a gap a
/// ball could slip past unobstructed, positioned `NET_DEPTH` behind the
/// real back wall (well short of `goal_back_wall_plane`'s own rigid
/// backstop at the full `GOAL_DEPTH` — see `NET_DEPTH`'s own doc comment).
/// Lies in the plane perpendicular to `+Y`, spanning `+X` (`width_axis`)
/// and `+Z` (`height_axis`) — the same axes `standard_goal_walls`'s own
/// `u_axis`/`v_axis` use for this wall's window.
fn net_panel(sign: f32) -> NetMesh {
    NetMesh::rectangular_grid(
        Vec3::new(0.0, sign * (BACK_WALL_Y + NET_DEPTH), GOAL_HEIGHT * 0.5),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        GOAL_HALF_WIDTH,
        GOAL_HEIGHT,
        NET_COLS,
        NET_ROWS,
    )
}

/// Both goals' own net panels (`RB-PHYSICS-001-FR-033`) — 2 `net::NetMesh`s
/// total, one per goal, added to `PhysicsWorld.nets` via `with_net`.
pub fn standard_nets() -> Vec<NetMesh> {
    vec![net_panel(1.0), net_panel(-1.0)]
}

/// Curved fillets for the standard arena: wall-to-floor/wall-to-ceiling
/// transitions for all 9 walls — the 4 cardinal walls
/// (`RB-PHYSICS-001-FR-020`) and, since `FR-021`, the 4 diagonal corner
/// walls too — plus, since `RB-PHYSICS-001-FR-022`, a fillet at each of the
/// 8 vertical edges where a corner wall meets its neighboring side or back
/// wall. 24 `StaticQuarterPipe`s total (16 floor/ceiling-seam fillets, one
/// per wall per seam, and 8 vertical-edge fillets, one per corner-wall
/// endpoint), each built by `StaticQuarterPipe::between_planes` from the
/// same flat planes `standard_walls` uses. A corner wall's own "along the
/// wall" direction isn't a coordinate axis, unlike a cardinal wall's, so its
/// floor/ceiling-seam `axis_direction` is computed via a cross product
/// (`floor.normal.cross(&wall.normal)`) rather than hand-picked — this
/// works because a vertical wall's normal is always perpendicular to the
/// floor/ceiling's regardless of the wall's own horizontal rotation. The
/// vertical-edge fillets, by contrast, bridge two planes that *aren't*
/// perpendicular (a corner wall meets its neighboring side/back wall at 135
/// degrees, not 90), which `between_planes` now handles directly (see its
/// own doc comment) — no separate construction path needed, and their own
/// `axis_direction` is simply `(0, 0, 1)`, since the edge itself is
/// vertical.
///
/// Since `RB-PHYSICS-001-FR-025`, the 8 floor/ceiling-seam fillets that
/// bridge a *corner* wall (as opposed to a cardinal side/back wall) use
/// `CORNER_ARCH_RADIUS` rather than `FILLET_RADIUS` — see that constant's
/// own doc comment. The other 16 fillets (8 cardinal-wall floor/ceiling
/// seams, 8 vertical corner edges) are unaffected and still use
/// `FILLET_RADIUS`.
pub fn standard_curves() -> Vec<StaticQuarterPipe> {
    let floor = standard_ground();
    let ceiling = ceiling_plane();
    let mut curves = Vec::with_capacity(24);

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
        //
        // These two seams use CORNER_ARCH_RADIUS, not FILLET_RADIUS
        // (RB-PHYSICS-001-FR-025) — see that constant's own doc comment for
        // why a corner wall's floor/ceiling arch is distinctly bigger than a
        // cardinal wall's.
        let floor_axis_direction = floor.normal.cross(&wall.normal);
        curves.push(StaticQuarterPipe::between_planes(
            &floor,
            &wall,
            CORNER_ARCH_RADIUS,
            floor_axis_direction,
        ));
        let ceiling_axis_direction = ceiling.normal.cross(&wall.normal);
        curves.push(StaticQuarterPipe::between_planes(
            &ceiling,
            &wall,
            CORNER_ARCH_RADIUS,
            ceiling_axis_direction,
        ));
    }

    // The 8 vertical-edge fillets (`RB-PHYSICS-001-FR-022`), one per corner
    // wall's two endpoints, where it meets its neighboring side or back
    // wall. Both bridged planes are vertical, so the edge itself -- and
    // therefore this fillet's axis -- runs along Z; `between_planes` derives
    // whatever sector angle actually results (a shallow ~45 degrees here,
    // not the 90 the floor/ceiling-seam fillets get, since these two planes
    // meet at 135 degrees rather than a right angle) with no help needed
    // from this call site beyond passing the two planes.
    let vertical = Vec3::new(0.0, 0.0, 1.0);
    for &(sx, sy) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let side = side_wall_plane(sx);
        let back = back_wall_plane(sy);
        let corner = corner_wall_plane(sx, sy);
        curves.push(StaticQuarterPipe::between_planes(
            &side,
            &corner,
            FILLET_RADIUS,
            vertical,
        ));
        curves.push(StaticQuarterPipe::between_planes(
            &corner,
            &back,
            FILLET_RADIUS,
            vertical,
        ));
    }

    curves
}

/// Compound-corner fillets for the standard arena (`RB-PHYSICS-001-FR-023`):
/// a small spherical patch at each of the 16 vertices where a corner wall's
/// own vertical-edge fillet (`standard_curves`) would otherwise meet a
/// floor- or ceiling-seam fillet at a single sharp point — 4 per corner wall
/// (floor+side, floor+back, ceiling+side, ceiling+back) times 4 corner
/// walls (one per quadrant). Each is built by
/// `StaticCornerFillet::between_three_planes` directly from the same three
/// flat planes `standard_walls` already builds (floor or ceiling, the
/// neighboring side or back wall, and the corner wall itself) — not from
/// the two fillets `standard_curves` builds at that vertex, since a
/// corner-fillet's center is already exactly their common axis
/// intersection (see `StaticCornerFillet::between_three_planes`'s own doc
/// comment).
///
/// All 16 fillets use `CORNER_ARCH_RADIUS` (`RB-PHYSICS-001-FR-025`), not
/// `FILLET_RADIUS` — every one of them touches one of a corner wall's own
/// floor/ceiling-seam arches (`standard_curves`), which since FR-025 use the
/// larger `CORNER_ARCH_RADIUS`, and `between_three_planes` needs one shared
/// radius across all three planes it blends to still meet that arch exactly
/// where their axes cross.
pub fn standard_corner_fillets() -> Vec<StaticCornerFillet> {
    let floor = standard_ground();
    let ceiling = ceiling_plane();
    let mut fillets = Vec::with_capacity(16);

    for &(sx, sy) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let side = side_wall_plane(sx);
        let back = back_wall_plane(sy);
        let corner = corner_wall_plane(sx, sy);
        fillets.push(StaticCornerFillet::between_three_planes(
            &floor,
            &side,
            &corner,
            CORNER_ARCH_RADIUS,
        ));
        fillets.push(StaticCornerFillet::between_three_planes(
            &floor,
            &corner,
            &back,
            CORNER_ARCH_RADIUS,
        ));
        fillets.push(StaticCornerFillet::between_three_planes(
            &ceiling,
            &side,
            &corner,
            CORNER_ARCH_RADIUS,
        ));
        fillets.push(StaticCornerFillet::between_three_planes(
            &ceiling,
            &corner,
            &back,
            CORNER_ARCH_RADIUS,
        ));
    }

    fillets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_walls_has_seven_planes() {
        assert_eq!(standard_walls().len(), 7);
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
    fn side_walls_are_symmetric() {
        let walls = standard_walls();
        // The first two entries are +-X (see standard_walls); the opposing
        // pair shares the same offset magnitude by construction. (The back
        // walls' own symmetry is checked in `both_goal_walls_share_one_
        // offset_magnitude` -- they're `StaticGoalWall`s now, not part of
        // this list, since RB-PHYSICS-001-FR-024.)
        assert_eq!(walls[0].offset, walls[1].offset);
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
        let ceiling = walls[2];
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
        let corner_wall_for_first_quadrant = walls[3]; // (sx, sy) = (1.0, 1.0)
        assert!(
            corner_wall_for_first_quadrant.signed_distance(&true_corner) < 0.0,
            "expected the true rectangular corner to be cut off by the corner wall"
        );
    }

    #[test]
    fn all_four_corner_walls_share_one_offset_magnitude() {
        let walls = standard_walls();
        let corner_offsets: Vec<f32> = walls[3..7].iter().map(|w| w.offset).collect();
        for offset in &corner_offsets[1..] {
            assert!((offset - corner_offsets[0]).abs() < 1e-4);
        }
    }

    #[test]
    fn standard_curves_has_twenty_four_fillets() {
        assert_eq!(standard_curves().len(), 24);
    }

    #[test]
    fn every_floor_or_ceiling_seam_curve_bridges_a_wall_to_the_floor_or_ceiling() {
        // Every floor/ceiling-seam fillet's axis should sit exactly its own
        // radius above the floor (a floor-side fillet) or that radius below
        // the ceiling (a ceiling-side fillet) -- never anywhere else. Only
        // the first 16 of standard_curves()'s 24 entries are floor/
        // ceiling-seam fillets (see its own doc comment for the
        // construction order); the last 8 are vertical-edge fillets
        // (RB-PHYSICS-001-FR-022), which don't bridge to the floor or
        // ceiling at all -- see `every_corner_edge_curve_runs_vertically`.
        //
        // Since RB-PHYSICS-001-FR-025, the first 8 (cardinal side/back
        // walls) use FILLET_RADIUS while the next 8 (diagonal corner walls)
        // use the larger CORNER_ARCH_RADIUS -- see
        // `standard_curves`'s own doc comment for the construction order
        // this indexing relies on.
        for curve in &standard_curves()[0..8] {
            let near_floor = (curve.axis_point.z - FILLET_RADIUS).abs() < 1e-3;
            let near_ceiling = (curve.axis_point.z - (CEILING_Z - FILLET_RADIUS)).abs() < 1e-3;
            assert!(
                near_floor || near_ceiling,
                "expected every cardinal-wall floor/ceiling-seam curve's axis to sit \
                 FILLET_RADIUS-in from the floor or ceiling, got z={}",
                curve.axis_point.z
            );
        }
        for curve in &standard_curves()[8..16] {
            let near_floor = (curve.axis_point.z - CORNER_ARCH_RADIUS).abs() < 1e-3;
            let near_ceiling = (curve.axis_point.z - (CEILING_Z - CORNER_ARCH_RADIUS)).abs() < 1e-3;
            assert!(
                near_floor || near_ceiling,
                "expected every corner-wall floor/ceiling-seam curve's axis to sit \
                 CORNER_ARCH_RADIUS-in from the floor or ceiling, got z={}",
                curve.axis_point.z
            );
        }
    }

    #[test]
    fn every_corner_edge_curve_runs_vertically() {
        // The 8 vertical-edge fillets (RB-PHYSICS-001-FR-022, the last 8 of
        // standard_curves()'s 24 entries) bridge two vertical walls, so
        // their own axis -- unlike a floor/ceiling-seam fillet's -- runs
        // straight up Z, not along some horizontal direction.
        for curve in &standard_curves()[16..24] {
            assert!(
                (curve.axis_direction.x.abs() < 1e-4)
                    && (curve.axis_direction.y.abs() < 1e-4)
                    && (curve.axis_direction.z.abs() - 1.0).abs() < 1e-4,
                "expected a vertical-edge curve's axis to run along Z, got {:?}",
                curve.axis_direction
            );
        }
    }

    #[test]
    fn every_standard_curve_sits_radius_in_from_a_vertical_wall() {
        // `between_planes` places its axis exactly `radius` from *each*
        // bridged plane (see `StaticQuarterPipe::between_planes`'s doc
        // comment), so every curve's axis must sit exactly its own radius
        // -- `FILLET_RADIUS` for a cardinal wall or vertical corner edge,
        // `CORNER_ARCH_RADIUS` for a corner wall's own floor/ceiling seam
        // (RB-PHYSICS-001-FR-025) -- from some vertical wall -- a side
        // wall, a back wall, or (since FR-021) a diagonal corner wall. The
        // back walls aren't in `standard_walls()` itself since
        // RB-PHYSICS-001-FR-024 (they're `StaticGoalWall`s now), so
        // they're added to this test's own vertical-wall list by hand --
        // `standard_curves` still builds its fillets from the plain
        // `back_wall_plane` underneath either way.
        let mut vertical_walls: Vec<StaticPlane> = standard_walls()
            .into_iter()
            .filter(|w| w.normal.z == 0.0)
            .collect();
        vertical_walls.push(back_wall_plane(1.0));
        vertical_walls.push(back_wall_plane(-1.0));
        for curve in standard_curves() {
            let sits_radius_in_from_some_wall = vertical_walls.iter().any(|wall| {
                let distance = wall.signed_distance(&curve.axis_point);
                (distance - FILLET_RADIUS).abs() < 1e-3
                    || (distance - CORNER_ARCH_RADIUS).abs() < 1e-3
            });
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
    fn a_corner_edge_fillets_axis_sits_radius_in_from_both_the_side_wall_and_the_corner_wall() {
        let side = side_wall_plane(1.0);
        let corner = corner_wall_plane(1.0, 1.0);
        let pipe = StaticQuarterPipe::between_planes(
            &side,
            &corner,
            FILLET_RADIUS,
            Vec3::new(0.0, 0.0, 1.0),
        );
        assert!((side.signed_distance(&pipe.axis_point) - FILLET_RADIUS).abs() < 1e-3);
        assert!((corner.signed_distance(&pipe.axis_point) - FILLET_RADIUS).abs() < 1e-3);
    }

    #[test]
    fn a_corner_edge_fillets_sector_spans_a_shallower_angle_than_a_floor_seam_fillets() {
        // The corner wall meets its neighboring side wall at 135 degrees
        // (not the floor/ceiling seam's 90), so this fillet's own sector --
        // the angle between sector_start and sector_end -- should come out
        // noticeably smaller than 90 degrees (see
        // RB-PHYSICS-001-FR-022's own doc comment for the exact 45-degree
        // figure this specific arena geometry produces).
        let side = side_wall_plane(1.0);
        let corner = corner_wall_plane(1.0, 1.0);
        let pipe = StaticQuarterPipe::between_planes(
            &side,
            &corner,
            FILLET_RADIUS,
            Vec3::new(0.0, 0.0, 1.0),
        );
        let angle = pipe.sector_start.dot(&pipe.sector_end).acos();
        assert!(
            (angle - std::f32::consts::FRAC_PI_4).abs() < 1e-3,
            "expected a 45-degree sector, got {angle} radians"
        );
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

    #[test]
    fn standard_corner_fillets_has_sixteen_fillets() {
        assert_eq!(standard_corner_fillets().len(), 16);
    }

    #[test]
    fn every_standard_corner_fillets_center_sits_radius_in_from_a_floor_or_ceiling_a_side_or_back_wall_and_a_corner_wall(
    ) {
        // Each of the 16 fillets should sit exactly CORNER_ARCH_RADIUS
        // (RB-PHYSICS-001-FR-025 -- all 16 switched from FILLET_RADIUS
        // since each touches a corner wall's own floor/ceiling-seam arch,
        // which now uses that larger radius too) from *some* floor/ceiling
        // plane, *some* side/back wall, and *some* corner wall -- proving
        // `between_three_planes` actually solved for the real triple
        // intersection this arena's geometry produces, not just some
        // arbitrary point.
        let floor_and_ceiling = [standard_ground(), ceiling_plane()];
        let side_and_back_walls: Vec<StaticPlane> = [1.0f32, -1.0]
            .iter()
            .flat_map(|&s| [side_wall_plane(s), back_wall_plane(s)])
            .collect();
        let corner_walls: Vec<StaticPlane> = [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)]
            .iter()
            .map(|&(sx, sy)| corner_wall_plane(sx, sy))
            .collect();

        for fillet in standard_corner_fillets() {
            let sits_radius_in = |plane: &StaticPlane| {
                (plane.signed_distance(&fillet.center) - CORNER_ARCH_RADIUS).abs() < 1e-2
            };
            assert!(
                floor_and_ceiling.iter().any(sits_radius_in),
                "expected {:?} to sit radius-in from the floor or ceiling",
                fillet.center
            );
            assert!(
                side_and_back_walls.iter().any(sits_radius_in),
                "expected {:?} to sit radius-in from a side or back wall",
                fillet.center
            );
            assert!(
                corner_walls.iter().any(sits_radius_in),
                "expected {:?} to sit radius-in from a corner wall",
                fillet.center
            );
        }
    }

    #[test]
    fn standard_goal_walls_has_two_walls() {
        assert_eq!(standard_goal_walls().len(), 2);
    }

    #[test]
    fn both_goal_walls_share_one_offset_magnitude() {
        let walls = standard_goal_walls();
        assert_eq!(walls[0].plane.offset, walls[1].plane.offset);
    }

    #[test]
    fn each_goal_walls_window_is_centered_on_the_wall_at_half_the_goal_height() {
        for wall in standard_goal_walls() {
            assert!(
                (wall.plane.signed_distance(&wall.window_center)).abs() < 1e-3,
                "expected the window's own center to sit exactly on the wall, got {:?}",
                wall.window_center
            );
            assert!((wall.window_center.x).abs() < 1e-6);
            assert!((wall.window_center.z - GOAL_HEIGHT * 0.5).abs() < 1e-3);
            assert_eq!(wall.half_width, GOAL_HALF_WIDTH);
            assert_eq!(wall.half_height, GOAL_HEIGHT * 0.5);
        }
    }

    #[test]
    fn standard_goal_cutout_fillets_has_six_fillets() {
        assert_eq!(standard_goal_cutout_fillets().len(), 6);
    }

    #[test]
    fn every_goal_cutout_fillet_sits_radius_in_from_a_back_wall_and_a_post_or_crossbar_plane() {
        // Same proof `every_standard_curve_sits_radius_in_from_a_vertical_wall`
        // gives for the arena's other fillets: `between_planes` places its
        // axis exactly `radius` in from *each* of the two planes it
        // bridges, so every goal-cutout fillet's axis must sit exactly
        // `FILLET_RADIUS` from some back wall, and also from some
        // post/crossbar plane -- proof these fillets were actually derived
        // from real geometry, not just built with plausible-looking
        // numbers.
        let back_walls = [back_wall_plane(1.0), back_wall_plane(-1.0)];
        let post_and_crossbar_planes = [
            goal_post_plane(1.0),
            goal_post_plane(-1.0),
            goal_crossbar_plane(),
        ];
        let sits_radius_in = |plane: &StaticPlane, point: &Vec3| {
            (plane.signed_distance(point) - FILLET_RADIUS).abs() < 1e-2
        };

        for fillet in standard_goal_cutout_fillets() {
            assert!(
                back_walls
                    .iter()
                    .any(|p| sits_radius_in(p, &fillet.axis_point)),
                "expected {:?} to sit radius-in from a back wall",
                fillet.axis_point
            );
            assert!(
                post_and_crossbar_planes
                    .iter()
                    .any(|p| sits_radius_in(p, &fillet.axis_point)),
                "expected {:?} to sit radius-in from a post or crossbar plane",
                fillet.axis_point
            );
        }
    }

    #[test]
    fn standard_goal_corner_fillets_has_four_fillets() {
        assert_eq!(standard_goal_corner_fillets().len(), 4);
    }

    #[test]
    fn every_goal_corner_fillets_center_sits_radius_in_from_a_back_wall_a_post_and_the_crossbar() {
        // Each of the 4 fillets should sit exactly FILLET_RADIUS from
        // *some* back wall, *some* post plane, and the crossbar plane
        // simultaneously -- proving `between_three_planes` actually solved
        // for the real triple intersection this goal's geometry produces,
        // not just some arbitrary point (the same proof
        // `every_standard_corner_fillets_center_sits_radius_in_from_a_floor_or_ceiling_a_side_or_back_wall_and_a_corner_wall`
        // gives for the arena's own compound corners).
        let back_walls = [back_wall_plane(1.0), back_wall_plane(-1.0)];
        let post_planes = [goal_post_plane(1.0), goal_post_plane(-1.0)];
        let crossbar = goal_crossbar_plane();
        let sits_radius_in = |plane: &StaticPlane, point: &Vec3| {
            (plane.signed_distance(point) - FILLET_RADIUS).abs() < 1e-2
        };

        for fillet in standard_goal_corner_fillets() {
            assert!(
                back_walls.iter().any(|p| sits_radius_in(p, &fillet.center)),
                "expected {:?} to sit radius-in from a back wall",
                fillet.center
            );
            assert!(
                post_planes
                    .iter()
                    .any(|p| sits_radius_in(p, &fillet.center)),
                "expected {:?} to sit radius-in from a post plane",
                fillet.center
            );
            assert!(
                sits_radius_in(&crossbar, &fillet.center),
                "expected {:?} to sit radius-in from the crossbar",
                fillet.center
            );
        }
    }

    #[test]
    fn standard_goal_back_walls_has_two_walls() {
        assert_eq!(standard_goal_back_walls().len(), 2);
    }

    #[test]
    fn every_goal_back_wall_sits_goal_depth_behind_the_real_back_wall() {
        for wall in standard_goal_back_walls() {
            // The real back wall (at BACK_WALL_Y from center) should sit
            // exactly GOAL_DEPTH in front of this plane -- proving it's
            // positioned relative to the actual back wall, not just some
            // arbitrary distant point.
            let point_on_real_back_wall = wall.normal * -BACK_WALL_Y;
            assert!(
                (wall.signed_distance(&point_on_real_back_wall) - GOAL_DEPTH).abs() < 1e-2,
                "expected the real back wall to sit exactly GOAL_DEPTH in front of {wall:?}"
            );
        }
    }

    #[test]
    fn standard_goal_side_walls_has_four_walls() {
        assert_eq!(standard_goal_side_walls().len(), 4);
    }

    #[test]
    fn every_goal_side_walls_plane_matches_some_goal_post_plane() {
        let post_planes = [goal_post_plane(1.0), goal_post_plane(-1.0)];
        for wall in standard_goal_side_walls() {
            assert!(
                post_planes.contains(&wall.plane),
                "expected {:?} to reuse some goal_post_plane unchanged",
                wall.plane
            );
        }
    }

    #[test]
    fn every_goal_side_walls_bound_covers_the_real_goal_depth_and_height() {
        for wall in standard_goal_side_walls() {
            // The bound's own y-extent should span exactly from the real
            // back wall out to GOAL_DEPTH behind it -- one of its two
            // edges (center +/- half_u) should sit exactly at the real
            // back wall (|y| == BACK_WALL_Y), the other exactly at
            // GOAL_DEPTH behind it, regardless of which goal this is.
            let near_edge = wall.bound_center.y - wall.half_u;
            let far_edge = wall.bound_center.y + wall.half_u;
            let edges_abs = [near_edge.abs(), far_edge.abs()];
            assert!(
                edges_abs.iter().any(|e| (e - BACK_WALL_Y).abs() < 1e-2),
                "expected one bound edge to sit at the real back wall, got {edges_abs:?}"
            );
            assert!(
                edges_abs
                    .iter()
                    .any(|e| (e - (BACK_WALL_Y + GOAL_DEPTH)).abs() < 1e-2),
                "expected one bound edge to sit GOAL_DEPTH behind the real back wall, got {edges_abs:?}"
            );
            assert!(
                (wall.half_v - GOAL_HEIGHT * 0.5).abs() < 1e-2,
                "expected the bound's own half-height to match GOAL_HEIGHT * 0.5"
            );
            assert!((wall.bound_center.z - GOAL_HEIGHT * 0.5).abs() < 1e-2);
        }
    }

    #[test]
    fn standard_goal_roofs_has_two_roofs() {
        assert_eq!(standard_goal_roofs().len(), 2);
    }

    #[test]
    fn every_goal_roofs_plane_is_the_goal_crossbar_plane() {
        let crossbar = goal_crossbar_plane();
        for roof in standard_goal_roofs() {
            assert_eq!(roof.plane, crossbar);
        }
    }

    #[test]
    fn every_goal_roofs_bound_covers_the_real_goal_width() {
        for roof in standard_goal_roofs() {
            assert!((roof.half_u - GOAL_HALF_WIDTH).abs() < 1e-2);
            assert!((roof.bound_center.x).abs() < 1e-2);
        }
    }

    #[test]
    fn standard_nets_has_two_nets() {
        assert_eq!(standard_nets().len(), 2);
    }

    #[test]
    fn every_net_sits_net_depth_behind_the_real_back_wall_and_spans_the_goal_mouth() {
        for net in standard_nets() {
            // Every net point (anchored or free) starts on the flat grid at
            // exactly y = +-(BACK_WALL_Y + NET_DEPTH) -- proving the panel's
            // own depth, not just its existence.
            let y_values: Vec<f32> = net.points.iter().map(|p| p.position.y).collect();
            let target = y_values[0].abs();
            assert!(
                (target - (BACK_WALL_Y + NET_DEPTH)).abs() < 1e-2,
                "expected every net point at |y|={}, got {target}",
                BACK_WALL_Y + NET_DEPTH
            );
            for y in &y_values {
                assert!((y.abs() - target).abs() < 1e-2);
            }

            // The grid's own corner points sit exactly at the goal mouth's
            // own rim -- the same GOAL_HALF_WIDTH/GOAL_HEIGHT footprint
            // `standard_goal_walls`' own window uses.
            let xs: Vec<f32> = net.points.iter().map(|p| p.position.x).collect();
            let zs: Vec<f32> = net.points.iter().map(|p| p.position.z).collect();
            let min_x = xs.iter().cloned().fold(f32::INFINITY, f32::min);
            let max_x = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let min_z = zs.iter().cloned().fold(f32::INFINITY, f32::min);
            let max_z = zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            assert!((min_x - (-GOAL_HALF_WIDTH)).abs() < 1e-2);
            assert!((max_x - GOAL_HALF_WIDTH).abs() < 1e-2);
            assert!((min_z - 0.0).abs() < 1e-2);
            assert!((max_z - GOAL_HEIGHT).abs() < 1e-2);
        }
    }
}
