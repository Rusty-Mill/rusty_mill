//! The metric library — the actual scoring IP (spec §6).
//!
//! Each metric is a pure reduction over the per-frame views + roles + events to
//! a single raw scalar (or `None` when the player had no qualifying frames).
//! Raw values are calibrated to 0–100 later by [`crate::config::Curve`].

use std::collections::BTreeMap;

use replay_analyzer::model::{Event, Vec3};

use crate::config::{Metric, ScoreConfig};
use crate::features::{FrameView, Third};
use crate::roles::{ManRole, Roles};

fn speed(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}
fn dist(a: Vec3, b: Vec3) -> f32 {
    speed(Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    })
}
fn ratio(num: usize, den: usize) -> Option<f32> {
    (den > 0).then(|| num as f32 / den as f32)
}

/// Compute every registered metric's raw value for the target player.
pub fn compute(
    frames: &[FrameView],
    roles: &Roles,
    events: &[Event],
    target_pri: i32,
    target_team: i32,
    cfg: &ScoreConfig,
) -> BTreeMap<Metric, Option<f32>> {
    let mut out = BTreeMap::new();
    out.insert(
        Metric::OvercommitRate,
        overcommit_rate(frames, roles, target_pri, target_team),
    );
    out.insert(
        Metric::GoalsideDiscipline1st,
        goalside_first(frames, roles, target_pri, target_team),
    );
    out.insert(
        Metric::SupportSpacing,
        support_spacing(frames, roles, target_pri, target_team),
    );
    out.insert(
        Metric::CentralSupportFraction,
        central_support(frames, roles, target_pri, target_team, cfg),
    );
    out.insert(
        Metric::DoubleCommitRate,
        double_commit(frames, target_team, cfg),
    );
    out.insert(
        Metric::BoostManagement,
        boost_management(frames, target_pri),
    );
    out.insert(
        Metric::PossessionRetention,
        possession_retention(events, target_pri, target_team),
    );
    out.insert(Metric::BallChaseIndex, ball_chase(frames, target_team));
    out.insert(
        Metric::GoalsideDisciplineTeam,
        goalside_team(frames, target_pri, target_team),
    );
    out
}

/// 1st-man frames where the target is ahead of the ball with no goal-side cover.
fn overcommit_rate(frames: &[FrameView], roles: &Roles, pri: i32, team: i32) -> Option<f32> {
    let (mut num, mut den) = (0usize, 0usize);
    for (i, f) in frames.iter().enumerate() {
        if roles.role_of(i, team, pri, f) != Some(ManRole::First) {
            continue;
        }
        let Some(c) = f.car(pri) else { continue };
        if !c.valid_pos {
            continue;
        }
        den += 1;
        let teammate_uncovered = f.teammate(team, pri).map(|t| !t.goalside).unwrap_or(true);
        if !c.goalside && teammate_uncovered {
            num += 1;
        }
    }
    ratio(num, den)
}

/// Fraction of defensive 1st-man frames the target stays goal-side.
fn goalside_first(frames: &[FrameView], roles: &Roles, pri: i32, team: i32) -> Option<f32> {
    let (mut num, mut den) = (0usize, 0usize);
    for (i, f) in frames.iter().enumerate() {
        if roles.role_of(i, team, pri, f) != Some(ManRole::First) {
            continue;
        }
        let Some(c) = f.car(pri) else { continue };
        if !c.valid_pos || c.ball_third != Third::Def {
            continue;
        }
        den += 1;
        if c.goalside {
            num += 1;
        }
    }
    ratio(num, den)
}

/// Mean teammate separation while the target is 2nd man.
fn support_spacing(frames: &[FrameView], roles: &Roles, pri: i32, team: i32) -> Option<f32> {
    let (mut sum, mut n) = (0.0f32, 0usize);
    for (i, f) in frames.iter().enumerate() {
        if roles.role_of(i, team, pri, f) != Some(ManRole::Second) {
            continue;
        }
        let (Some(c), Some(mate)) = (f.car(pri), f.teammate(team, pri)) else {
            continue;
        };
        if !c.valid_pos {
            continue;
        }
        sum += dist(c.p, mate.p);
        n += 1;
    }
    (n > 0).then(|| sum / n as f32)
}

/// Fraction of 2nd-man frames spent in central, goal-side support.
fn central_support(
    frames: &[FrameView],
    roles: &Roles,
    pri: i32,
    team: i32,
    cfg: &ScoreConfig,
) -> Option<f32> {
    let (mut num, mut den) = (0usize, 0usize);
    for (i, f) in frames.iter().enumerate() {
        if roles.role_of(i, team, pri, f) != Some(ManRole::Second) {
            continue;
        }
        let Some(c) = f.car(pri) else { continue };
        if !c.valid_pos {
            continue;
        }
        den += 1;
        if c.pa.x.abs() < cfg.central_x && c.goalside {
            num += 1;
        }
    }
    ratio(num, den)
}

/// Fraction of frames both teammates are pressuring (low time_to_ball).
fn double_commit(frames: &[FrameView], team: i32, cfg: &ScoreConfig) -> Option<f32> {
    let (mut num, mut den) = (0usize, 0usize);
    for f in frames {
        let cars: Vec<_> = f.team_cars(team).filter(|c| c.valid_pos).collect();
        if cars.len() < 2 {
            continue;
        }
        den += 1;
        if cars.iter().all(|c| c.time_to_ball < cfg.press_ttb_s) {
            num += 1;
        }
    }
    ratio(num, den)
}

/// Mean boost held by the target (0–100 units).
fn boost_management(frames: &[FrameView], pri: i32) -> Option<f32> {
    let (mut sum, mut n) = (0.0f32, 0usize);
    for f in frames {
        if let Some(b) = f.car(pri).and_then(|c| c.boost) {
            sum += b as f32 / 2.55;
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f32)
}
/// Touches by the target that keep possession (next touch is same team).
fn possession_retention(events: &[Event], pri: i32, team: i32) -> Option<f32> {
    let seq: Vec<(i32, Option<i32>)> = events
        .iter()
        .filter_map(|e| match e {
            Event::Touch { pri, team, .. } => Some((*pri, *team)),
            _ => None,
        })
        .collect();

    let (mut num, mut den) = (0usize, 0usize);
    for w in seq.windows(2) {
        if w[0].0 != pri {
            continue;
        }
        den += 1;
        if w[1].1 == Some(team) {
            num += 1;
        }
    }
    ratio(num, den)
}

/// Mean chase-correlation: both teammates driving at the ball together.
fn ball_chase(frames: &[FrameView], team: i32) -> Option<f32> {
    let (mut sum, mut n) = (0.0f32, 0usize);
    for f in frames {
        let cars: Vec<_> = f.team_cars(team).filter(|c| c.valid_pos).collect();
        if cars.len() < 2 {
            continue;
        }
        let align =
            |c: &crate::features::CarView| (c.closing_speed / speed(c.v).max(1.0)).clamp(0.0, 1.0);
        sum += align(cars[0]) * align(cars[1]);
        n += 1;
    }
    (n > 0).then(|| sum / n as f32)
}

/// Fraction of defensive frames at least one teammate is goal-side.
fn goalside_team(frames: &[FrameView], pri: i32, team: i32) -> Option<f32> {
    let (mut num, mut den) = (0usize, 0usize);
    for f in frames {
        let c = f.car(pri);
        let valid_def = c
            .map(|c| c.valid_pos && c.ball_third == Third::Def)
            .unwrap_or(false);
        if !valid_def {
            continue;
        }
        den += 1;
        if f.team_cars(team).any(|c| c.goalside) {
            num += 1;
        }
    }
    ratio(num, den)
}
