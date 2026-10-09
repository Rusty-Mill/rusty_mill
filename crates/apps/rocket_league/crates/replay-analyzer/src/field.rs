//! Standard Soccar arena constants, in Rocket League unreal units (uu).
//!
//! All decoded positions are already in uu (boxcars divides raw fixed-point by
//! 100). These constants exist so reconstruction sanity-checks and downstream
//! consumers share one source of truth instead of scattering magic numbers.

/// Half-width of the field: side walls sit at `x = ±SIDE_WALL_X`.
pub const SIDE_WALL_X: f32 = 4096.0;
/// Half-length of the field: back walls / goal mouths sit at `y = ±BACK_WALL_Y`.
pub const BACK_WALL_Y: f32 = 5120.0;
/// Ceiling height: `z = CEILING_Z`.
pub const CEILING_Z: f32 = 2044.0;
/// Floor height.
pub const FLOOR_Z: f32 = 0.0;

/// Ball radius (uu). The ball *center* therefore rests at roughly this height
/// and can intrude one radius past a wall plane into the goal.
pub const BALL_RADIUS: f32 = 92.75;

/// Depth the ball can travel past the back-wall plane into the goal (uu).
pub const GOAL_DEPTH_Y: f32 = 880.0;

/// Supersonic speed threshold (uu/s): at or above this, a car is "supersonic"
/// (max ground speed is ~2300 uu/s).
pub const SUPERSONIC_SPEED: f32 = 2200.0;

/// "Boost speed" threshold (uu/s): the speed a car can only sustain while
/// boosting/dodging. Below this is "slow" (throttle only); at/above it up to
/// [`SUPERSONIC_SPEED`] is "boost speed" — mirrors ballchasing's speed buckets.
pub const BOOST_SPEED: f32 = 1400.0;

/// Maximum raw replicated boost value; full boost.
pub const BOOST_MAX_BYTE: u8 = 255;

/// Convert a raw replicated boost byte (`0..=255`) to a percentage (`0..=100`).
pub fn boost_percent(byte: u8) -> f32 {
    byte as f32 / (BOOST_MAX_BYTE as f32) * 100.0
}

/// The 6 **big** boost pads (full boost), as world `(x, y)` on the floor.
/// Standard Soccar layout (RLBot "Useful Game Values"); pads sit at `z≈73`.
pub const BIG_BOOST_PADS: [(f32, f32); 6] = [
    (3584.0, 0.0),
    (-3584.0, 0.0),
    (3072.0, 4096.0),
    (-3072.0, 4096.0),
    (3072.0, -4096.0),
    (-3072.0, -4096.0),
];

/// The 28 **small** boost pads (12 boost each), as world `(x, y)` on the floor.
pub const SMALL_BOOST_PADS: [(f32, f32); 28] = [
    (0.0, -4240.0),
    (-1792.0, -4184.0),
    (1792.0, -4184.0),
    (-940.0, -3308.0),
    (940.0, -3308.0),
    (0.0, -2816.0),
    (-3584.0, -2484.0),
    (3584.0, -2484.0),
    (-1788.0, -2300.0),
    (1788.0, -2300.0),
    (-2048.0, -1036.0),
    (0.0, -1024.0),
    (2048.0, -1036.0),
    (-1024.0, 0.0),
    (1024.0, 0.0),
    (-2048.0, 1036.0),
    (0.0, 1024.0),
    (2048.0, 1036.0),
    (-1788.0, 2300.0),
    (1788.0, 2300.0),
    (-3584.0, 2484.0),
    (3584.0, 2484.0),
    (0.0, 2816.0),
    (-940.0, 3308.0),
    (940.0, 3308.0),
    (-1792.0, 4184.0),
    (1792.0, 4184.0),
    (0.0, 4240.0),
];

/// Boost a small pad grants (percent); big pads fill to 100.
pub const SMALL_PAD_BOOST: f32 = 12.0;

/// Which kind of boost pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadKind {
    Big,
    Small,
}

/// Nominal boost a pad grants into an empty tank (percent).
pub fn pad_nominal(kind: PadKind) -> f32 {
    match kind {
        PadKind::Big => 100.0,
        PadKind::Small => SMALL_PAD_BOOST,
    }
}

/// The boost pad nearest a position (compared in the floor plane), with its kind,
/// world `(x, y)`, and distance. Boost only comes from pads in standard Soccar, so
/// a pickup attributes to the nearest pad of the relevant kind.
pub fn nearest_pad(p: [f32; 3], kind: PadKind) -> ((f32, f32), f32) {
    let pads: &[(f32, f32)] = match kind {
        PadKind::Big => &BIG_BOOST_PADS,
        PadKind::Small => &SMALL_BOOST_PADS,
    };
    let mut best = ((0.0, 0.0), f32::INFINITY);
    for &(px, py) in pads {
        let d = ((p[0] - px).powi(2) + (p[1] - py).powi(2)).sqrt();
        if d < best.1 {
            best = ((px, py), d);
        }
    }
    best
}

/// The nearest pad of **either** kind to a position (kind, `(x,y)`, distance).
pub fn nearest_pad_any(p: [f32; 3]) -> (PadKind, (f32, f32), f32) {
    let (big_pos, big_d) = nearest_pad(p, PadKind::Big);
    let (small_pos, small_d) = nearest_pad(p, PadKind::Small);
    if big_d <= small_d {
        (PadKind::Big, big_pos, big_d)
    } else {
        (PadKind::Small, small_pos, small_d)
    }
}

/// Canonical kickoff spawn locations, as `(|x|, |y|)` magnitude pairs. The five
/// standard kickoff positions (two diagonal corners, two back, one far-back
/// center) collapse to these three magnitudes by side/team symmetry. Every car
/// spawns on one of them; used to validate kickoff-frame geometry.
pub const KICKOFF_SPAWNS: [(f32, f32); 3] = [
    (2048.0, 2560.0), // diagonal corners
    (256.0, 3840.0),  // back
    (0.0, 4608.0),    // far-back center
];

/// True if a ball *center* position is physically inside the arena envelope,
/// allowing one ball-radius of wall intrusion and the goal recess on `y`.
///
/// This is a garbage-detector: a bad decode yields wild or non-finite values,
/// not merely a ball pressed against a wall. `tol` widens the envelope further.
pub fn ball_in_arena(p: [f32; 3], tol: f32) -> bool {
    p.iter().all(|c| c.is_finite())
        && p[0].abs() <= SIDE_WALL_X + BALL_RADIUS + tol
        && p[1].abs() <= BACK_WALL_Y + GOAL_DEPTH_Y + tol
        && p[2] >= FLOOR_Z - tol
        && p[2] <= CEILING_Z + BALL_RADIUS + tol
}

/// True if a ground-level position matches a canonical kickoff spawn (compared
/// on coordinate magnitudes, so it is side/team independent).
pub fn is_kickoff_spawn(p: [f32; 3], tol: f32) -> bool {
    let (ax, ay) = (p[0].abs(), p[1].abs());
    KICKOFF_SPAWNS
        .iter()
        .any(|(sx, sy)| (ax - sx).abs() <= tol && (ay - sy).abs() <= tol)
}

/// Arena geometry as **data** (vs. the bare constants above), so it can vary by
/// map. Physics constants (ball radius, supersonic speed) are map-independent and
/// stay as module constants; this captures only the arena envelope a non-standard
/// map would change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldGeometry {
    pub side_wall_x: f32,
    pub back_wall_y: f32,
    pub ceiling_z: f32,
    pub goal_depth_y: f32,
}

impl FieldGeometry {
    /// Standard Soccar geometry (the module constants).
    pub const fn standard() -> Self {
        FieldGeometry {
            side_wall_x: SIDE_WALL_X,
            back_wall_y: BACK_WALL_Y,
            ceiling_z: CEILING_Z,
            goal_depth_y: GOAL_DEPTH_Y,
        }
    }
}

impl Default for FieldGeometry {
    fn default() -> Self {
        FieldGeometry::standard()
    }
}

/// How a replay's map relates to standard Soccar geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapClass {
    /// Standard Soccar geometry — the competitive arenas are cosmetic reskins
    /// with identical collision, so kinematic analysis is valid.
    Standard,
    /// A recognized non-standard arena or game mode (different field shape/size or
    /// goals). Geometry-derived analysis (kickoffs, walls/ceiling, field thirds,
    /// the drawn field) is **unreliable** and should be flagged.
    NonStandard,
}

/// MapName substrings (lowercased) for known **non-standard-geometry** maps and
/// modes. Deliberately conservative — only clearly different fields — so the
/// overwhelmingly-common standard arenas are never misflagged. Extend as more
/// maps are confirmed.
const NON_STANDARD_MAPS: &[&str] = &[
    "hoops", // basketball: smaller field, elevated round goals
    "hoopsstadium",
    "shattershot", // Dropshot: hexagonal floor, no back-wall goals
    "octagon",
    "pillars",
    "badlands",
    "starbase", // Starbase ARC: curved/oval field, raised centre
];

/// Classify a replay's `MapName` (the header value, e.g. `"EuroStadium_P"`).
///
/// Recognized non-standard maps/modes are flagged; everything else is treated as
/// standard, since the standard competitive arenas (different *names*, identical
/// geometry) are by far the common case and `None`/unknown names are almost
/// always standard Soccar.
pub fn classify_map(map: Option<&str>) -> MapClass {
    match map {
        Some(m) => {
            let key = m.to_ascii_lowercase();
            if NON_STANDARD_MAPS.iter().any(|p| key.contains(p)) {
                MapClass::NonStandard
            } else {
                MapClass::Standard
            }
        }
        None => MapClass::Standard,
    }
}

/// Whether geometry-derived analysis is trustworthy for this map.
pub fn is_standard_geometry(map: Option<&str>) -> bool {
    classify_map(map) == MapClass::Standard
}

/// Arena geometry for a map. Currently always [`FieldGeometry::standard`]:
/// non-standard arenas keep standard geometry as the best available value until
/// real collision dimensions are measured — use [`classify_map`] to flag them.
pub fn geometry_for_map(_map: Option<&str>) -> FieldGeometry {
    FieldGeometry::standard()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_and_cosmetic_arenas_classify_standard() {
        // The two test-sample maps (Mannfield, DFH Stadium) and any unknown name.
        for m in [
            "EuroStadium_P",
            "Stadium_P",
            "CS_P",
            "Park_Night_P",
            "BrandNew_P",
        ] {
            assert_eq!(classify_map(Some(m)), MapClass::Standard, "{m}");
        }
        assert_eq!(classify_map(None), MapClass::Standard);
        assert!(is_standard_geometry(Some("Stadium_P")));
    }

    #[test]
    fn known_non_standard_maps_are_flagged() {
        for m in ["HoopsStadium_P", "ShatterShot_P", "Octagon_P", "Starbase_P"] {
            assert_eq!(classify_map(Some(m)), MapClass::NonStandard, "{m}");
            assert!(!is_standard_geometry(Some(m)), "{m}");
        }
    }

    #[test]
    fn geometry_for_map_is_standard_for_now() {
        assert_eq!(
            geometry_for_map(Some("HoopsStadium_P")),
            FieldGeometry::standard()
        );
    }
}
