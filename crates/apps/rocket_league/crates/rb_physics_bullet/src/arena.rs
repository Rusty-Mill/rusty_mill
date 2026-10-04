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
//! **Since `RB-PHYSICS-001-FR-106` (ADR-0025) the side ramps and the four
//! corners are Rocket League's own collision triangles**
//! (`standard_meshes`), replacing the corner walls' planes, the side and
//! corner seams, the vertical corner edges and FR-102's swept corner
//! fillets described above and below: the facets sit up to ~2.5 uu inside
//! any smooth fit, and the owner's corner-wall ride touches them. Only the
//! back walls' seams (`standard_curves`) and the goal mouth stay analytic.
//!
//! **Since `RB-PHYSICS-001-FR-113` (ADR-0033) the goals and back walls are
//! mesh too** (`standard_goal_meshes`), so `PhysicsWorld::standard_arena`
//! is exactly RocketSim's `Arena::_SetupArenaCollisionShapes`: the ground,
//! side walls and ceiling as planes, everything else as the game's
//! triangles (RocketSim's own 16 mesh files since FR-117, ADR-0037).
//! `standard_curves`, the goal walls, goal boxes, goal fillets and nets
//! below are no longer part of it; they remain as built and tested shapes.
//! FR-102's measured radii survive for the back seams
//! (`BACK_FLOOR_RADIUS`, `CEILING_RADIUS`).
//!
//! Since `RB-PHYSICS-001-FR-112` (ADR-0032) `PhysicsWorld::standard_arena`
//! leaves out `standard_goal_cutout_fillets` and
//! `standard_goal_corner_fillets`: built concave between the back wall and
//! the post/crossbar planes, they filled the goal mouth itself, and a ball
//! shot into the goal bounced out (`hitjump.jsonl` 56.3 s). The back-wall
//! floor seam stops at the goal posts for the same reason. Both functions
//! remain, described below as built.
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
//! the crossbar) — reusing `GOAL_FILLET_RADIUS` unchanged, since (unlike
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
//! **Still not modeled**: the goal mesh's own shape (its fillets keep the
//! `GOAL_FILLET_RADIUS` placeholder), and any detail of the real field
//! mesh finer than these planes, cylinders and tori. See `RB-PHYSICS-001`'s
//! Non-goals.

use crate::body::{
    StaticBoundedWall, StaticCornerFillet, StaticGoalWall, StaticPlane, StaticQuarterPipe,
};
use crate::mesh::StaticMesh;
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

/// Ceiling height along Z (`RB-PHYSICS-001-FR-036`). Confirmed against
/// RocketSim's own `ARENA_HEIGHT` constant (`src/RLConst.h`), used the same
/// way this port uses `CEILING_Z` — as the ceiling collision plane's
/// position measured from the same floor-at-zero datum, not a different
/// reference point — and independently cross-checked by fitting a circle to
/// RLUtilities' own extracted real-game collision-mesh vertices for the
/// wall-to-ceiling ramp, which lands within 0.25 units of `2048.0` (the
/// same fitting method reproduces the already-known floor and side-wall
/// positions to within 0.05 units, validating it). This port previously
/// used `2044.0`, an unrelated older community figure (the RLBot wiki's own
/// commit history traces it to a since-superseded pre-2018 measurement);
/// `2048.0` is the correct value, not a different convention.
pub const CEILING_Z: f32 = 2048.0;

/// How far back from the true rectangular corner (where a side wall would
/// meet a back wall at 90 degrees) each of the four diagonal corner walls
/// is inset, along both axes equally. Confirmed correct, not a placeholder
/// (`RB-PHYSICS-001-FR-036`): `SIDE_WALL_X - CORNER_LENGTH = 2944.0`, and
/// `2944.0 + BACK_WALL_Y = 8064.0` matches the diagonal-corner-plane
/// intercept `RB-PHYSICS-001-FR-036`'s research independently measured
/// from RLUtilities' real extracted collision-mesh vertices (a flat
/// diagonal plane at 45 degrees intersecting both axes at ±8064) — this
/// project's earlier claim that it was an uncalibrated placeholder chosen
/// only for a "recognizably octagonal" look was itself incorrect. The real
/// arena's corners aren't a single flat 45-degree cut all the way to the
/// floor/ceiling seam (they curve, and blend into ramps this port doesn't
/// model either), but the flat corner-wall plane itself is exact.
pub const CORNER_LENGTH: f32 = 1152.0;

/// Radii of the arena's curved transitions (`RB-PHYSICS-001-FR-102`,
/// ADR-0022), fitted to the real soccar collision mesh as extracted in
/// RLUtilities (`samuelpmish/RLUtilities`, `assets/soccar/`, GPL-3.0; only measured
/// dimensions are taken, no data is copied): its
/// floor-ramp, ceiling-ramp and corner vertices lie on circles of these
/// radii to within 0.5 uu (the 864 uu edges to within 2 uu). They replace
/// FR-025's uncalibrated 292/750 uu placeholders. The flat walls they join
/// (`SIDE_WALL_X`, `BACK_WALL_Y`, `CORNER_LENGTH`, `CEILING_Z`) are
/// tangent to them exactly in that mesh too.
///
/// Since `RB-PHYSICS-001-FR-106` (ADR-0025) the side ramps and corners are
/// the real triangle meshes (`standard_meshes`); only the back walls' seams
/// stay analytic.
///
/// Back wall to floor: tighter than every other floor ramp.
pub const BACK_FLOOR_RADIUS: f32 = 160.0;
/// Every wall to the ceiling.
pub const CEILING_RADIUS: f32 = 512.0;

/// Uncalibrated placeholder: the radius rounding the goal mouth's posts and
/// crossbar (`standard_goal_cutout_fillets`, `standard_goal_corner_fillets`).
/// Kept at the arena's old 292 uu fillet value: FR-102 calibrated only the
/// arena's own transitions, not the goal mesh.
pub const GOAL_FILLET_RADIUS: f32 = 292.0;

/// Half-width of the goal-mouth window cut into each back wall — a
/// commonly-cited community-measured Rocket League dimension (same
/// sourcing tier as `SIDE_WALL_X`). Confirmed exact against the current
/// RLBot wiki's own cited "Goal center-to-post" value
/// (`RB-PHYSICS-001-FR-055`, fetched directly rather than assumed from
/// `RB-PHYSICS-001-FR-036`'s prior research into the same wiki page) —
/// no longer merely "commonly-cited", the same upgrade `GOAL_DEPTH`
/// itself already got from FR-036.
pub const GOAL_HALF_WIDTH: f32 = 892.755;

/// Height of the goal-mouth window — same sourcing tier as
/// `GOAL_HALF_WIDTH`, and confirmed the same way and at the same time
/// (`RB-PHYSICS-001-FR-055`), against the wiki's own cited "Goal height"
/// value.
pub const GOAL_HEIGHT: f32 = 642.775;

/// How far behind the back wall the goal box's own interior extends
/// (`RB-PHYSICS-001-FR-029`). Confirmed against the current RLBot wiki's
/// own cited goal depth (`RB-PHYSICS-001-FR-036`'s research) — not an
/// uncalibrated placeholder, despite this project's earlier claim (from
/// `RB-PHYSICS-001-FR-031`) that no reference existed for it at all.
pub const GOAL_DEPTH: f32 = 880.0;

/// How far behind the real back wall a goal's `net::NetMesh` panel sits
/// (`RB-PHYSICS-001-FR-033`) — deliberately less than `GOAL_DEPTH`, so a
/// ball entering the goal always meets the springy net well before it could
/// ever reach `goal_back_wall_plane`'s own rigid backstop (still there,
/// completely unchanged, as a safety net *behind* the net for the
/// vanishingly unlikely case the mesh's own solve lets the ball slip past
/// it — see `net::NetMesh`'s own doc comment for what a car, which isn't
/// tested against the mesh at all, still collides with instead). Unlike
/// `GOAL_DEPTH` itself (confirmed, see above), this fraction of it is
/// this project's own uncalibrated invention — no reference exists for
/// how far into the goal a real net actually hangs, only for the goal
/// box's own total depth.
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

/// The arena's flat boundary planes besides the floor and back walls: the
/// 2 side walls (`+-X`) and the ceiling, each with its normal pointing
/// into the playable volume. Since `RB-PHYSICS-001-FR-106` the diagonal
/// corner walls are part of the corner meshes (`standard_meshes`), whose
/// flat diagonal facets lie on the plane `|x| + |y| = 8064` (`CORNER_LENGTH`);
/// a plane there as well would contact the same corners twice. The back
/// walls carry the goal mouth (`standard_goal_walls`).
pub fn standard_walls() -> Vec<StaticPlane> {
    vec![side_wall_plane(1.0), side_wall_plane(-1.0), ceiling_plane()]
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
                GOAL_FILLET_RADIUS,
                Vec3::new(0.0, 0.0, 1.0),
            ));
        }
        fillets.push(StaticQuarterPipe::between_planes(
            &wall,
            &crossbar,
            GOAL_FILLET_RADIUS,
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
/// `between_three_planes`'s own doc comment). Reuses `GOAL_FILLET_RADIUS`
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
                GOAL_FILLET_RADIUS,
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

/// The back walls' floor and ceiling seams (`RB-PHYSICS-001-FR-020`, radii
/// since FR-102): 4 `StaticQuarterPipe`s, floor then ceiling for `+Y` then
/// `-Y`. Since `RB-PHYSICS-001-FR-106` the side ramps, the corners and
/// their seams are the real meshes (`standard_meshes`); the back walls
/// belong to the goal mesh, which is not modeled, so they keep these.
pub fn standard_curves() -> Vec<StaticQuarterPipe> {
    let floor = standard_ground();
    let ceiling = ceiling_plane();
    let along = Vec3::new(1.0, 0.0, 0.0);
    [1.0f32, -1.0]
        .into_iter()
        .flat_map(|sign| {
            let wall = back_wall_plane(sign);
            let seam = StaticQuarterPipe::between_planes(&floor, &wall, BACK_FLOOR_RADIUS, along);
            // The goal floor runs flat into the goal: no seam across its
            // mouth (`RB-PHYSICS-001-FR-112`).
            let far = Vec3::new(SIDE_WALL_X, 0.0, 0.0);
            let post = Vec3::new(GOAL_HALF_WIDTH, 0.0, 0.0);
            [
                seam.with_span(-far, -post),
                seam.with_span(post, far),
                StaticQuarterPipe::between_planes(&ceiling, &wall, CEILING_RADIUS, along),
            ]
        })
        .collect()
}

/// Inside the arena, for orienting mesh triangles: the arena is convex, so
/// every boundary triangle faces this point.
const ARENA_INSIDE: Vec3 = Vec3::new(0.0, 0.0, CEILING_Z * 0.5);

/// The arena's curved parts as Rocket League's own collision triangles
/// (`RB-PHYSICS-001-FR-106`, ADR-0025): RocketSim's 16 soccar mesh files
/// (`assets/soccar/README.md`, Apache-2.0) in its load order, each
/// reporting triangles in Bullet's BVH order (FR-117, ADR-0037). 4 corners
/// (0-3), 4 goal halves with their back wall (4-7), 2 ceiling ramps (8,
/// 9), 4 floor ramps (10-13), 2 ceiling ramps (14, 15).
pub fn standard_meshes() -> Vec<StaticMesh> {
    const FILES: [&[u8]; 16] = [
        include_bytes!("../assets/soccar/mesh_0.cmf"),
        include_bytes!("../assets/soccar/mesh_1.cmf"),
        include_bytes!("../assets/soccar/mesh_2.cmf"),
        include_bytes!("../assets/soccar/mesh_3.cmf"),
        include_bytes!("../assets/soccar/mesh_4.cmf"),
        include_bytes!("../assets/soccar/mesh_5.cmf"),
        include_bytes!("../assets/soccar/mesh_6.cmf"),
        include_bytes!("../assets/soccar/mesh_7.cmf"),
        include_bytes!("../assets/soccar/mesh_8.cmf"),
        include_bytes!("../assets/soccar/mesh_9.cmf"),
        include_bytes!("../assets/soccar/mesh_10.cmf"),
        include_bytes!("../assets/soccar/mesh_11.cmf"),
        include_bytes!("../assets/soccar/mesh_12.cmf"),
        include_bytes!("../assets/soccar/mesh_13.cmf"),
        include_bytes!("../assets/soccar/mesh_14.cmf"),
        include_bytes!("../assets/soccar/mesh_15.cmf"),
    ];
    FILES
        .iter()
        .filter_map(|file| StaticMesh::from_cmf(file))
        .collect()
}

/// The goals and the back walls around them (`RB-PHYSICS-001-FR-113`):
/// the `+y` goal's two halves (`x >= 0`, then `x <= 0`), then the `-y`
/// goal's (`x <= 0`, then `x >= 0`), each spanning `|x| <= 2688` from the
/// field side of the back wall to the back of the goal.
pub fn standard_goal_meshes() -> Vec<StaticMesh> {
    let mut meshes = standard_meshes();
    meshes.drain(..4);
    meshes.truncate(4);
    meshes
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn standard_walls_has_three_planes() {
        assert_eq!(standard_walls().len(), 3);
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
    fn standard_curves_are_the_back_walls_floor_and_ceiling_seams() {
        // Per back wall: the floor seam either side of the goal mouth
        // (RB-PHYSICS-001-FR-112), then the ceiling seam.
        let curves = standard_curves();
        assert_eq!(curves.len(), 6);
        for seams in curves.chunks(3) {
            for floor in &seams[..2] {
                assert!((floor.radius - BACK_FLOOR_RADIUS).abs() < 1e-6);
                assert!((floor.axis_point.z - BACK_FLOOR_RADIUS).abs() < 1e-3);
                let wall_distance = BACK_WALL_Y - floor.axis_point.y.abs();
                assert!((wall_distance - BACK_FLOOR_RADIUS).abs() < 1e-2);
            }
            assert!((seams[2].radius - CEILING_RADIUS).abs() < 1e-6);
            assert!((seams[2].axis_point.z - (CEILING_Z - CEILING_RADIUS)).abs() < 1e-3);
            assert!(seams[2].span.0.is_infinite() && seams[2].span.1.is_infinite());
        }
    }

    /// RB-PHYSICS-001-FR-112: the back-wall floor seam stops at the goal
    /// posts, so a point in the goal mouth meets no seam, and one beside
    /// the goal still does.
    #[test]
    fn the_back_wall_floor_seam_stops_at_the_goal_mouth() {
        let seams: Vec<_> = standard_curves()
            .into_iter()
            .filter(|c| (c.radius - BACK_FLOOR_RADIUS).abs() < 1e-6 && c.axis_point.y > 0.0)
            .collect();
        let corner_of_seam = |x: f32| Vec3::new(x, BACK_WALL_Y - 5.0, 5.0);
        let touched = |x: f32| {
            seams.iter().any(|s| {
                crate::collision::sphere_vs_quarter_pipe(corner_of_seam(x), 0.0, s).is_some()
            })
        };
        assert!(!touched(0.0), "the goal mouth has no seam");
        assert!(!touched(GOAL_HALF_WIDTH - 1.0));
        assert!(touched(GOAL_HALF_WIDTH + 1.0), "beside the goal it does");
        assert!(touched(-(GOAL_HALF_WIDTH + 1.0)));
    }

    #[test]
    fn standard_meshes_are_rocketsims_sixteen() {
        // RB-PHYSICS-001-FR-117: 4 corners (880 triangles), 4 goal halves
        // (983, minus any without area), 2 + 2 ceiling ramps (16) and 4
        // floor ramps (126), in RocketSim's load order.
        let counts: Vec<usize> = standard_meshes()
            .iter()
            .map(|mesh| mesh.triangles().len())
            .collect();
        assert_eq!(counts.len(), 16);
        assert_eq!(counts[..4], [880; 4]);
        assert!(
            counts[4..8].iter().all(|&n| n > 970 && n <= 983),
            "{counts:?}"
        );
        assert_eq!(counts[8..], [16, 16, 126, 126, 126, 126, 16, 16]);
    }

    #[test]
    fn every_corner_and_ramp_triangle_faces_the_arena() {
        // The goals (FR-113) face as wound: their roof faces down.
        let meshes = standard_meshes();
        for mesh in meshes[..4].iter().chain(&meshes[8..]) {
            for triangle in mesh.triangles() {
                let [a, b, c] = triangle.vertices;
                let centroid = (a + b + c) * (1.0 / 3.0);
                assert!(triangle.normal.dot(&(ARENA_INSIDE - centroid)) > 0.0);
            }
        }
    }

    /// RB-PHYSICS-001-FR-113: the goal mesh faces the way it is played on:
    /// the back wall toward the field, the goal's floor up, its roof down,
    /// its back toward the field; the mirrored goal the same way.
    #[test]
    fn goal_mesh_triangles_face_the_playable_side() {
        let goals = standard_goal_meshes();
        // The `x >= 0` half of each goal.
        let x = 300.0;
        for (mesh, toward_field) in [&goals[0], &goals[3]].into_iter().zip([-1.0f32, 1.0]) {
            let probe = |origin: Vec3, direction: Vec3| {
                mesh.raycast(origin, direction, 400.0).map(|hit| hit.normal)
            };
            let side = -toward_field;
            // Back wall above the goal, from the field.
            let wall = probe(
                Vec3::new(x, side * (BACK_WALL_Y - 100.0), 1200.0),
                Vec3::new(0.0, side, 0.0),
            )
            .expect("back wall");
            assert!(wall.y * toward_field > 0.9, "{wall:?}");
            // Goal roof, from inside the goal.
            let roof = probe(
                Vec3::new(x, side * (BACK_WALL_Y + 300.0), 300.0),
                Vec3::new(0.0, 0.0, 1.0),
            )
            .expect("goal roof");
            assert!(roof.z < -0.9, "{roof:?}");
            // Goal back, from inside the goal.
            let back = probe(
                Vec3::new(x, side * (BACK_WALL_Y + 500.0), 150.0),
                Vec3::new(0.0, side, 0.0),
            )
            .expect("goal back");
            assert!(back.y * toward_field > 0.5, "{back:?}");
        }
    }

    /// The first mesh hit straight along `direction` from `origin`.
    fn mesh_hit(origin: Vec3, direction: Vec3) -> Option<f32> {
        standard_meshes()
            .iter()
            .filter_map(|mesh| mesh.raycast(origin, direction, 2000.0))
            .map(|hit| hit.distance)
            .min_by(f32::total_cmp)
    }

    #[test]
    fn every_corners_flat_diagonal_lies_on_the_corner_wall_plane() {
        // Mirroring check: from the middle of each quadrant's corner wall,
        // 300 uu in and 1000 uu up, the wall is 300 uu along its normal.
        let middle = SIDE_WALL_X - CORNER_LENGTH * 0.5;
        let along = BACK_WALL_Y - CORNER_LENGTH * 0.5;
        for (sx, sy) in [(1.0f32, 1.0f32), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            let normal = Vec3::new(-sx, -sy, 0.0) * std::f32::consts::FRAC_1_SQRT_2;
            let on_wall = Vec3::new(sx * middle, sy * along, 1000.0);
            let distance = mesh_hit(on_wall + normal * 300.0, -normal)
                .expect("every quadrant has a corner wall");
            assert!((distance - 300.0).abs() < 0.5, "({sx}, {sy}): {distance}");
        }
    }

    #[test]
    fn a_side_floor_ramp_follows_its_256_uu_circle_to_within_a_facet() {
        // 128 uu out from the side wall the 256 uu ramp is at
        // z = 256 - sqrt(256^2 - 128^2); a facet sits at most ~1.3 uu above.
        for sign in [1.0f32, -1.0] {
            let x = sign * (SIDE_WALL_X - 128.0);
            let height = 300.0
                - mesh_hit(Vec3::new(x, 100.0, 300.0), Vec3::new(0.0, 0.0, -1.0))
                    .expect("the floor ramp is under the side wall");
            let circle = 256.0 - (256.0f32 * 256.0 - 128.0 * 128.0).sqrt();
            assert!(
                (0.0..1.5).contains(&(height - circle)),
                "side {sign}: ramp at {height}, circle {circle}"
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
        // `GOAL_FILLET_RADIUS` from some back wall, and also from some
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
            (plane.signed_distance(point) - GOAL_FILLET_RADIUS).abs() < 1e-2
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
        // Each of the 4 fillets should sit exactly GOAL_FILLET_RADIUS from
        // *some* back wall, *some* post plane, and the crossbar plane
        // simultaneously -- proving `between_three_planes` actually solved
        // for the real triple intersection this goal's geometry produces,
        // not just some arbitrary point (the same proof
        // once gave for the arena's own compound corners, before FR-102
        // replaced them, later replaced by the FR-106 meshes).
        let back_walls = [back_wall_plane(1.0), back_wall_plane(-1.0)];
        let post_planes = [goal_post_plane(1.0), goal_post_plane(-1.0)];
        let crossbar = goal_crossbar_plane();
        let sits_radius_in = |plane: &StaticPlane, point: &Vec3| {
            (plane.signed_distance(point) - GOAL_FILLET_RADIUS).abs() < 1e-2
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
