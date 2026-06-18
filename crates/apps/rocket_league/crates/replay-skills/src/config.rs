//! Versioned, data-driven skill-detection thresholds.
//!
//! Every threshold a detector keys off lives here, never hard-coded into
//! [`crate::detect`], so detection is reproducible and tunable without touching
//! detector code. The built-in [`SkillConfig::default`] is a *pre-calibration*
//! starting point — sensible geometry-driven guesses, not corpus-fit numbers.
//! Changing any default MUST bump [`SKILL_CONFIG_VERSION`] (mirrors the scoring
//! crate's `SCORE_CONFIG_VERSION` discipline).

use serde::{Deserialize, Serialize};

/// Version stamped onto every [`crate::report::SkillReport`]; bump when the
/// defaults below change so a detection result is always reproducible.
pub const SKILL_CONFIG_VERSION: &str = "skcfg-v2";

/// Thresholds for the kinematic skill detectors. All distances are unreal units
/// (uu), speeds uu/s, times seconds, boost a raw replicated byte (0..=255).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillConfig {
    /// Stamped onto the report for reproducibility.
    pub version: String,

    // --- Aerial / air dribble ---
    /// Min car height (z) for a touch to count as an aerial.
    pub aerial_min_height: f32,
    /// Car height at which an aerial scores full confidence.
    pub high_aerial_height: f32,
    /// Min ball height (z) for an aerial — excludes low ground scoops.
    pub aerial_min_ball_height: f32,
    /// Max gap between consecutive aerial touches to chain an air dribble.
    pub air_dribble_window_s: f32,
    /// Min number of chained aerial touches to call it an air dribble.
    pub air_dribble_min_touches: usize,
    /// Max gap between a player's two contacts to count as one double touch.
    pub double_touch_window_s: f32,
    /// Min ball height at the second contact for a double touch (excludes a
    /// ground dribble's low micro-touches).
    pub double_touch_min_ball_height: f32,

    // --- Wall / ceiling ---
    /// Distance from a wall plane (|x|=side, |y|=back) to be "on the wall".
    pub wall_plane_tol: f32,
    /// Min height up the wall for a wall touch (excludes ground play near a wall).
    pub wall_min_height: f32,
    /// Distance below the ceiling to count as "at the ceiling".
    pub ceiling_tol: f32,
    /// Min time spent at the ceiling to register a ceiling play.
    pub ceiling_min_duration_s: f32,

    // --- Ground dribble / flick ---
    /// Min sustained duration to register a ground dribble.
    pub dribble_min_duration_s: f32,
    /// Ball height band (z) for a ball balanced on the car: lower bound.
    pub dribble_ball_height_min: f32,
    /// Ball height band (z) for a ball balanced on the car: upper bound.
    pub dribble_ball_height_max: f32,
    /// Max horizontal ball↔car distance for the ball to count as carried.
    pub dribble_horiz_radius: f32,
    /// Max carrier height (z) — the dribbler is on the ground.
    pub dribble_carrier_max_z: f32,
    /// Min upward ball velocity gained on the release touch of a flick.
    pub flick_min_up_dv: f32,
    /// Max incoming ball speed for a flick — the ball was carried, not driven in.
    pub flick_max_incoming: f32,

    // --- Striking ---
    /// Min resulting ball speed for a power shot.
    pub power_shot_min_speed: f32,
    /// Fraction of the resulting speed that must point at the opponent goal.
    pub power_shot_min_goalward: f32,
    /// Min direction change (degrees) between incoming and outgoing ball for a redirect.
    pub redirect_min_angle_deg: f32,
    /// Min incoming ball speed for a redirect — ignores carries / soft taps.
    pub redirect_min_incoming: f32,
    /// Min resulting ball speed for a redirect.
    pub redirect_min_speed: f32,

    // --- Speed ---
    /// Min sustained time at supersonic to register the skill (de-spams blips).
    pub supersonic_min_duration_s: f32,
    /// Hysteresis: a supersonic run *continues* until speed drops below this,
    /// so one sustained sprint isn't fragmented by ticks under the entry speed.
    pub supersonic_release_speed: f32,

    // --- Kickoff ---
    /// Window after a kickoff in which the first touch is attributed to it.
    pub kickoff_touch_window_s: f32,

    // --- Boost ---
    /// Boost byte at/above which a pickup is a full (big) pad.
    pub boost_steal_min_amount: u8,
    /// Min boost-byte jump in one step to count as a pad pickup.
    pub boost_steal_min_gain: u8,
    /// Min attacking-frame y (opponent half) for a pickup to be a "steal".
    pub boost_steal_min_y: f32,

    // --- Matching ---
    /// Tolerance for binding a touch event to its grid frame.
    pub touch_frame_tol_s: f32,
}

impl Default for SkillConfig {
    fn default() -> Self {
        SkillConfig {
            version: SKILL_CONFIG_VERSION.to_string(),

            aerial_min_height: 300.0,
            high_aerial_height: 900.0,
            aerial_min_ball_height: 250.0,
            air_dribble_window_s: 2.0,
            air_dribble_min_touches: 2,
            double_touch_window_s: 0.8,
            double_touch_min_ball_height: 300.0,

            wall_plane_tol: 150.0,
            wall_min_height: 300.0,
            ceiling_tol: 200.0,
            ceiling_min_duration_s: 0.1,

            dribble_min_duration_s: 0.75,
            dribble_ball_height_min: 120.0,
            dribble_ball_height_max: 350.0,
            dribble_horiz_radius: 140.0,
            dribble_carrier_max_z: 120.0,
            flick_min_up_dv: 550.0,
            flick_max_incoming: 800.0,

            power_shot_min_speed: 2000.0,
            power_shot_min_goalward: 0.6,
            redirect_min_angle_deg: 55.0,
            redirect_min_incoming: 700.0,
            redirect_min_speed: 1200.0,

            supersonic_min_duration_s: 0.5,
            supersonic_release_speed: 2100.0,

            kickoff_touch_window_s: 6.0,

            boost_steal_min_amount: 250,
            boost_steal_min_gain: 100,
            boost_steal_min_y: 2000.0,

            touch_frame_tol_s: 0.05,
        }
    }
}
