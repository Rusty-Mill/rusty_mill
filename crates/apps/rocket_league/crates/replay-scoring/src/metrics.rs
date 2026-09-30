//! The metric library — the actual scoring IP (spec §6).
//!
//! Each metric is a pure reduction over the per-frame views + roles + events to
//! a single raw scalar (or `None` when the player had no qualifying frames).
//! Raw values are calibrated to 0–100 later by [`crate::config::Curve`].

use std::collections::BTreeMap;

use replay_analyzer::analyze::normalize::flip_xy;
use replay_analyzer::field::{BACK_WALL_Y, SUPERSONIC_SPEED};
use replay_analyzer::model::{Event, Vec3};

use crate::config::{Metric, ScoreConfig};
use crate::episodes::{challenges, followed_touches, losses, recoveries};
use crate::features::{dist, frame_at_time, FrameView, Third};
use crate::roles::{ManRole, Roles};

fn speed(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
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
    out.insert(
        Metric::DangerousTurnover,
        dangerous_turnover(frames, events, target_pri, target_team),
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
    // Candidate metrics (experimental in the default config): computed and
    // reconciled against ΔV, but excluded from the composite until promoted.
    out.insert(
        Metric::FacingBallShare,
        facing_ball_share(frames, target_pri),
    );
    out.insert(Metric::ReverseDriving, reverse_driving(frames, target_pri));
    // Experimental candidates (diagnostics only): strong rank signals but
    // redundant with the mechanical-tempo metrics, so out of the composite.
    out.insert(Metric::Pace, pace(frames, target_pri));
    out.insert(Metric::Agility, agility(frames, target_pri));
    out.insert(
        Metric::BoostStarvation,
        boost_starvation(frames, target_pri),
    );
    out.insert(Metric::ShotAngle, shot_angle(frames, events, target_pri));
    out.insert(Metric::WhiffRate, whiff_rate(frames, events, target_pri));
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

/// Position-weighted **dangerous-turnover** rate. Among the target's touches that
/// hand the ball to the opponent (the next touch is a *known* other team), each is
/// weighted by how deep in the target's own half the giveaway happened — a
/// turnover at the back wall counts ~1, one at midfield ~0 — then divided by the
/// player's total (followed) touches. This is the kinematic proxy for the value
/// model's "giveaway that handed the opponent scoring probability": a turnover
/// near your own net is far more costly than one in the offensive third, even
/// though plain `possession_retention` treats them the same. Lower is better;
/// `None` with no touches.
fn dangerous_turnover(frames: &[FrameView], events: &[Event], pri: i32, team: i32) -> Option<f32> {
    let touches = followed_touches(events, pri);
    let danger = losses(frames, events, pri, team)
        .iter()
        .fold(0.0f32, |s, e| s + e.danger());
    (touches > 0).then(|| danger / touches as f32)
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
    let eps = challenges(frames, roles, pri, team, cfg);
    let ok = eps.iter().filter(|e| e.is_ok()).count();
    ratio(ok, eps.len())
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

/// Mean recovery time (s): a reducer over the target's recovery episodes.
fn recovery_speed(frames: &[FrameView], pri: i32, cfg: &ScoreConfig) -> Option<f32> {
    let eps = recoveries(frames, pri, cfg);
    (!eps.is_empty()).then(|| eps.iter().map(|e| e.dur()).sum::<f32>() / eps.len() as f32)
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

/// Candidate: fraction of valid frames the target is oriented toward the ball
/// (heading within ~35° of the direction to the ball) — a spatial-awareness
/// signal. Reads the same per-frame yaw the bcstats `percent_facing_ball` uses.
fn facing_ball_share(frames: &[FrameView], pri: i32) -> Option<f32> {
    const CONE_COS: f32 = 0.819; // cos 35°
    let (mut num, mut den) = (0usize, 0usize);
    for f in frames {
        let (Some(c), Some(ball)) = (f.car(pri), f.ball) else {
            continue;
        };
        if !c.valid_pos {
            continue;
        }
        let to_ball = Vec3 {
            x: ball.p.x - c.p.x,
            y: ball.p.y - c.p.y,
            z: 0.0,
        };
        let Some(align) = c.forward_align(to_ball) else {
            continue;
        };
        den += 1;
        if align >= CONE_COS {
            num += 1;
        }
    }
    ratio(num, den)
}

/// Candidate: mean speed (uu/s) over present frames — raw pace/tempo. Higher ranks
/// simply play faster; independent enough from air time to be worth testing.
fn pace(frames: &[FrameView], pri: i32) -> Option<f32> {
    let (mut sum, mut n) = (0.0f32, 0usize);
    for f in frames {
        if let Some(c) = f.car(pri) {
            sum += speed(c.v);
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f32)
}

/// Candidate: mean duration (s) of a **zero-boost run** — how long the player
/// stays stranded on empty each time, not how often they touch 0. A brief dip
/// before grabbing a pad barely registers; being parked at 0 for seconds does.
/// Lower is better. `0.0` when the player is never fully empty; gaps (demo/
/// respawn absences) end a run rather than bridging it.
fn boost_starvation(frames: &[FrameView], pri: i32) -> Option<f32> {
    if frames.len() < 2 {
        return None;
    }
    let dt = frames[1].t - frames[0].t;
    if dt <= 0.0 {
        return None;
    }
    let (mut runs, mut run) = (Vec::<u32>::new(), 0u32);
    for f in frames {
        if matches!(f.car(pri), Some(c) if c.boost == Some(0)) {
            run += 1;
        } else if run > 0 {
            runs.push(run);
            run = 0;
        }
    }
    if run > 0 {
        runs.push(run);
    }
    if runs.is_empty() {
        return Some(0.0); // never stranded
    }
    let mean_frames = runs.iter().sum::<u32>() as f32 / runs.len() as f32;
    Some(mean_frames * dt)
}

/// Candidate: mean horizontal acceleration magnitude (uu/s²) across consecutive
/// present frames — mechanical sharpness / agility, independent of raw pace. Demo/
/// respawn velocity jumps are capped out, and gaps (absent frames) aren't bridged.
fn agility(frames: &[FrameView], pri: i32) -> Option<f32> {
    const MAX_ACCEL: f32 = 12_000.0; // ignore teleport/respawn discontinuities
    let (mut sum, mut n) = (0.0f32, 0usize);
    let mut prev: Option<(f32, Vec3)> = None; // (time, velocity)
    for f in frames {
        match f.car(pri) {
            Some(c) => {
                if let Some((pt, pv)) = prev {
                    let dt = f.t - pt;
                    if dt > 0.0 {
                        let a = ((c.v.x - pv.x).powi(2) + (c.v.y - pv.y).powi(2)).sqrt() / dt;
                        if a <= MAX_ACCEL {
                            sum += a;
                            n += 1;
                        }
                    }
                }
                prev = Some((f.t, c.v));
            }
            None => prev = None, // don't bridge acceleration across a gap
        }
    }
    (n > 0).then(|| sum / n as f32)
}

/// Candidate: mean **shooting angle** (deg) over the target's strikes from the
/// offensive half — the XY angle between the outgoing ball velocity and the line
/// from the ball to the centre of the opponent's goal mouth. Low = strikes lined
/// up with the goal; high = sprayed wide of it. A touch qualifies only as a real
/// strike (post-touch ball speed above a floor) that moves goalward from the
/// offensive half, so defensive clears never count; a hard centring pass that
/// drifts goalward still does — the corpus decides whether the signal survives
/// that. Lower is better; `None` with no qualifying strikes.
fn shot_angle(frames: &[FrameView], events: &[Event], pri: i32) -> Option<f32> {
    const MIN_STRIKE_SPEED: f32 = 1400.0; // a real strike, not a dribble tap

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
        let (Some(c), Some(ball)) = (f.car(pri), f.ball) else {
            continue;
        };
        // Attacking frame: the opponent goal mouth centre is at (0, +BACK_WALL_Y).
        let bp = flip_xy(ball.p, c.attack_sign);
        let bv = flip_xy(ball.v, c.attack_sign);
        if speed(ball.v) < MIN_STRIKE_SPEED || bv.y <= 0.0 || bp.y <= 0.0 {
            continue;
        }
        let (gx, gy) = (-bp.x, BACK_WALL_Y - bp.y);
        let shot_n = (bv.x * bv.x + bv.y * bv.y).sqrt();
        let goal_n = (gx * gx + gy * gy).sqrt();
        if shot_n < f32::EPSILON || goal_n < f32::EPSILON {
            continue;
        }
        let cos = ((bv.x * gx + bv.y * gy) / (shot_n * goal_n)).clamp(-1.0, 1.0);
        sum += cos.acos().to_degrees();
        n += 1;
    }
    (n > 0).then(|| sum / n as f32)
}

/// Candidate: **whiff rate** — of the target's strike attempts, the fraction that
/// never made contact. An attempt is a maximal run of valid frames inside
/// striking range of the ball, entered with real speed aimed squarely at it
/// (closing-speed floor + closing fraction, so a drive-by past the ball doesn't
/// count); it is a whiff when no touch by the player lands within the run (± a
/// grid step of slack). Motion-only caveats: a deliberate fake is
/// indistinguishable from a miss and counts, and so does being beaten to the
/// ball mid-swing — both read as failed challenges (the guide's "whiffed or
/// beaten" signal, D1-15). Lower is better; `None` with no attempts.
fn whiff_rate(frames: &[FrameView], events: &[Event], pri: i32) -> Option<f32> {
    const NEAR_UU: f32 = 320.0; // striking range (touch pairing uses ~300)
    const MIN_ENTRY_CLOSING: f32 = 500.0; // uu/s toward the ball at zone entry
    const MIN_CLOSING_FRAC: f32 = 0.70; // velocity mostly at the ball, not past it
    const TOUCH_TOL_S: f32 = 0.15; // grid/touch timing slack

    let touches: Vec<f32> = events
        .iter()
        .filter_map(|e| match e {
            Event::Touch { t, pri: tp, .. } if *tp == pri => Some(*t),
            _ => None,
        })
        .collect();

    // Maximal near-ball runs: (start_t, last_t, entered_as_a_swing).
    let mut runs: Vec<(f32, f32, bool)> = Vec::new();
    let mut open: Option<(f32, f32, bool)> = None;
    for f in frames {
        let near = f
            .car(pri)
            .filter(|c| c.valid_pos && c.dist_to_ball <= NEAR_UU);
        if let Some(c) = near {
            match open.as_mut() {
                Some((_, last, _)) => *last = f.t,
                None => {
                    let swing = c.closing_speed >= MIN_ENTRY_CLOSING
                        && c.closing_speed >= MIN_CLOSING_FRAC * speed(c.v);
                    open = Some((f.t, f.t, swing));
                }
            }
        } else if let Some(run) = open.take() {
            runs.push(run);
        }
    }
    if let Some(run) = open {
        runs.push(run);
    }

    let (mut whiffs, mut attempts) = (0usize, 0usize);
    for (start, last, swing) in runs {
        if !swing {
            continue;
        }
        attempts += 1;
        let hit = touches
            .iter()
            .any(|&t| t >= start - TOUCH_TOL_S && t <= last + TOUCH_TOL_S);
        if !hit {
            whiffs += 1;
        }
    }
    ratio(whiffs, attempts)
}

/// Candidate: fraction of grounded driving frames spent moving in reverse
/// (horizontal velocity opposed to the car's heading) — a car-control signal.
fn reverse_driving(frames: &[FrameView], pri: i32) -> Option<f32> {
    const MIN_SPEED: f32 = 150.0; // ignore near-stationary jitter
    let (mut num, mut den) = (0usize, 0usize);
    for f in frames {
        let Some(c) = f.car(pri) else { continue };
        if c.airborne {
            continue;
        }
        let hspeed = (c.v.x * c.v.x + c.v.y * c.v.y).sqrt();
        if hspeed < MIN_SPEED {
            continue;
        }
        let Some(align) = c.forward_align(c.v) else {
            continue;
        };
        den += 1;
        if align < 0.0 {
            num += 1;
        }
    }
    ratio(num, den)
}
