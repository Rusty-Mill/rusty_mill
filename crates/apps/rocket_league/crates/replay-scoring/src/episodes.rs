//! Episodes: the moments behind the aggregate metrics.
//!
//! Each emitter holds the scan loop of one metric, and the metric is a reducer
//! over its episodes — so the list a player sees and the score they get cannot
//! drift apart. Design: `docs/episodes-design.md`.

use replay_analyzer::model::{CanonicalMatch, Vec3};
use serde::Serialize;

use crate::config::ScoreConfig;
use crate::features::{build_frames, dist, sub, CarView, FrameView};
use crate::roles::{self, ManRole, Roles};

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
}

pub const MISS_BOOST: u8 = 1;
pub const MISS_FACE: u8 = 2;
pub const MISS_LATE: u8 = 4;

impl Episode {
    /// Start time (s).
    pub fn t(&self) -> f32 {
        match self {
            Self::Recovery { t0, .. } | Self::Challenge { t0, .. } => *t0,
        }
    }
    pub fn pri(&self) -> i32 {
        match self {
            Self::Recovery { pri, .. } | Self::Challenge { pri, .. } => *pri,
        }
    }
    pub fn dur(&self) -> f32 {
        match self {
            Self::Recovery { dur, .. } | Self::Challenge { dur, .. } => *dur,
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

/// Every player's episodes, in time order.
pub fn extract(m: &CanonicalMatch, cfg: &ScoreConfig) -> Vec<Episode> {
    let frames = build_frames(m, cfg);
    let roles = roles::assign(&frames, &crate::teams(m), cfg);
    let mut out: Vec<Episode> = Vec::new();
    for t in &m.tracks {
        out.extend(recoveries(&frames, t.pri, cfg));
        out.extend(challenges(&frames, &roles, t.pri, t.team.unwrap_or(0), cfg));
    }
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
}
