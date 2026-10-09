//! Kinematic skill detectors.
//!
//! Each detector is an independent pure function over the canonical match's
//! resampled grid, events, and tracks — exactly the style of
//! `replay_analyzer::analyze::events`, so each is unit-testable against a
//! hand-built grid. [`crate::detect_all`] runs them and merges the output.
//!
//! Everything here is *inferred from motion*: replays carry positions,
//! velocities, rotations and boost, not controller inputs (spec §0), so a
//! positive is heuristic and carries a confidence. Skills that need input-only
//! state (flip resets, half-flips) are intentionally not attempted.

use std::collections::HashMap;

use replay_analyzer::analyze::normalize::flip_xy;
use replay_analyzer::field;
use replay_analyzer::model::{Event, GridCar, GridFrame, PlayerTrack, Resampled, Vec3};

use crate::config::SkillConfig;
use crate::report::SkillInstance;
use crate::skill::Skill;

// --- small kinematic helpers ---

fn speed(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

fn horiz_dist(a: Vec3, b: Vec3) -> f32 {
    let (dx, dy) = (a.x - b.x, a.y - b.y);
    (dx * dx + dy * dy).sqrt()
}

/// Angle (degrees) between two velocity vectors; 0 if either is ~stationary.
fn angle_deg(a: Vec3, b: Vec3) -> f32 {
    let (na, nb) = (speed(a), speed(b));
    if na < 1e-3 || nb < 1e-3 {
        return 0.0;
    }
    let dot = (a.x * b.x + a.y * b.y + a.z * b.z) / (na * nb);
    dot.clamp(-1.0, 1.0).acos().to_degrees()
}

/// Confidence ramp on `[lo, hi]` mapped to `0.5..=1.0` (a detection that just
/// clears its gate scores 0.5; one well past it scores 1.0).
fn conf(x: f32, lo: f32, hi: f32) -> f32 {
    if hi <= lo {
        return 1.0;
    }
    0.5 + 0.5 * ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Index of the grid frame whose time is closest to `t`, within `tol`.
fn frame_at(frames: &[GridFrame], t: f32, tol: f32) -> Option<usize> {
    if frames.is_empty() {
        return None;
    }
    // First frame with time >= t.
    let mut lo = 0usize;
    let mut hi = frames.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if frames[mid].t < t {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    // Closest of {lo-1, lo}.
    let mut best: Option<(usize, f32)> = None;
    for cand in [lo.wrapping_sub(1), lo] {
        if cand < frames.len() {
            let d = (frames[cand].t - t).abs();
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((cand, d));
            }
        }
    }
    best.filter(|&(_, d)| d <= tol).map(|(i, _)| i)
}

fn car_in(frame: &GridFrame, pri: i32) -> Option<&GridCar> {
    frame.cars.iter().find(|c| c.pri == pri)
}

/// `pri -> (name, team)` from coalesced tracks (mirrors `events::pri_lookup`).
fn pri_lookup(tracks: &[PlayerTrack]) -> HashMap<i32, (String, Option<i32>)> {
    tracks
        .iter()
        .map(|t| (t.pri, (t.player.clone(), t.team)))
        .collect()
}

/// Per-skill attacking-frame sign for a team (defaults to +1 if unknown).
fn attack_sign(resampled: &Resampled, team: Option<i32>) -> i32 {
    team.and_then(|t| resampled.team_attack_sign.get(&t).copied())
        .unwrap_or(1)
}

// --- detectors ---

/// Every sustained-speed run (pre-gate): `(pri, start, end, peak_speed)`. Holds the
/// supersonic entry/release-hysteresis context; the duration floor is applied by the
/// caller. Shared by [`supersonic`] and candidate-mode ([`supersonic_candidates`]).
fn speed_runs(resampled: &Resampled, cfg: &SkillConfig) -> Vec<(i32, f32, f32, f32)> {
    // pri -> (start_t, last_t, peak_speed)
    let mut active: HashMap<i32, (f32, f32, f32)> = HashMap::new();
    let mut runs = Vec::new();

    for f in &resampled.frames {
        // A car absent this frame (respawn/gap) ends any run it was in.
        let present: std::collections::HashSet<i32> = f.cars.iter().map(|c| c.pri).collect();
        let absent: Vec<i32> = active
            .keys()
            .copied()
            .filter(|p| !present.contains(p))
            .collect();
        for pri in absent {
            if let Some((s, e, peak)) = active.remove(&pri) {
                runs.push((pri, s, e, peak));
            }
        }

        // Extend or start runs in place; defer closes (can't remove while the
        // `get_mut` borrow is live) to a second pass.
        let mut to_close: Vec<i32> = Vec::new();
        for c in &f.cars {
            let s = speed(c.v);
            if let Some(run) = active.get_mut(&c.pri) {
                if s >= cfg.supersonic_release_speed {
                    run.1 = f.t;
                    run.2 = run.2.max(s);
                } else {
                    to_close.push(c.pri);
                }
            } else if s >= field::SUPERSONIC_SPEED {
                active.insert(c.pri, (f.t, f.t, s));
            }
        }
        for pri in to_close {
            if let Some((s, e, peak)) = active.remove(&pri) {
                runs.push((pri, s, e, peak));
            }
        }
    }
    for (pri, (s, e, peak)) in active.drain() {
        runs.push((pri, s, e, peak));
    }
    runs
}

/// Sustained supersonic sprints: one instance per run a car holds top speed,
/// entered at `field::SUPERSONIC_SPEED` and held until it drops below
/// `supersonic_release_speed` (hysteresis, so a single sprint isn't fragmented),
/// keeping only runs of at least `supersonic_min_duration_s`.
pub fn supersonic(
    resampled: &Resampled,
    tracks: &[PlayerTrack],
    cfg: &SkillConfig,
) -> Vec<SkillInstance> {
    let lookup = pri_lookup(tracks);
    let mut out = Vec::new();
    for (pri, start, end, peak) in speed_runs(resampled, cfg) {
        emit_speed_run(pri, (start, end, peak), &lookup, cfg, &mut out);
    }
    out
}

/// Candidate-mode for supersonic: the duration of *every* sustained-speed run
/// (pre-gate), short ones included, so the calibrator can fit the duration floor.
pub fn supersonic_candidates(resampled: &Resampled, cfg: &SkillConfig) -> Vec<f32> {
    speed_runs(resampled, cfg)
        .into_iter()
        .map(|(_, s, e, _)| e - s)
        .collect()
}

fn emit_speed_run(
    pri: i32,
    run: (f32, f32, f32),
    lookup: &HashMap<i32, (String, Option<i32>)>,
    cfg: &SkillConfig,
    out: &mut Vec<SkillInstance>,
) {
    let (start, end, peak) = run;
    if end - start < cfg.supersonic_min_duration_s {
        return;
    }
    let (player, team) = resolve(pri, lookup);
    out.push(SkillInstance {
        skill: Skill::Supersonic,
        t: start,
        pri,
        player,
        team,
        confidence: 1.0,
        metric: end - start,
        detail: format!("peak={:.0}uu/s dur={:.2}s", peak, end - start),
    });
}

/// Car height at the touch frame for `pri` — the raw per-touch metric behind the
/// aerial gate, read *without* gating. Shared by [`aerial_touch`] (which then
/// applies the gate) and by candidate-mode ([`aerial_candidates`]).
fn touch_car_z(resampled: &Resampled, fi: usize, pri: i32) -> Option<f32> {
    Some(car_in(&resampled.frames[fi], pri)?.p.z)
}

/// Whether a touch at frame `fi` was an aerial; returns `(car_z, ball_z)`.
fn aerial_touch(
    resampled: &Resampled,
    fi: usize,
    pri: i32,
    cfg: &SkillConfig,
) -> Option<(f32, f32)> {
    let car_z = touch_car_z(resampled, fi, pri)?;
    let ball_z = resampled.frames[fi].ball?.p.z;
    (car_z >= cfg.aerial_min_height && ball_z >= cfg.aerial_min_ball_height)
        .then_some((car_z, ball_z))
}

/// Aerials: a ball touch taken with the car airborne and elevated.
pub fn aerials(resampled: &Resampled, events: &[Event], cfg: &SkillConfig) -> Vec<SkillInstance> {
    let mut out = Vec::new();
    for e in events {
        let Event::Touch {
            t,
            pri,
            player,
            team,
        } = e
        else {
            continue;
        };
        let Some(fi) = frame_at(&resampled.frames, *t, cfg.touch_frame_tol_s) else {
            continue;
        };
        if let Some((car_z, ball_z)) = aerial_touch(resampled, fi, *pri, cfg) {
            out.push(SkillInstance {
                skill: Skill::Aerial,
                t: *t,
                pri: *pri,
                player: player.clone(),
                team: *team,
                confidence: conf(car_z, cfg.aerial_min_height, cfg.high_aerial_height),
                metric: car_z,
                detail: format!("car_z={car_z:.0} ball_z={ball_z:.0}"),
            });
        }
    }
    out
}

/// Candidate-mode for aerials: the car height at *every* ball touch (pre-gate).
///
/// The [`aerials`] detector keeps only touches clearing `aerial_min_height` /
/// `aerial_min_ball_height`, so a detected aerial's metric is observed only
/// *above* the floor — the floor itself can never be fit from gated data
/// (selection bias). This emits the full candidate population (ground touches
/// included; aerials are its upper tail) so the calibrator can place the floor
/// and the full-confidence top on the touch-height distribution. Pure twin of
/// [`aerials`]: same touch→frame resolution, no gate.
pub fn aerial_candidates(resampled: &Resampled, events: &[Event], cfg: &SkillConfig) -> Vec<f32> {
    let mut out = Vec::new();
    for e in events {
        let Event::Touch { t, pri, .. } = e else {
            continue;
        };
        let Some(fi) = frame_at(&resampled.frames, *t, cfg.touch_frame_tol_s) else {
            continue;
        };
        if let Some(car_z) = touch_car_z(resampled, fi, *pri) {
            out.push(car_z);
        }
    }
    out
}

/// Air dribbles: a run of `>= air_dribble_min_touches` aerial touches by the same
/// player, each within `air_dribble_window_s` of the last. One instance per run.
pub fn air_dribbles(
    resampled: &Resampled,
    events: &[Event],
    cfg: &SkillConfig,
) -> Vec<SkillInstance> {
    // Gather aerial touches in time order: (t, pri, player, team).
    let mut aerial: Vec<(f32, i32, Option<String>, Option<i32>)> = Vec::new();
    for e in events {
        let Event::Touch {
            t,
            pri,
            player,
            team,
        } = e
        else {
            continue;
        };
        if let Some(fi) = frame_at(&resampled.frames, *t, cfg.touch_frame_tol_s) {
            if aerial_touch(resampled, fi, *pri, cfg).is_some() {
                aerial.push((*t, *pri, player.clone(), *team));
            }
        }
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i < aerial.len() {
        // Extend a same-player chain while gaps stay within the window.
        let mut j = i + 1;
        while j < aerial.len()
            && aerial[j].1 == aerial[i].1
            && aerial[j].0 - aerial[j - 1].0 <= cfg.air_dribble_window_s
        {
            j += 1;
        }
        let len = j - i;
        if len >= cfg.air_dribble_min_touches {
            let (t, pri, player, team) = aerial[i].clone();
            out.push(SkillInstance {
                skill: Skill::AirDribble,
                t,
                pri,
                player,
                team,
                confidence: conf(
                    len as f32,
                    cfg.air_dribble_min_touches as f32,
                    cfg.air_dribble_min_touches as f32 + 3.0,
                ),
                metric: aerial[j - 1].0 - t,
                detail: format!("touches={} dur={:.2}s", len, aerial[j - 1].0 - t),
            });
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Double touches: the same player contacts the ball twice in quick succession
/// (within `double_touch_window_s`, no other player's touch between) with the
/// ball elevated at the second contact (`double_touch_min_ball_height`) — a
/// pop-and-strike, e.g. off the backboard. Emitted at the second touch.
///
/// This is kinematically separable (touch timing + ball height) and so, unlike
/// the input-only mechanics the catalog deliberately omits (wave dash, half
/// flip), it earns a detector. The ball-height gate keeps a ground dribble's low
/// micro-touches out.
pub fn double_touches(
    resampled: &Resampled,
    events: &[Event],
    cfg: &SkillConfig,
) -> Vec<SkillInstance> {
    let touches: Vec<(f32, i32, Option<String>, Option<i32>)> = events
        .iter()
        .filter_map(|e| match e {
            Event::Touch {
                t,
                pri,
                player,
                team,
            } => Some((*t, *pri, player.clone(), *team)),
            _ => None,
        })
        .collect();

    let mut out = Vec::new();
    for w in touches.windows(2) {
        let (t0, pri0, _, _) = &w[0];
        let (t1, pri1, player, team) = &w[1];
        if pri0 != pri1 {
            continue; // an intervening opponent touch breaks the pair
        }
        let dt = t1 - t0;
        if dt <= cfg.touch_frame_tol_s || dt > cfg.double_touch_window_s {
            continue;
        }
        let Some(fi) = frame_at(&resampled.frames, *t1, cfg.touch_frame_tol_s) else {
            continue;
        };
        let Some(ball) = resampled.frames[fi].ball else {
            continue;
        };
        if ball.p.z < cfg.double_touch_min_ball_height {
            continue;
        }
        out.push(SkillInstance {
            skill: Skill::DoubleTouch,
            t: *t1,
            pri: *pri1,
            player: player.clone(),
            team: *team,
            confidence: conf(
                ball.p.z,
                cfg.double_touch_min_ball_height,
                cfg.double_touch_min_ball_height + 600.0,
            ),
            metric: dt,
            detail: format!("gap={dt:.2}s ball_z={:.0}uu", ball.p.z),
        });
    }
    out
}

/// Ceiling plays: one instance per contiguous run a car spends within
/// `ceiling_tol` of the ceiling, lasting at least `ceiling_min_duration_s`.
pub fn ceiling_plays(
    resampled: &Resampled,
    tracks: &[PlayerTrack],
    cfg: &SkillConfig,
) -> Vec<SkillInstance> {
    let lookup = pri_lookup(tracks);
    let mut out = Vec::new();
    for (pri, start, end, peak_z) in ceiling_runs(resampled, cfg) {
        emit_ceiling_run(pri, (start, end, peak_z), &lookup, cfg, &mut out);
    }
    out
}

/// Every ceiling run (pre-gate): `(pri, start, end, peak_z)`. Holds the
/// near-the-ceiling context; the duration floor is applied by the caller. Shared by
/// [`ceiling_plays`] and candidate-mode ([`ceiling_candidates`]).
fn ceiling_runs(resampled: &Resampled, cfg: &SkillConfig) -> Vec<(i32, f32, f32, f32)> {
    let threshold = field::CEILING_Z - cfg.ceiling_tol;
    let mut active: HashMap<i32, (f32, f32, f32)> = HashMap::new(); // start, last, peak_z
    let mut runs = Vec::new();

    for f in &resampled.frames {
        let mut hot: HashMap<i32, f32> = HashMap::new();
        for c in &f.cars {
            if c.p.z >= threshold {
                hot.insert(c.pri, c.p.z);
            }
        }
        let ended: Vec<i32> = active
            .keys()
            .copied()
            .filter(|p| !hot.contains_key(p))
            .collect();
        for pri in ended {
            if let Some((s, e, peak)) = active.remove(&pri) {
                runs.push((pri, s, e, peak));
            }
        }
        for (pri, z) in hot {
            active
                .entry(pri)
                .and_modify(|r| {
                    r.1 = f.t;
                    r.2 = r.2.max(z);
                })
                .or_insert((f.t, f.t, z));
        }
    }
    for (pri, (s, e, peak)) in active.drain() {
        runs.push((pri, s, e, peak));
    }
    runs
}

/// Candidate-mode for ceiling plays: the duration of *every* ceiling run (pre-gate),
/// short ones included, so the calibrator can fit the duration floor.
pub fn ceiling_candidates(resampled: &Resampled, cfg: &SkillConfig) -> Vec<f32> {
    ceiling_runs(resampled, cfg)
        .into_iter()
        .map(|(_, s, e, _)| e - s)
        .collect()
}

fn emit_ceiling_run(
    pri: i32,
    run: (f32, f32, f32),
    lookup: &HashMap<i32, (String, Option<i32>)>,
    cfg: &SkillConfig,
    out: &mut Vec<SkillInstance>,
) {
    let (start, end, peak_z) = run;
    if end - start < cfg.ceiling_min_duration_s {
        return;
    }
    let (player, team) = resolve(pri, lookup);
    out.push(SkillInstance {
        skill: Skill::CeilingPlay,
        t: start,
        pri,
        player,
        team,
        confidence: 1.0,
        metric: end - start,
        detail: format!("peak_z={peak_z:.0} dur={:.2}s", end - start),
    });
}

/// Wall plays: a ball touch taken with the car up on a side or back wall.
pub fn wall_plays(
    resampled: &Resampled,
    events: &[Event],
    cfg: &SkillConfig,
) -> Vec<SkillInstance> {
    let mut out = Vec::new();
    for e in events {
        let Event::Touch {
            t,
            pri,
            player,
            team,
        } = e
        else {
            continue;
        };
        let Some(fi) = frame_at(&resampled.frames, *t, cfg.touch_frame_tol_s) else {
            continue;
        };
        let Some(car) = car_in(&resampled.frames[fi], *pri) else {
            continue;
        };
        let on_side = car.p.x.abs() >= field::SIDE_WALL_X - cfg.wall_plane_tol;
        let on_back = car.p.y.abs() >= field::BACK_WALL_Y - cfg.wall_plane_tol;
        if (on_side || on_back) && car.p.z >= cfg.wall_min_height {
            let which = if on_side { "side" } else { "back" };
            out.push(SkillInstance {
                skill: Skill::WallPlay,
                t: *t,
                pri: *pri,
                player: player.clone(),
                team: *team,
                confidence: conf(car.p.z, cfg.wall_min_height, field::CEILING_Z * 0.6),
                metric: car.p.z,
                detail: format!("{which} wall z={:.0}", car.p.z),
            });
        }
    }
    out
}

/// The car carrying the ball this frame (ball balanced on top, near the ground),
/// if any: the nearest car horizontally that sits under a low, on-roof ball.
fn dribble_carrier(f: &GridFrame, cfg: &SkillConfig) -> Option<i32> {
    let b = f.ball?;
    if !(cfg.dribble_ball_height_min..=cfg.dribble_ball_height_max).contains(&b.p.z) {
        return None;
    }
    let (car, d) = f
        .cars
        .iter()
        .map(|c| (c, horiz_dist(c.p, b.p)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    (d <= cfg.dribble_horiz_radius && car.p.z < cfg.dribble_carrier_max_z).then_some(car.pri)
}

/// Ground dribbles: a sustained run where one car keeps the ball balanced on top
/// near the ground. One instance per run lasting `>= dribble_min_duration_s`.
pub fn ground_dribbles(
    resampled: &Resampled,
    tracks: &[PlayerTrack],
    cfg: &SkillConfig,
) -> Vec<SkillInstance> {
    let lookup = pri_lookup(tracks);
    let mut out = Vec::new();
    for (pri, start, end, _) in dribble_runs(resampled, cfg) {
        emit_dribble(Some((pri, start, end)), &lookup, cfg, &mut out);
    }
    out
}

/// Every ground-dribble run (pre-gate): `(pri, start, end, 0.0)`. Holds the
/// `dribble_carrier` (ball low and tracking the car) context; the duration floor is
/// applied by the caller. Shared by [`ground_dribbles`] and [`dribble_candidates`].
fn dribble_runs(resampled: &Resampled, cfg: &SkillConfig) -> Vec<(i32, f32, f32, f32)> {
    let mut runs = Vec::new();
    let mut run: Option<(i32, f32, f32)> = None; // pri, start, last

    for f in &resampled.frames {
        match dribble_carrier(f, cfg) {
            Some(pri) => match &mut run {
                Some((rp, _, last)) if *rp == pri => *last = f.t,
                _ => {
                    if let Some((p, s, e)) = run.take() {
                        runs.push((p, s, e, 0.0));
                    }
                    run = Some((pri, f.t, f.t));
                }
            },
            None => {
                if let Some((p, s, e)) = run.take() {
                    runs.push((p, s, e, 0.0));
                }
            }
        }
    }
    if let Some((p, s, e)) = run.take() {
        runs.push((p, s, e, 0.0));
    }
    runs
}

/// Candidate-mode for ground dribbles: the duration of *every* dribble run
/// (pre-gate), short ones included, so the calibrator can fit the duration floor.
pub fn dribble_candidates(resampled: &Resampled, cfg: &SkillConfig) -> Vec<f32> {
    dribble_runs(resampled, cfg)
        .into_iter()
        .map(|(_, s, e, _)| e - s)
        .collect()
}

fn emit_dribble(
    run: Option<(i32, f32, f32)>,
    lookup: &HashMap<i32, (String, Option<i32>)>,
    cfg: &SkillConfig,
    out: &mut Vec<SkillInstance>,
) {
    let Some((pri, start, end)) = run else { return };
    let dur = end - start;
    if dur < cfg.dribble_min_duration_s {
        return;
    }
    let (player, team) = resolve(pri, lookup);
    out.push(SkillInstance {
        skill: Skill::GroundDribble,
        t: start,
        pri,
        player,
        team,
        confidence: conf(
            dur,
            cfg.dribble_min_duration_s,
            cfg.dribble_min_duration_s * 3.0,
        ),
        metric: dur,
        detail: format!("dur={dur:.2}s"),
    });
}

/// Carried-ball touch read for flicks: `(up_dv, ball_z)` when the touch releases a
/// *carried* ball (low, slow incoming) — the flick context, read *without* the
/// `flick_min_up_dv` gate. Shared by [`flicks`] (which then applies the floor) and
/// candidate-mode ([`flick_candidates`]). `None` when not a flick candidate.
fn flick_candidate(frames: &[GridFrame], fi: usize, cfg: &SkillConfig) -> Option<(f32, f32)> {
    if fi == 0 {
        return None;
    }
    let (pre, post) = (frames[fi - 1].ball?, frames[fi].ball?);
    let carried = post.p.z <= cfg.dribble_ball_height_max && speed(pre.v) <= cfg.flick_max_incoming;
    carried.then_some((post.v.z, post.p.z))
}

/// Flicks: a touch that releases a carried (low, slow) ball sharply upward.
pub fn flicks(resampled: &Resampled, events: &[Event], cfg: &SkillConfig) -> Vec<SkillInstance> {
    let frames = &resampled.frames;
    let mut out = Vec::new();
    for e in events {
        let Event::Touch {
            t,
            pri,
            player,
            team,
        } = e
        else {
            continue;
        };
        let Some(fi) = frame_at(frames, *t, cfg.touch_frame_tol_s) else {
            continue;
        };
        let Some((up_dv, ball_z)) = flick_candidate(frames, fi, cfg) else {
            continue;
        };
        if up_dv >= cfg.flick_min_up_dv {
            out.push(SkillInstance {
                skill: Skill::Flick,
                t: *t,
                pri: *pri,
                player: player.clone(),
                team: *team,
                confidence: conf(up_dv, cfg.flick_min_up_dv, cfg.flick_min_up_dv + 800.0),
                metric: up_dv,
                detail: format!("up_v={up_dv:.0} ball_z={ball_z:.0}"),
            });
        }
    }
    out
}

/// Candidate-mode for flicks: the up-velocity imparted to *every carried* ball
/// (pre-gate). Holds the carry context that defines a flick attempt and drops the
/// `flick_min_up_dv` floor, so the calibrator sees gentle releases and real flicks
/// alike — the gap between them is where the floor belongs.
pub fn flick_candidates(resampled: &Resampled, events: &[Event], cfg: &SkillConfig) -> Vec<f32> {
    let frames = &resampled.frames;
    let mut out = Vec::new();
    for e in events {
        let Event::Touch { t, .. } = e else {
            continue;
        };
        let Some(fi) = frame_at(frames, *t, cfg.touch_frame_tol_s) else {
            continue;
        };
        if let Some((up_dv, _)) = flick_candidate(frames, fi, cfg) {
            out.push(up_dv);
        }
    }
    out
}

/// Goalward touch read for power shots: `(speed, goalward)` when the touch sends
/// the ball toward the attacking goal — the power-shot context, read *without* the
/// `power_shot_min_speed` gate. Shared by [`power_shots`] (which then applies the
/// floor) and candidate-mode ([`power_shot_candidates`]). `None` when not goalward.
fn power_shot_candidate(
    resampled: &Resampled,
    fi: usize,
    team: Option<i32>,
    cfg: &SkillConfig,
) -> Option<(f32, f32)> {
    let ball = resampled.frames[fi].ball?;
    let spd = speed(ball.v);
    // Rotate the result into the toucher's attacking frame: +Y is the goal.
    let goalward = flip_xy(ball.v, attack_sign(resampled, team)).y;
    (goalward >= cfg.power_shot_min_goalward * spd).then_some((spd, goalward))
}

/// Power shots: a touch that sends the ball fast and toward the opponent goal.
pub fn power_shots(
    resampled: &Resampled,
    events: &[Event],
    cfg: &SkillConfig,
) -> Vec<SkillInstance> {
    let frames = &resampled.frames;
    let mut out = Vec::new();
    for e in events {
        let Event::Touch {
            t,
            pri,
            player,
            team,
        } = e
        else {
            continue;
        };
        let Some(fi) = frame_at(frames, *t, cfg.touch_frame_tol_s) else {
            continue;
        };
        let Some((spd, goalward)) = power_shot_candidate(resampled, fi, *team, cfg) else {
            continue;
        };
        if spd >= cfg.power_shot_min_speed {
            out.push(SkillInstance {
                skill: Skill::PowerShot,
                t: *t,
                pri: *pri,
                player: player.clone(),
                team: *team,
                confidence: conf(spd, cfg.power_shot_min_speed, 3500.0),
                metric: spd,
                detail: format!("speed={spd:.0} goalward={goalward:.0}"),
            });
        }
    }
    out
}

/// Candidate-mode for power shots: the ball speed of *every goalward* touch
/// (pre-gate). Holds the direction context that defines a power-shot attempt and
/// drops the `power_shot_min_speed` floor, so the calibrator sees gentle passes and
/// real cannons alike.
pub fn power_shot_candidates(
    resampled: &Resampled,
    events: &[Event],
    cfg: &SkillConfig,
) -> Vec<f32> {
    let mut out = Vec::new();
    for e in events {
        let Event::Touch { t, team, .. } = e else {
            continue;
        };
        let Some(fi) = frame_at(&resampled.frames, *t, cfg.touch_frame_tol_s) else {
            continue;
        };
        if let Some((spd, _)) = power_shot_candidate(resampled, fi, *team, cfg) {
            out.push(spd);
        }
    }
    out
}

/// Fast-redirect touch read: `(angle, post_speed)` when a touch turns a fast
/// incoming ball back toward goal while keeping it fast — the redirect context,
/// read *without* the `redirect_min_angle_deg` gate. Shared by [`redirects`] (which
/// then applies the angle floor) and candidate-mode ([`redirect_candidates`]).
/// `None` when not a redirect candidate (slow in/out, or not goalward).
fn redirect_candidate(
    resampled: &Resampled,
    fi: usize,
    team: Option<i32>,
    cfg: &SkillConfig,
) -> Option<(f32, f32)> {
    if fi == 0 {
        return None;
    }
    let (pre, post) = (resampled.frames[fi - 1].ball?, resampled.frames[fi].ball?);
    let ang = angle_deg(pre.v, post.v);
    let post_spd = speed(post.v);
    let goalward = flip_xy(post.v, attack_sign(resampled, team)).y;
    (speed(pre.v) >= cfg.redirect_min_incoming
        && post_spd >= cfg.redirect_min_speed
        && goalward > 0.0)
        .then_some((ang, post_spd))
}

/// Redirects: a touch that sharply turns a fast incoming ball back toward goal.
pub fn redirects(resampled: &Resampled, events: &[Event], cfg: &SkillConfig) -> Vec<SkillInstance> {
    let frames = &resampled.frames;
    let mut out = Vec::new();
    for e in events {
        let Event::Touch {
            t,
            pri,
            player,
            team,
        } = e
        else {
            continue;
        };
        let Some(fi) = frame_at(frames, *t, cfg.touch_frame_tol_s) else {
            continue;
        };
        let Some((ang, post_spd)) = redirect_candidate(resampled, fi, *team, cfg) else {
            continue;
        };
        if ang >= cfg.redirect_min_angle_deg {
            out.push(SkillInstance {
                skill: Skill::Redirect,
                t: *t,
                pri: *pri,
                player: player.clone(),
                team: *team,
                confidence: conf(ang, cfg.redirect_min_angle_deg, 130.0),
                metric: ang,
                detail: format!("angle={ang:.0}deg speed={post_spd:.0}"),
            });
        }
    }
    out
}

/// Candidate-mode for redirects: the turn angle of *every* fast-in/fast-out
/// goalward touch (pre-gate). Holds the redirect context (fast incoming, still fast
/// and goalward after) and drops the `redirect_min_angle_deg` floor, so the
/// calibrator sees glancing touches and sharp redirects alike.
pub fn redirect_candidates(resampled: &Resampled, events: &[Event], cfg: &SkillConfig) -> Vec<f32> {
    let mut out = Vec::new();
    for e in events {
        let Event::Touch { t, team, .. } = e else {
            continue;
        };
        let Some(fi) = frame_at(&resampled.frames, *t, cfg.touch_frame_tol_s) else {
            continue;
        };
        if let Some((ang, _)) = redirect_candidate(resampled, fi, *team, cfg) {
            out.push(ang);
        }
    }
    out
}

/// Car speed (uu/s) marking the kickoff "GO": well above resampling jitter, far
/// below real driving speed, so the first frame any car clears it is the release.
const KICKOFF_GO_SPEED: f32 = 100.0;

/// The "GO" time of a kickoff whose first touch is at `touch_t`: walk *backward*
/// over the grid from the touch through the continuous run of car motion, stopping
/// at the frozen countdown — the release is where that run begins.
///
/// Anchoring on the touch makes it robust to brief pre-countdown twitches (a car
/// settling on respawn): those sit before the frozen gap, so the backward walk
/// stops after them. `None` if no car moves at all between `kt` and the touch
/// (degenerate — not a real kickoff). For post-goal kickoffs whose event frame
/// already sits at the release, the run reaches back to `kt`, so GO ≈ `kt`.
fn kickoff_go(resampled: &Resampled, kt: f32, touch_t: f32) -> Option<f32> {
    let frames = &resampled.frames;
    let lo = frames.partition_point(|f| f.t < kt);
    let hi = frames.partition_point(|f| f.t <= touch_t);
    let span = &frames[lo..hi];
    let moving = |f: &GridFrame| f.cars.iter().any(|c| speed(c.v) >= KICKOFF_GO_SPEED);
    if !span.iter().any(&moving) {
        return None; // cars never moved between setup and touch — degenerate
    }
    let mut go = kt;
    for f in span.iter().rev() {
        if moving(f) {
            go = f.t;
        } else {
            break;
        }
    }
    Some(go)
}

/// Kickoff first touches: for each kickoff, the first touch after setup, with
/// `metric` the time from car *release* ("GO") to that touch.
///
/// The metric is measured from GO, not the kickoff event time `kt`: `kt` lands at
/// the countdown start for some kickoffs (opening) and at the release for others
/// (post-goal), so a raw `touch - kt` folds the frozen ~3 s countdown in and pins
/// against the window cap. Measuring from GO yields a true, varying approach time.
pub fn kickoff_first_touches(
    resampled: &Resampled,
    events: &[Event],
    cfg: &SkillConfig,
) -> Vec<SkillInstance> {
    let window = cfg.kickoff_touch_window_s;
    let mut out = Vec::new();
    for (i, e) in events.iter().enumerate() {
        let Event::Kickoff { t: kt } = e else {
            continue;
        };
        // First touch after the kickoff — the ball is frozen until release, so the
        // next touch is the kickoff touch (window spans the full countdown+approach).
        for f in &events[i + 1..] {
            if f.time() > kt + window {
                break;
            }
            if let Event::Touch {
                t,
                pri,
                player,
                team,
            } = f
            {
                let Some(go) = kickoff_go(resampled, *kt, *t) else {
                    break; // cars never released — degenerate kickoff
                };
                out.push(SkillInstance {
                    skill: Skill::KickoffFirstTouch,
                    t: *t,
                    pri: *pri,
                    player: player.clone(),
                    team: *team,
                    confidence: 1.0,
                    metric: t - go,
                    detail: format!("{:.2}s after release", t - go),
                });
                break;
            }
        }
    }
    out
}

/// Boost steals: a full big-pad pickup taken in the opponent's half (denying the
/// opponent their corner boost). Read from raw track samples, gap-aware.
pub fn boost_steals(
    tracks: &[PlayerTrack],
    resampled: &Resampled,
    cfg: &SkillConfig,
) -> Vec<SkillInstance> {
    let mut out = Vec::new();
    for tr in tracks {
        let sign = attack_sign(resampled, tr.team);
        for w in tr.samples.windows(2) {
            let (prev, cur) = (&w[0], &w[1]);
            // A pickup that straddles a respawn gap is a spawn refill, not a steal.
            if tr.gaps.iter().any(|g| prev.t <= g.start && cur.t >= g.end) {
                continue;
            }
            let (Some(pb), Some(cb)) = (prev.boost, cur.boost) else {
                continue;
            };
            if cb >= cfg.boost_steal_min_amount && cb.saturating_sub(pb) >= cfg.boost_steal_min_gain
            {
                let ap = flip_xy(cur.p, sign);
                if ap.y >= cfg.boost_steal_min_y {
                    out.push(SkillInstance {
                        skill: Skill::BoostSteal,
                        t: cur.t,
                        pri: tr.pri,
                        player: Some(tr.player.clone()),
                        team: tr.team,
                        confidence: 0.7,
                        // Boost gained, as a percent of a full tank (the raw
                        // 0..=255 replication byte is a network detail).
                        metric: f32::from(cb - pb) / 255.0 * 100.0,
                        detail: format!("+{} to {} at y={:.0}", cb - pb, cb, ap.y),
                    });
                }
            }
        }
    }
    out
}

/// Demos: each demolition credited to its attacker (authoritative event).
pub fn demos(events: &[Event], tracks: &[PlayerTrack]) -> Vec<SkillInstance> {
    let lookup = pri_lookup(tracks);
    let mut out = Vec::new();
    for e in events {
        let Event::Demo {
            t,
            attacker_pri,
            attacker,
            victim,
            ..
        } = e
        else {
            continue;
        };
        let Some(pri) = attacker_pri else { continue };
        let team = lookup.get(pri).and_then(|(_, tm)| *tm);
        out.push(SkillInstance {
            skill: Skill::Demo,
            t: *t,
            pri: *pri,
            player: attacker.clone(),
            team,
            confidence: 1.0,
            metric: 1.0,
            detail: match victim {
                Some(v) => format!("demoed {v}"),
                None => "demoed opponent".to_string(),
            },
        });
    }
    out
}

/// Resolve a PRI to `(player, team)` via the lookup, tolerating an unknown PRI.
fn resolve(
    pri: i32,
    lookup: &HashMap<i32, (String, Option<i32>)>,
) -> (Option<String>, Option<i32>) {
    match lookup.get(&pri) {
        Some((name, team)) => (Some(name.clone()), *team),
        None => (None, None),
    }
}
