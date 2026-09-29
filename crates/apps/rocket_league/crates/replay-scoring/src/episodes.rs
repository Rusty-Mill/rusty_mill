//! Episodes: the moments behind the aggregate metrics.
//!
//! Each emitter holds the scan loop of one metric, and the metric is a reducer
//! over its episodes — so the list a player sees and the score they get cannot
//! drift apart. Design: `docs/episodes-design.md`.

use replay_analyzer::model::CanonicalMatch;
use serde::Serialize;

use crate::config::ScoreConfig;
use crate::features::{build_frames, sub, FrameView};

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
}

impl Episode {
    /// Start time (s).
    pub fn t(&self) -> f32 {
        match self {
            Self::Recovery { t0, .. } => *t0,
        }
    }
    pub fn pri(&self) -> i32 {
        match self {
            Self::Recovery { pri, .. } => *pri,
        }
    }
    pub fn dur(&self) -> f32 {
        match self {
            Self::Recovery { dur, .. } => *dur,
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

/// Every player's episodes, in time order.
pub fn extract(m: &CanonicalMatch, cfg: &ScoreConfig) -> Vec<Episode> {
    let frames = build_frames(m, cfg);
    let mut out: Vec<Episode> = m
        .tracks
        .iter()
        .flat_map(|t| recoveries(&frames, t.pri, cfg))
        .collect();
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
}
