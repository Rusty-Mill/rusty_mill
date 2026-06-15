//! The metric library — the actual scoring IP (spec §6).
//!
//! Each metric is a pure reduction over the per-frame views + roles + events to
//! a single raw scalar (or `None` when the player had no qualifying frames).
//! Raw values are calibrated to 0–100 later by [`crate::config::Curve`].

use std::collections::BTreeMap;

use replay_analyzer::field::SUPERSONIC_SPEED;
use replay_analyzer::model::{Event, Vec3};

use crate::config::{Metric, ScoreConfig};
use crate::features::{CarView, FrameView, Third};
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
        boost_management(frames, target_pri, cfg),
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
    out.insert(
        Metric::ChallengeTiming,
        challenge_timing(frames, roles, target_pri, target_team, cfg),
    );
    out.insert(
        Metric::FirstTouchValue,
        first_touch_value(frames, roles, events, target_pri, target_team, cfg),
    );
    out.insert(
        Metric::TransitionReadiness,
        transition_readiness(frames, roles, events, target_pri, target_team, cfg),
    );
    out.insert(
        Metric::RecoverySpeed,
        recovery_speed(frames, target_pri, cfg),
    );
    out.insert(Metric::AerialPresence, aerial_presence(frames, target_pri));
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

/// Boost discipline as a higher-is-better composite (spec §6). On the calibration
/// corpus only two boost-economy signals carry rank — supersonic time-share
/// (ρ≈+0.56) and boost-collection rate (ρ≈+0.52) — while mean boost (−0.04) and
/// time-at-zero (+0.03) do not (hoarding isn't skill). So this blends those two,
/// each scaled to a comparable range, into one score.
fn boost_management(frames: &[FrameView], pri: i32, cfg: &ScoreConfig) -> Option<f32> {
    let (mut supersonic, mut n_s) = (0usize, 0usize);
    let (mut collected, mut prev) = (0.0f32, None::<f32>);
    let (mut t_first, mut t_last) = (None::<f32>, 0.0f32);
    let mut saw_boost = false;
    for f in frames {
        let Some(c) = f.car(pri) else {
            prev = None; // track gap: never bridge a boost delta across it
            continue;
        };
        n_s += 1;
        if speed(c.v) >= SUPERSONIC_SPEED {
            supersonic += 1;
        }
        if t_first.is_none() {
            t_first = Some(f.t);
        }
        t_last = f.t;
        match c.boost {
            Some(b) => {
                saw_boost = true;
                let pct = b as f32 / 2.55;
                if let Some(p) = prev {
                    if pct > p {
                        collected += pct - p; // boost gained = a pickup
                    }
                }
                prev = Some(pct);
            }
            None => prev = None,
        }
    }
    if n_s == 0 || !saw_boost {
        return None;
    }
    let dur = (t_last - t_first.unwrap_or(0.0)).max(1.0);
    let frac_supersonic = supersonic as f32 / n_s as f32;
    let collect_norm = (collected / dur / cfg.boost_collect_scale).min(1.0);
    Some(0.5 * frac_supersonic + 0.5 * collect_norm)
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

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

/// Index of the grid frame nearest `t` (frames are time-sorted ascending).
fn frame_at_time(frames: &[FrameView], t: f32) -> Option<usize> {
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

/// A detected 50/50 at frame `i`: the target (1st man, valid) and the opponent's
/// 1st man are both inside `challenge_radius_uu` of the ball. Returns the target
/// car, the opponent's 1st man, and the target→ball vector.
fn contest_at<'a>(
    f: &'a FrameView,
    roles: &Roles,
    i: usize,
    pri: i32,
    team: i32,
    cfg: &ScoreConfig,
) -> Option<(&'a CarView, &'a CarView, Vec3)> {
    if roles.role_of(i, team, pri, f) != Some(ManRole::First) {
        return None;
    }
    let c = f.car(pri)?;
    if !c.valid_pos {
        return None;
    }
    let ball = f.ball?;
    let opp_team = f.cars.iter().find(|o| o.team != team).map(|o| o.team)?;
    let opp = f.car(roles.first(i, opp_team)?)?;
    if dist(ball.p, c.p) <= cfg.challenge_radius_uu
        && dist(ball.p, opp.p) <= cfg.challenge_radius_uu
    {
        Some((c, opp, sub(ball.p, c.p)))
    } else {
        None
    }
}

/// Quality of arrival on detected 50/50s: fraction of contests the target (as
/// 1st man) reaches with boost, facing the ball, and not late versus the
/// opponent's 1st man. A decision-discipline (coaching) signal: it is weak for
/// *absolute* rank (a 50/50-win-rate variant measured ≈0 on the corpus — within
/// a matched game outcomes are roughly even regardless of rank), so calibration
/// down-weights it, but it still flags poor challenge habits.
fn challenge_timing(
    frames: &[FrameView],
    roles: &Roles,
    pri: i32,
    team: i32,
    cfg: &ScoreConfig,
) -> Option<f32> {
    let (mut num, mut den) = (0usize, 0usize);
    let mut in_contest = false;
    for (i, f) in frames.iter().enumerate() {
        match contest_at(f, roles, i, pri, team, cfg) {
            Some((c, opp, to_ball)) => {
                if !in_contest {
                    den += 1;
                    let boost_ok = c
                        .boost
                        .map(|b| b as f32 / 2.55 >= cfg.challenge_boost_min)
                        .unwrap_or(false);
                    let face_ok = c
                        .forward_align(to_ball)
                        .map(|a| a >= cfg.facing_cos_min)
                        .unwrap_or(false);
                    let timing_ok = c.time_to_ball <= opp.time_to_ball * cfg.challenge_late_margin;
                    if boost_ok && face_ok && timing_ok {
                        num += 1;
                    }
                }
                in_contest = true;
            }
            None => in_contest = false,
        }
    }
    ratio(num, den)
}

/// Mean post-touch ball progression over the target's 1st-man touches: `+`
/// toward the opponent half (attacking frame), with an extra `−1` when the
/// touch is a high-speed boom (`speed > boom_speed_uu`).
fn first_touch_value(
    frames: &[FrameView],
    roles: &Roles,
    events: &[Event],
    pri: i32,
    team: i32,
    cfg: &ScoreConfig,
) -> Option<f32> {
    let (mut sum, mut n) = (0.0f32, 0usize);
    for e in events {
        let Event::Touch { t, pri: tp, .. } = e else {
            continue;
        };
        if *tp != pri {
            continue;
        }
        let Some(i) = frame_at_time(frames, *t) else {
            continue;
        };
        let f = &frames[i];
        if roles.role_of(i, team, pri, f) != Some(ManRole::First) {
            continue;
        }
        let (Some(c), Some(ball)) = (f.car(pri), f.ball) else {
            continue;
        };
        let sp = speed(ball.v);
        if sp < f32::EPSILON {
            continue;
        }
        let fwd = (ball.v.y * c.attack_sign as f32) / sp;
        let boom = if sp > cfg.boom_speed_uu { 1.0 } else { 0.0 };
        sum += fwd - boom;
        n += 1;
    }
    (n > 0).then(|| sum / n as f32)
}

/// On possession flips, fraction the target (as 2nd man) is *ready to step up*
/// to 1st: fuelled (boost) and facing the play, with goal-side cover required
/// only on **defensive** transitions (the team just lost the ball). The earlier
/// "always goal-side + facing the ball" form rewarded ball-watching and
/// anti-correlated with rank on the calibration corpus; this rewards the active,
/// boost-ready rotation that higher ranks actually perform.
fn transition_readiness(
    frames: &[FrameView],
    roles: &Roles,
    events: &[Event],
    pri: i32,
    team: i32,
    cfg: &ScoreConfig,
) -> Option<f32> {
    let window = cfg.transition_window_s;
    let poss: Vec<(i32, f32)> = events
        .iter()
        .filter_map(|e| match e {
            Event::Possession { team, start, .. } => Some((*team, *start)),
            _ => None,
        })
        .collect();
    let (mut num, mut den) = (0usize, 0usize);
    for w in poss.windows(2) {
        if w[0].0 == w[1].0 {
            continue; // same team kept the ball — not a flip
        }
        let Some(i) = frame_at_time(frames, w[1].1) else {
            continue;
        };
        let f = &frames[i];
        if roles.role_of(i, team, pri, f) != Some(ManRole::Second) {
            continue;
        }
        let Some(c) = f.car(pri) else { continue };
        if !c.valid_pos {
            continue;
        }
        den += 1;
        // Ready = the 2nd man steps up after the flip: its time-to-ball shrinks
        // over the window (it closes in to take over the play), rather than
        // ball-watching from a static "textbook" posture.
        let ttb0 = c.time_to_ball;
        let stepped_up = frame_at_time(frames, w[1].1 + window)
            .and_then(|j| frames[j].car(pri))
            .map(|c2| c2.time_to_ball < ttb0)
            .unwrap_or(false);
        if stepped_up {
            num += 1;
        }
    }
    ratio(num, den)
}

/// Mean time (s) from the end of an airborne phase until the target is back
/// wheels-down, upright, and facing the ball — each recovery capped at
/// `recovery_cap_s` (also charged when a recovery never completes in time).
fn recovery_speed(frames: &[FrameView], pri: i32, cfg: &ScoreConfig) -> Option<f32> {
    let (mut sum, mut n) = (0.0f32, 0usize);
    let mut air_end: Option<f32> = None;
    for f in frames {
        let Some(c) = f.car(pri) else {
            air_end = None; // track gap (dead) — abandon any pending recovery
            continue;
        };
        if c.airborne {
            air_end = Some(f.t);
            continue;
        }
        let Some(end) = air_end else { continue };
        let dur = f.t - end;
        let recovered = c.upright
            && f.ball
                .and_then(|b| c.forward_align(sub(b.p, c.p)))
                .map(|a| a >= cfg.facing_cos_min)
                .unwrap_or(false);
        if recovered {
            sum += dur.min(cfg.recovery_cap_s);
            n += 1;
            air_end = None;
        } else if dur > cfg.recovery_cap_s {
            sum += cfg.recovery_cap_s;
            n += 1;
            air_end = None;
        }
    }
    (n > 0).then(|| sum / n as f32)
}

/// Fraction of present frames the target is airborne (off the ground). The
/// single strongest rank signal on the calibration corpus (ρ≈+0.59): higher
/// ranks live in the air far more.
fn aerial_presence(frames: &[FrameView], pri: i32) -> Option<f32> {
    let (mut num, mut den) = (0usize, 0usize);
    for f in frames {
        if let Some(c) = f.car(pri) {
            den += 1;
            if c.airborne {
                num += 1;
            }
        }
    }
    ratio(num, den)
}
