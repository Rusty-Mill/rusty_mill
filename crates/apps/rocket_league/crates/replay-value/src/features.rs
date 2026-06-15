//! State featurization.
//!
//! A *state* is one resampled grid frame viewed from a chosen team's attacking
//! frame (that team attacks `+Y`). Features are deliberately team-symmetric:
//! the same physical moment yields mirror-image feature vectors for the two
//! teams, which both doubles the data and bakes in the symmetry the value model
//! should respect. Everything here is a pure function of the canonical model.

use replay_analyzer::analyze::normalize::attacking_frame;
use replay_analyzer::field::{boost_percent, BACK_WALL_Y, CEILING_Z, SIDE_WALL_X};
use replay_analyzer::model::{GridCar, GridFrame, Vec3};
use std::collections::BTreeMap;

/// Number of features in a state vector.
pub const N_FEATURES: usize = 10;

/// Human-readable feature names, index-aligned with [`StateFeatures::x`].
pub const FEATURE_NAMES: [&str; N_FEATURES] = [
    "ball_y",            // toward opponent goal (+), normalized by half-length
    "ball_x_abs",        // lateral offset from centre (0 central .. 1 wall)
    "ball_z",            // height (0 floor .. 1 ceiling)
    "ball_vy",           // ball velocity toward opponent goal
    "ball_speed",        // ball speed magnitude
    "min_att_dist",      // nearest attacker's distance to the ball
    "min_def_dist",      // nearest defender's distance to the ball
    "att_ahead_frac",    // fraction of attackers ahead of the ball (committed)
    "def_goalside_frac", // fraction of defenders goal-side of the ball
    "boost_diff",        // mean attacker boost − mean defender boost (fraction)
];

/// Velocity normalization scale (uu/s); a hard ball clear approaches this.
const SPEED_SCALE: f32 = 6000.0;
/// Field diagonal (uu): sqrt((2·4096)² + (2·5120)²) ≈ 13113.6. Distance scale.
const FIELD_DIAG: f32 = 13_113.6;

/// One sampled game state as a feature vector from `team`'s attacking frame.
#[derive(Debug, Clone, PartialEq)]
pub struct StateFeatures {
    pub t: f32,
    pub team: i32,
    pub x: [f32; N_FEATURES],
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn mean_boost(cars: &[&GridCar]) -> f32 {
    let (mut sum, mut n) = (0.0f32, 0usize);
    for c in cars {
        if let Some(b) = c.boost {
            sum += boost_percent(b);
            n += 1;
        }
    }
    if n > 0 {
        sum / n as f32
    } else {
        0.0
    }
}

/// Build features for `team` from an already attacking-frame-rotated grid frame.
/// `None` when there is no ball or no live car for `team` this frame.
pub fn features_from_attacking(af: &GridFrame, team: i32) -> Option<StateFeatures> {
    let ball = af.ball?;
    let att: Vec<&GridCar> = af.cars.iter().filter(|c| c.team == Some(team)).collect();
    let def: Vec<&GridCar> = af
        .cars
        .iter()
        .filter(|c| c.team.is_some() && c.team != Some(team))
        .collect();
    if att.is_empty() {
        return None;
    }

    let by = ball.p.y;
    let speed = (ball.v.x * ball.v.x + ball.v.y * ball.v.y + ball.v.z * ball.v.z).sqrt();

    let min_att = att
        .iter()
        .map(|c| dist(c.p, ball.p))
        .fold(f32::INFINITY, f32::min);
    let min_def = def
        .iter()
        .map(|c| dist(c.p, ball.p))
        .fold(f32::INFINITY, f32::min);
    let min_def = if min_def.is_finite() {
        min_def
    } else {
        FIELD_DIAG
    };

    let att_ahead = att.iter().filter(|c| c.p.y > by).count() as f32 / att.len() as f32;
    let def_goalside = if def.is_empty() {
        0.0
    } else {
        def.iter().filter(|c| c.p.y > by).count() as f32 / def.len() as f32
    };

    Some(StateFeatures {
        t: af.t,
        team,
        x: [
            by / BACK_WALL_Y,
            ball.p.x.abs() / SIDE_WALL_X,
            ball.p.z / CEILING_Z,
            ball.v.y / SPEED_SCALE,
            speed / SPEED_SCALE,
            (min_att / FIELD_DIAG).min(1.0),
            (min_def / FIELD_DIAG).min(1.0),
            att_ahead,
            def_goalside,
            (mean_boost(&att) - mean_boost(&def)) / 100.0,
        ],
    })
}

/// Build features for `team` from a world-frame grid frame and the team signs.
pub fn state_features(
    frame: &GridFrame,
    team: i32,
    signs: &BTreeMap<i32, i32>,
) -> Option<StateFeatures> {
    features_from_attacking(&attacking_frame(frame, team, signs), team)
}

/// Index of the grid frame nearest `t` (frames are time-sorted ascending).
pub fn frame_index_at(frames: &[GridFrame], t: f32) -> Option<usize> {
    if frames.is_empty() {
        return None;
    }
    let i = frames.partition_point(|f| f.t < t);
    if i == 0 {
        return Some(0);
    }
    if i >= frames.len() {
        return Some(frames.len() - 1);
    }
    let prev = i - 1;
    if (frames[i].t - t).abs() <= (t - frames[prev].t).abs() {
        Some(i)
    } else {
        Some(prev)
    }
}
