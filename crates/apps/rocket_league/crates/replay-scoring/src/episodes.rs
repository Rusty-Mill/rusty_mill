//! Episodes: the moments behind the aggregate metrics.
//!
//! Each emitter holds the scan loop of one metric, and the metric is a reducer
//! over its episodes — so the list a player sees and the score they get cannot
//! drift apart. Design: `docs/episodes-design.md`.

use replay_analyzer::field::{BACK_WALL_Y, BALL_RADIUS, SIDE_WALL_X};
use replay_analyzer::model::{CanonicalMatch, Event, StatKind, Vec3};
use serde::Serialize;

use crate::config::ScoreConfig;
use crate::features::{build_frames, dist, frame_at_time, sub, CarView, FrameView};
use crate::roles::{self, ManRole, Roles};
use crate::xg::XgModel;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Episode {
    /// Airborne end → wheels-down, upright and facing the ball. `dur` is capped at
    /// `recovery_cap_s`; `done` means it completed within the cap.
    Recovery {
        pri: i32,
        t0: f32,
        dur: f32,
        done: bool,
    },
    /// A 1st-man ↔ opponent-1st-man contest of the ball. `miss` holds the failed
    /// arrival tests (`MISS_*`); `dur` is how long the contest lasted.
    Challenge {
        pri: i32,
        opp: i32,
        t0: f32,
        dur: f32,
        miss: u8,
    },
    /// A touch whose next known touch is the other team's. `danger` is the depth
    /// of the ball in the player's own half (0 = midfield or beyond, 1 = back
    /// wall); `opp_dist` is the nearest opponent to the ball.
    Loss {
        pri: i32,
        t: f32,
        at: [f32; 2],
        danger: f32,
        opp_dist: f32,
    },
    /// A shot on the opponent goal. `t` is the shooter's touch just before the
    /// scoreboard counter ticked; `aim` is where the ball crosses the goal plane
    /// (x, z; +x is the shooter's right), from its post-touch velocity under gravity, and `tta` the time to get there.
    Shot {
        pri: i32,
        t: f32,
        speed: f32,
        aim: Option<[f32; 2]>,
        tta: Option<f32>,
        on_target: bool,
        outcome: Outcome,
        /// Ball position at the strike in the shooter's frame (+y toward the goal, +x right).
        at: [f32; 2],
        /// Opponents (keeper included) between the ball and the goal line, within a goal-width lane.
        def: u8,
        /// Chance of scoring, from the [`XgModel`] the episodes were extracted with.
        xg: f32,
    },
    /// `pri` demolished `victim`. `down` is how long the victim was out (until their
    /// car reappears, at most `DEMO_DOWN_CAP_S`); `goal` is whether the demolisher's
    /// team scored within `GOAL_AFTER_DEMO_S` of it.
    Demo {
        pri: i32,
        victim: i32,
        t: f32,
        down: f32,
        goal: bool,
    },
}

/// How a shot ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Goal,
    Saved,
    Off,
}

pub const MISS_BOOST: u8 = 1;
pub const MISS_FACE: u8 = 2;
pub const MISS_LATE: u8 = 4;

impl Episode {
    /// Start time (s).
    pub fn t(&self) -> f32 {
        match self {
            Self::Recovery { t0: t, .. }
            | Self::Challenge { t0: t, .. }
            | Self::Loss { t, .. }
            | Self::Shot { t, .. }
            | Self::Demo { t, .. } => *t,
        }
    }
    pub fn pri(&self) -> i32 {
        match self {
            Self::Recovery { pri, .. }
            | Self::Challenge { pri, .. }
            | Self::Loss { pri, .. }
            | Self::Shot { pri, .. }
            | Self::Demo { pri, .. } => *pri,
        }
    }
    pub fn dur(&self) -> f32 {
        match self {
            Self::Recovery { dur, .. } | Self::Challenge { dur, .. } => *dur,
            Self::Demo { down, .. } => *down,
            Self::Loss { .. } | Self::Shot { .. } => 0.0,
        }
    }
}

/// `pri`'s recoveries, in time order.
pub(crate) fn recoveries(frames: &[FrameView], pri: i32, cfg: &ScoreConfig) -> Vec<Episode> {
    let mut out = Vec::new();
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
        let Some(t0) = air_end else { continue };
        let dur = f.t - t0;
        let facing = f
            .ball
            .and_then(|b| c.forward_align(sub(b.p, c.p)))
            .is_some_and(|a| a >= cfg.facing_cos_min);
        let recovered = c.upright && facing;
        if recovered || dur > cfg.recovery_cap_s {
            let done = recovered && dur <= cfg.recovery_cap_s;
            out.push(Episode::Recovery {
                pri,
                t0,
                dur: dur.min(cfg.recovery_cap_s),
                done,
            });
            air_end = None;
        }
    }
    out
}

impl Episode {
    /// Own-half depth of a `Loss` (0 for other kinds).
    pub fn danger(&self) -> f32 {
        match self {
            Self::Loss { danger, .. } => *danger,
            _ => 0.0,
        }
    }
    /// A challenge that passed every arrival test.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Challenge { miss: 0, .. })
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

/// `pri`'s contests as 1st man, in time order; arrival is judged at contest start.
pub(crate) fn challenges(
    frames: &[FrameView],
    roles: &Roles,
    pri: i32,
    team: i32,
    cfg: &ScoreConfig,
) -> Vec<Episode> {
    let mut out: Vec<Episode> = Vec::new();
    let mut open = false; // the last episode is still in contact
    for (i, f) in frames.iter().enumerate() {
        let Some((c, opp, to_ball)) = contest_at(f, roles, i, pri, team, cfg) else {
            open = false;
            continue;
        };
        if open {
            if let Some(Episode::Challenge { t0, dur, .. }) = out.last_mut() {
                *dur = f.t - *t0;
            }
            continue;
        }
        open = true;
        let boost_ok = c
            .boost
            .is_some_and(|b| b as f32 / 2.55 >= cfg.challenge_boost_min);
        let face_ok = c
            .forward_align(to_ball)
            .is_some_and(|a| a >= cfg.facing_cos_min);
        let on_time = c.time_to_ball <= opp.time_to_ball * cfg.challenge_late_margin;
        let miss = [
            (boost_ok, MISS_BOOST),
            (face_ok, MISS_FACE),
            (on_time, MISS_LATE),
        ]
        .iter()
        .filter(|(ok, _)| !ok)
        .fold(0, |m, (_, bit)| m | bit);
        out.push(Episode::Challenge {
            pri,
            opp: opp.pri,
            t0: f.t,
            dur: 0.0,
            miss,
        });
    }
    out
}

/// `pri`'s touches that have a following touch (the loss-rate denominator).
pub(crate) fn followed_touches(events: &[Event], pri: i32) -> usize {
    touches(events).windows(2).filter(|w| w[0].0 == pri).count()
}

fn touches(events: &[Event]) -> Vec<(i32, Option<i32>, f32)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Touch { pri, team, t, .. } => Some((*pri, *team, *t)),
            _ => None,
        })
        .collect()
}

/// `pri`'s possession losses, in time order. Dead-ball and goal-ended
/// possessions have no next touch, so they are not losses.
pub(crate) fn losses(frames: &[FrameView], events: &[Event], pri: i32, team: i32) -> Vec<Episode> {
    let mut out = Vec::new();
    for w in touches(events).windows(2) {
        if w[0].0 != pri || !matches!(w[1].1, Some(nt) if nt != team) {
            continue;
        }
        let Some(i) = frame_at_time(frames, w[0].2) else {
            continue;
        };
        let f = &frames[i];
        let (Some(c), Some(ball)) = (f.car(pri), f.ball) else {
            continue;
        };
        // Forward (+) is toward the opponent goal; the deep own half is negative.
        let fwd = ball.p.y * c.attack_sign as f32;
        let opp_dist = f
            .cars
            .iter()
            .filter(|o| o.team != team)
            .map(|o| dist(o.p, ball.p))
            .fold(f32::INFINITY, f32::min);
        out.push(Episode::Loss {
            pri,
            t: w[0].2,
            at: [ball.p.x, ball.p.y],
            danger: (-fwd / BACK_WALL_Y).clamp(0.0, 1.0),
            opp_dist,
        });
    }
    out
}

/// A victim that hasn't reappeared by now is counted as down this long.
const DEMO_DOWN_CAP_S: f32 = 10.0;
/// The victim of a demo must leave the grid within this long of the event.
const DEMO_GONE_S: f32 = 1.0;
/// A goal this soon after a demo is credited to it.
const GOAL_AFTER_DEMO_S: f32 = 8.0;
const GRAVITY: f32 = 650.0;
/// Ball bounce restitution off the floor.
const RESTITUTION: f32 = 0.6;
/// Goal mouth half-width and height (uu).
const GOAL_HALF_W: f32 = 892.755;
const GOAL_H: f32 = 642.775;
/// A shot counter's tick may lag the strike; a touch this far back is still the shooter's.
const STRIKE_LAG_S: f32 = 1.5;
/// A save follows its shot by at most this long.
const SAVE_WITHIN_S: f32 = 5.0;
/// A goal follows its shot by at most this long.
const GOAL_WITHIN_S: f32 = 10.0;
/// Half-width (uu) of the lane in front of the ball in which an opponent counts as a defender.
const DEF_LANE: f32 = 900.0;
/// Trajectories that take longer than this to reach the goal plane are not projected.
const MAX_TTA_S: f32 = 5.0;

/// Where a ball at `p` with velocity `v` crosses the plane `y = goal_y`: `(x, z, time)`.
/// Steps at 60 Hz under gravity, bouncing off the floor and the side walls; `None` if
/// it isn't heading to the plane within `MAX_TTA_S`.
fn goal_plane_crossing(mut p: Vec3, mut v: Vec3, goal_y: f32) -> Option<(f32, f32, f32)> {
    const DT: f32 = 1.0 / 60.0;
    if (goal_y - p.y) * v.y <= 0.0 {
        return None;
    }
    let mut t = 0.0;
    while t < MAX_TTA_S {
        let prev = p;
        v.z -= GRAVITY * DT;
        p = Vec3 {
            x: p.x + v.x * DT,
            y: p.y + v.y * DT,
            z: p.z + v.z * DT,
        };
        t += DT;
        if p.z < BALL_RADIUS {
            p.z = BALL_RADIUS;
            v.z = if v.z < -GRAVITY * DT {
                -v.z * RESTITUTION
            } else {
                0.0
            };
        }
        if p.x.abs() > SIDE_WALL_X - BALL_RADIUS {
            v.x = -v.x;
            p.x = prev.x;
        }
        if (p.y - goal_y) * (prev.y - goal_y) <= 0.0 {
            return Some((p.x, p.z, t));
        }
    }
    None
}

/// Every shot (scoreboard `Shot` counter), in time order. A goal is credited to the
/// scorer's latest earlier shot, a save to the latest unresolved opposing shot before
/// it; anything else is `Off`.
pub(crate) fn shots(m: &CanonicalMatch, frames: &[FrameView], xg: &XgModel) -> Vec<Episode> {
    let mut out = Vec::new();
    let mut shooters = Vec::new(); // (player name, team) per shot, aligned with `out`
    for e in &m.events {
        let Event::Stat {
            t: stat_t,
            pri,
            player,
            team: Some(team),
            kind: StatKind::Shot,
        } = e
        else {
            continue;
        };
        let strike = (m.events.iter())
            .filter_map(|e| match e {
                Event::Touch { t, pri: p, .. }
                    if p == pri && *t <= *stat_t && stat_t - t <= STRIKE_LAG_S =>
                {
                    Some(*t)
                }
                _ => None,
            })
            .next_back()
            .unwrap_or(*stat_t);
        let Some(i) = frame_at_time(frames, strike) else {
            continue;
        };
        // The ball a frame after the touch carries the post-touch velocity.
        let Some(ball) = frames[(i + 1).min(frames.len() - 1)].ball else {
            continue;
        };
        let Some(&sign) = m.resampled.team_attack_sign.get(team) else {
            continue;
        };
        let cross = goal_plane_crossing(ball.p, ball.v, sign as f32 * BACK_WALL_Y);
        let on_target = cross.is_some_and(|(x, z, _)| {
            x.abs() <= GOAL_HALF_W - BALL_RADIUS && z <= GOAL_H - BALL_RADIUS
        });
        let at = [ball.p.x * sign as f32, ball.p.y * sign as f32];
        let def = frames[i]
            .cars
            .iter()
            .filter(|c| c.team != *team)
            .filter(|c| {
                c.p.y * sign as f32 > at[1] && (c.p.x * sign as f32 - at[0]).abs() < DEF_LANE
            })
            .count() as u8;
        out.push(Episode::Shot {
            pri: *pri,
            t: strike,
            speed: dist(
                ball.v,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            ),
            aim: cross.map(|(x, z, _)| [x * sign as f32, z]),
            tta: cross.map(|(_, _, t)| t),
            on_target,
            outcome: Outcome::Off,
            at,
            def,
            xg: 0.0,
        });
        let last = out.len() - 1;
        let p = xg.predict(&out[last]);
        if let Episode::Shot { xg: x, .. } = &mut out[last] {
            *x = p;
        }
        shooters.push((player.clone(), *team));
    }
    // Resolve outcomes in time order so each goal/save claims one shot.
    let mut claimed = vec![false; out.len()];
    let mut claim =
        |out: &mut [Episode], at: f32, within: f32, ok: &dyn Fn(usize) -> bool, to: Outcome| {
            let hit = (0..out.len())
                .rev()
                .find(|&k| !claimed[k] && ok(k) && out[k].t() <= at && at - out[k].t() <= within);
            if let Some(k) = hit {
                claimed[k] = true;
                if let Episode::Shot {
                    outcome, on_target, ..
                } = &mut out[k]
                {
                    (*outcome, *on_target) = (to, true);
                }
            }
        };
    // Goals are authoritative, so they claim first; saves take what is left.
    for e in &m.events {
        if let Event::Goal {
            t,
            scorer: Some(name),
            ..
        } = e
        {
            claim(
                &mut out,
                *t,
                GOAL_WITHIN_S,
                &|k| shooters[k].0.as_deref() == Some(name),
                Outcome::Goal,
            );
        }
    }
    for e in &m.events {
        if let Event::Stat {
            t,
            team: Some(team),
            kind: StatKind::Save,
            ..
        } = e
        {
            claim(
                &mut out,
                *t,
                SAVE_WITHIN_S,
                &|k| shooters[k].1 != *team,
                Outcome::Saved,
            );
        }
    }
    out
}

/// Every demolition with a known attacker and victim, in time order.
pub(crate) fn demos(m: &CanonicalMatch, frames: &[FrameView]) -> Vec<Episode> {
    let team_of = |pri: i32| m.tracks.iter().find(|t| t.pri == pri).and_then(|t| t.team);
    m.events
        .iter()
        .filter_map(|e| {
            let Event::Demo {
                t,
                attacker_pri: Some(a),
                victim_pri: Some(v),
                ..
            } = e
            else {
                return None;
            };
            // The victim is out from the demo until their car is back on the grid; one that
            // never leaves it within `DEMO_GONE_S` of the event was not actually removed.
            let mut later = frames.iter().skip_while(|f| f.t < *t - 0.2);
            let gone = later.find(|f| f.t <= *t + DEMO_GONE_S && f.car(*v).is_none());
            let down = gone.map_or(0.0, |_| {
                let back = later.find(|f| f.car(*v).is_some());
                back.map_or(DEMO_DOWN_CAP_S, |f| f.t - t)
                    .min(DEMO_DOWN_CAP_S)
            });
            let goal = m.events.iter().any(|g| {
                matches!(g, Event::Goal { t: gt, team, .. }
                    if *team == team_of(*a) && (*t..=*t + GOAL_AFTER_DEMO_S).contains(gt))
            });
            Some(Episode::Demo {
                pri: *a,
                victim: *v,
                t: *t,
                down,
                goal,
            })
        })
        .collect()
}

/// Every player's episodes, in time order (shots scored by the prior xG model).
pub fn extract(m: &CanonicalMatch, cfg: &ScoreConfig) -> Vec<Episode> {
    extract_with(m, cfg, &XgModel::default())
}

/// [`extract`] with a fitted xG model.
pub fn extract_with(m: &CanonicalMatch, cfg: &ScoreConfig, xg: &XgModel) -> Vec<Episode> {
    let frames = build_frames(m, cfg);
    let roles = roles::assign(&frames, &crate::teams(m), cfg);
    let mut out: Vec<Episode> = Vec::new();
    for t in &m.tracks {
        out.extend(recoveries(&frames, t.pri, cfg));
        let team = t.team.unwrap_or(0);
        out.extend(challenges(&frames, &roles, t.pri, team, cfg));
        out.extend(losses(&frames, &m.events, t.pri, team));
    }
    out.extend(shots(m, &frames, xg));
    out.extend(demos(m, &frames));
    out.sort_by(|a, b| a.t().total_cmp(&b.t()).then(a.pri().cmp(&b.pri())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::{CarView, Third};
    use replay_analyzer::model::{Kin, Rot3, Vec3};

    const Z: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// One frame at `i × 0.1 s`; the ball sits ahead at +x, so `yaw` 0 faces it and π faces away.
    fn frame(i: usize, car: Option<(bool, f32)>) -> FrameView {
        let cars = car.map_or(vec![], |(airborne, yaw)| {
            vec![CarView {
                pri: 1,
                team: 0,
                p: Z,
                v: Z,
                pa: Z,
                boost: None,
                valid_pos: true,
                dist_to_ball: 0.0,
                closing_speed: 0.0,
                time_to_ball: 0.0,
                goalside: true,
                ball_third: Third::Mid,
                rot: Some(Rot3 {
                    pitch: 0.0,
                    yaw,
                    roll: 0.0,
                }),
                airborne,
                upright: true,
                attack_sign: 1,
            }]
        });
        let ball = Some(Kin {
            p: Vec3 { x: 1000.0, ..Z },
            v: Z,
        });
        FrameView {
            t: i as f32 * 0.1,
            ball,
            cars,
        }
    }

    fn run(cars: &[Option<(bool, f32)>]) -> Vec<Episode> {
        let frames: Vec<_> = cars.iter().enumerate().map(|(i, c)| frame(i, *c)).collect();
        recoveries(&frames, 1, &ScoreConfig::default())
    }

    #[test]
    fn landing_facing_the_ball_is_a_done_recovery() {
        let eps = run(&[Some((true, 0.0)), Some((true, 0.0)), Some((false, 0.0))]);
        let [Episode::Recovery {
            pri: 1,
            t0,
            dur,
            done: true,
        }] = eps[..]
        else {
            panic!("{eps:?}")
        };
        assert!(
            (t0 - 0.1).abs() < 1e-5 && (dur - 0.1).abs() < 1e-5,
            "{eps:?}"
        );
    }

    #[test]
    fn never_facing_the_ball_times_out_at_the_cap() {
        let mut cars = vec![Some((true, 0.0))];
        cars.extend((0..40).map(|_| Some((false, std::f32::consts::PI))));
        let eps = run(&cars);
        let cap = ScoreConfig::default().recovery_cap_s;
        assert!(
            matches!(eps[..], [Episode::Recovery { dur, done: false, .. }] if dur == cap),
            "{eps:?}"
        );
    }

    #[test]
    fn a_track_gap_abandons_the_pending_recovery() {
        assert!(run(&[Some((true, 0.0)), None, Some((false, 0.0))]).is_empty());
    }

    /// One frame: pri 1 (blue) vs pri 2 (orange) at the ball; pri 1 has the given
    /// boost byte, yaw and time-to-ball, pri 2 arrives at 1.0 s.
    fn duel(i: usize, boost: u8, yaw: f32, ttb: f32) -> FrameView {
        let mut f = frame(i, Some((false, yaw)));
        let mut me = f.cars.remove(0);
        me.p = Vec3 { x: 500.0, ..Z };
        me.boost = Some(boost);
        me.time_to_ball = ttb;
        let opp = CarView {
            pri: 2,
            team: 1,
            p: Vec3 { x: 1500.0, ..Z },
            time_to_ball: 1.0,
            ..me
        };
        f.cars = vec![me, opp];
        f
    }

    fn miss_of(boost: u8, yaw: f32, ttb: f32) -> Vec<Episode> {
        let cfg = ScoreConfig::default();
        let frames: Vec<_> = (0..3).map(|i| duel(i, boost, yaw, ttb)).collect();
        let roles = roles::assign(&frames, &[0, 1], &cfg);
        challenges(&frames, &roles, 1, 0, &cfg)
    }

    #[test]
    fn a_clean_arrival_is_one_ok_challenge_spanning_the_contest() {
        let eps = miss_of(255, 0.0, 0.5);
        let [Episode::Challenge {
            opp: 2,
            t0,
            dur,
            miss: 0,
            ..
        }] = eps[..]
        else {
            panic!("{eps:?}")
        };
        assert!(t0 == 0.0 && (dur - 0.2).abs() < 1e-5, "{eps:?}");
    }

    #[test]
    fn each_failed_test_sets_its_own_bit() {
        let miss = |e: Vec<Episode>| match e[..] {
            [Episode::Challenge { miss, .. }] => miss,
            _ => panic!("{e:?}"),
        };
        assert_eq!(miss(miss_of(0, 0.0, 0.5)), MISS_BOOST);
        assert_eq!(miss(miss_of(255, std::f32::consts::PI, 0.5)), MISS_FACE);
        assert_eq!(miss(miss_of(255, 0.0, 2.0)), MISS_LATE);
        assert_eq!(miss(miss_of(0, std::f32::consts::PI, 2.0)), 7);
    }

    fn touch(t: f32, pri: i32, team: Option<i32>) -> Event {
        Event::Touch {
            t,
            pri,
            player: None,
            team,
        }
    }

    #[test]
    fn a_loss_needs_a_known_opposing_next_touch() {
        // pri 1 (blue) at y = -2560 with the ball, attacking +y: half the back wall deep.
        let mut f = duel(0, 255, 0.0, 0.5);
        f.ball = Some(replay_analyzer::model::Kin {
            p: Vec3 {
                x: 0.0,
                y: -2560.0,
                z: 0.0,
            },
            v: Z,
        });
        f.cars[0].attack_sign = 1;
        f.cars[1].p = Vec3 {
            x: 300.0,
            y: -2560.0,
            z: 0.0,
        };
        let frames = [f];
        let run = |next: Option<i32>| {
            losses(
                &frames,
                &[touch(0.0, 1, Some(0)), touch(1.0, 2, next)],
                1,
                0,
            )
        };
        let [Episode::Loss {
            t,
            danger,
            opp_dist,
            ..
        }] = run(Some(1))[..]
        else {
            panic!("{:?}", run(Some(1)))
        };
        assert!(t == 0.0 && (danger - 0.5).abs() < 1e-3 && (opp_dist - 300.0).abs() < 1e-3);
        assert!(
            run(None).is_empty(),
            "unknown-team next touch is not a loss"
        );
        assert!(
            run(Some(0)).is_empty(),
            "a teammate's touch keeps possession"
        );
    }

    const V: fn(f32, f32, f32) -> Vec3 = |x, y, z| Vec3 { x, y, z };

    #[test]
    fn a_straight_shot_crosses_the_goal_plane_where_it_is_aimed() {
        // 2000 uu/s down the field from mid-air, centred: drops under gravity to z ≈ 0.5·g·t².
        let (x, z, t) =
            goal_plane_crossing(V(0.0, 0.0, 400.0), V(0.0, 2000.0, 0.0), BACK_WALL_Y).unwrap();
        assert!((t - BACK_WALL_Y / 2000.0).abs() < 0.02 && x.abs() < 1e-3);
        assert!(
            (z - (400.0 - 0.5 * GRAVITY * t * t).max(BALL_RADIUS)).abs() < 40.0,
            "{z}"
        );
    }

    #[test]
    fn a_rolling_ball_stays_on_the_floor_and_a_backwards_one_never_arrives() {
        let (_, z, _) =
            goal_plane_crossing(V(0.0, 0.0, BALL_RADIUS), V(0.0, 1500.0, 0.0), BACK_WALL_Y)
                .unwrap();
        assert!((z - BALL_RADIUS).abs() < 1.0, "{z}");
        assert!(
            goal_plane_crossing(V(0.0, 0.0, 100.0), V(0.0, -1500.0, 0.0), BACK_WALL_Y).is_none()
        );
        assert!(
            goal_plane_crossing(V(0.0, 0.0, 100.0), V(0.0, 100.0, 0.0), BACK_WALL_Y).is_none(),
            "too slow"
        );
    }
}
