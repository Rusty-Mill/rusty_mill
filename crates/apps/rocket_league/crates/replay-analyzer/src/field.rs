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

/// Maximum raw replicated boost value; full boost.
pub const BOOST_MAX_BYTE: u8 = 255;

/// Convert a raw replicated boost byte (`0..=255`) to a percentage (`0..=100`).
pub fn boost_percent(byte: u8) -> f32 {
    byte as f32 / (BOOST_MAX_BYTE as f32) * 100.0
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
