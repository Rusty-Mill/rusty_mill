//! State featurization.
//!
//! A *state* is one resampled grid frame viewed from a chosen team's attacking
//! frame (that team attacks `+Y`). Features are deliberately team-symmetric:
//! the same physical moment yields mirror-image feature vectors for the two
//! teams, which both doubles the data and bakes in the symmetry the value model
//! should respect. Everything here is a pure function of the canonical model.

use replay_analyzer::analyze::normalize::attacking_frame;
use replay_analyzer::field::{boost_percent, BACK_WALL_Y, CEILING_Z, SIDE_WALL_X};
use replay_analyzer::model::{Event, GridCar, GridFrame, Vec3};
use std::collections::BTreeMap;

/// Number of features in a state vector.
pub const N_FEATURES: usize = 22;

/// Human-readable feature names, index-aligned with [`StateFeatures::x`].
///
/// Indices 0–9 are the v1 set (kept in place so old index-based references hold);
/// 10–17 are the v2 additions — per-car velocity/orientation, numerical
/// advantage, and boost extremes — all derivable from the attack-frame grid and
/// team-symmetric, so they keep the mirror property the model relies on.
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
    // --- v2 additions ---
    "man_advantage",   // (attackers alive − defenders alive) / 3 — demo/numerical edge
    "ball_goal_dist",  // ball → opponent-goal-mouth distance (nonlinear shot proximity)
    "min_att_closing", // nearest attacker's closing speed toward the ball (+ = onto it)
    "min_def_closing", // nearest defender's closing speed toward the ball
    "att_facing_ball", // nearest attacker's forward·(to-ball) alignment (−1..1)
    "def_facing_ball", // nearest defender's forward·(to-ball) alignment (−1..1)
    "max_att_boost",   // most boost held by any attacker (a loaded threat)
    "max_def_boost",   // most boost held by any defender (can they challenge/clear)
    // --- v3: temporal / context (need the frame sequence + events) ---
    "ball_vy_trend",        // ball y-velocity now − ~0.4s ago (momentum toward goal)
    "time_since_att_touch", // seconds since the attacking team last touched (recency)
    "att_ball_carry",       // an attacker is carrying/dribbling the ball
    "def_ball_carry",       // a defender is carrying the ball
];

/// Look-back for the momentum trend (s) and caps for the carry / recency features.
const TREND_LAG_S: f32 = 0.4;
const CARRY_RADIUS: f32 = 170.0; // car↔ball horizontal distance to count as a carry
const CARRY_Z_LO: f32 = 80.0; // ball sitting above the car (roof) — lower bound
const CARRY_Z_HI: f32 = 260.0; // …and upper bound
const CARRY_Z_MAX: f32 = 450.0; // above this it's an aerial, not a ground carry
const RECENCY_CAP_S: f32 = 10.0; // cap/scale for "time since last touch"

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

/// Most boost (0–100) held by any car in the set; 0 if none carry a gauge.
fn max_boost(cars: &[&GridCar]) -> f32 {
    cars.iter()
        .filter_map(|c| c.boost.map(boost_percent))
        .fold(0.0, f32::max)
}

/// The car in `cars` nearest the ball, if any.
fn nearest_to_ball<'a>(cars: &[&'a GridCar], ball: Vec3) -> Option<&'a GridCar> {
    cars.iter()
        .copied()
        .min_by(|a, b| dist(a.p, ball).total_cmp(&dist(b.p, ball)))
}

/// Speed (uu/s, attack frame) at which `c` is approaching the ball: the component
/// of its velocity along the unit vector toward the ball. Positive = closing in,
/// negative = backing off. 0 when the car is on the ball (degenerate direction).
fn closing_speed(c: &GridCar, ball: Vec3) -> f32 {
    let (dx, dy, dz) = (ball.x - c.p.x, ball.y - c.p.y, ball.z - c.p.z);
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if len < 1.0 {
        return 0.0;
    }
    (c.v.x * dx + c.v.y * dy + c.v.z * dz) / len
}

/// Cosine of the angle between `c`'s forward axis (from yaw) and the horizontal
/// direction to the ball: +1 dead-on, −1 facing away, 0 broadside. 0 when the
/// car carries no orientation or sits on the ball.
fn facing_ball_cos(c: &GridCar, ball: Vec3) -> f32 {
    let Some(rot) = c.rot else {
        return 0.0;
    };
    let (tx, ty) = (ball.x - c.p.x, ball.y - c.p.y);
    let len = (tx * tx + ty * ty).sqrt();
    if len < 1.0 {
        return 0.0;
    }
    (rot.yaw.cos() * tx + rot.yaw.sin() * ty) / len
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

    // v2: numerical edge (demos/respawns thin a team), shot proximity, and the
    // velocity/orientation/boost of the car most likely to touch the ball next.
    let man_advantage = ((att.len() as f32 - def.len() as f32) / 3.0).clamp(-1.0, 1.0);
    // Opponent goal mouth sits at +Y in the attack frame (centre, on the floor).
    let goal = Vec3 {
        x: 0.0,
        y: BACK_WALL_Y,
        z: 0.0,
    };
    let ball_goal_dist = (dist(ball.p, goal) / FIELD_DIAG).min(1.0);
    let near_att = nearest_to_ball(&att, ball.p);
    let near_def = nearest_to_ball(&def, ball.p);
    let min_att_closing = near_att.map(|c| closing_speed(c, ball.p)).unwrap_or(0.0) / SPEED_SCALE;
    let min_def_closing = near_def.map(|c| closing_speed(c, ball.p)).unwrap_or(0.0) / SPEED_SCALE;
    let att_facing = near_att.map(|c| facing_ball_cos(c, ball.p)).unwrap_or(0.0);
    let def_facing = near_def.map(|c| facing_ball_cos(c, ball.p)).unwrap_or(0.0);

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
            man_advantage,
            ball_goal_dist,
            min_att_closing,
            min_def_closing,
            att_facing,
            def_facing,
            max_boost(&att) / 100.0,
            max_boost(&def) / 100.0,
            // v3 temporal/context slots — 0 for a bare single frame; filled by
            // `features_at`, which has the frame sequence + events.
            0.0, // ball_vy_trend
            0.0, // time_since_att_touch
            0.0, // att_ball_carry
            0.0, // def_ball_carry
        ],
    })
}

/// True (1.0) if any car in `cars` is carrying the ball (balanced on its roof:
/// close horizontally, ball sitting in the roof-height band, not an aerial).
fn carrying(cars: &[&GridCar], ball: Vec3) -> f32 {
    if ball.z > CARRY_Z_MAX {
        return 0.0;
    }
    for c in cars {
        let (dx, dy) = (ball.x - c.p.x, ball.y - c.p.y);
        let horiz = (dx * dx + dy * dy).sqrt();
        let dz = ball.z - c.p.z;
        if horiz < CARRY_RADIUS && (CARRY_Z_LO..=CARRY_Z_HI).contains(&dz) {
            return 1.0;
        }
    }
    0.0
}

/// Full feature vector for `team` at grid frame `idx`: the single-frame base
/// (0–17) plus the v3 temporal/context features (18–21) that need the frame
/// sequence and the touch/goal events — momentum (`ball_vy_trend`), possession
/// recency (`time_since_att_touch`), and ball-carry/dribble for either side.
/// `None` when the base features are unavailable (no ball / no live car).
pub fn features_at(
    frames: &[GridFrame],
    idx: usize,
    team: i32,
    signs: &BTreeMap<i32, i32>,
    events: &[Event],
) -> Option<StateFeatures> {
    let af = attacking_frame(&frames[idx], team, signs);
    let mut sf = features_from_attacking(&af, team)?;
    let now = frames[idx].t;

    if let Some(ball) = af.ball {
        // Momentum: ball y-velocity (attack frame) now vs ~TREND_LAG_S ago.
        let dt = frames.get(1).map(|f| f.t - frames[0].t).unwrap_or(0.0);
        if dt > 0.0 {
            let lag = ((TREND_LAG_S / dt).round() as usize).max(1);
            if let Some(prev) = idx
                .checked_sub(lag)
                .and_then(|j| attacking_frame(&frames[j], team, signs).ball)
            {
                sf.x[18] = (ball.v.y - prev.v.y) / SPEED_SCALE;
            }
        }
        // Ball-carry / dribble, either side.
        let att: Vec<&GridCar> = af.cars.iter().filter(|c| c.team == Some(team)).collect();
        let def: Vec<&GridCar> = af
            .cars
            .iter()
            .filter(|c| c.team.is_some() && c.team != Some(team))
            .collect();
        sf.x[20] = carrying(&att, ball.p);
        sf.x[21] = carrying(&def, ball.p);
    }

    // Possession recency: seconds since the attacking team's last touch (≤ now),
    // capped and scaled to [0, 1]; 1.0 if they haven't touched yet.
    let last_touch = events
        .iter()
        .filter_map(|e| match e {
            Event::Touch { t, team: Some(tt), .. } if *tt == team && *t <= now => Some(*t),
            _ => None,
        })
        .fold(f32::NEG_INFINITY, f32::max);
    sf.x[19] = if last_touch.is_finite() {
        ((now - last_touch).min(RECENCY_CAP_S)) / RECENCY_CAP_S
    } else {
        1.0
    };

    Some(sf)
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
