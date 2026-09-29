//! Per-frame scoring views derived from the canonical resampled grid.
//!
//! Everything role/metric code needs, computed once: distance/time-to-ball,
//! attacking-frame position, goal-side flag, and which third the ball is in
//! relative to each car's team. Kickoff windows and post-demo respawn windows
//! are masked out of positional validity (per spec §11).

use replay_analyzer::analyze::normalize::flip_xy;
use replay_analyzer::model::{CanonicalMatch, Event, Kin, Rot3, Vec3};

use crate::config::ScoreConfig;

/// Field third the ball occupies, relative to a car's team (attacking `+Y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Third {
    Def,
    Mid,
    Att,
}

/// One car's derived state in a frame.
#[derive(Debug, Clone, Copy)]
pub struct CarView {
    pub pri: i32,
    pub team: i32,
    pub p: Vec3,
    pub v: Vec3,
    /// Position in this car's attacking frame (team attacks `+Y`).
    pub pa: Vec3,
    pub boost: Option<u8>,
    /// Valid for positional metrics (not in a kickoff or own respawn window).
    pub valid_pos: bool,
    pub dist_to_ball: f32,
    pub closing_speed: f32,
    pub time_to_ball: f32,
    /// Car is between the ball and its own goal (attacking frame: `pa.y < ball`).
    pub goalside: bool,
    pub ball_third: Third,
    /// Car orientation (carried from the last keyframe), `None` until observed.
    pub rot: Option<Rot3>,
    /// Car centre is above the airborne height threshold (aerial / off-ground).
    pub airborne: bool,
    /// Wheels-down attitude: |pitch| and |roll| within the upright threshold.
    pub upright: bool,
    /// This team's attack sign (multiply world x/y to view the team attacking +Y).
    pub attack_sign: i32,
}

impl CarView {
    /// World-space forward (heading) unit vector from yaw; `None` if no rotation
    /// has been observed for this car yet.
    pub fn facing(&self) -> Option<Vec3> {
        let r = self.rot?;
        Some(Vec3 {
            x: r.yaw.cos(),
            y: r.yaw.sin(),
            z: 0.0,
        })
    }

    /// Cosine alignment of the car's heading with `dir` in the XY plane, in
    /// `[-1, 1]`; `None` if the car has no rotation or `dir` is ~vertical.
    pub fn forward_align(&self, dir: Vec3) -> Option<f32> {
        let f = self.facing()?;
        let n = (dir.x * dir.x + dir.y * dir.y).sqrt();
        (n > f32::EPSILON).then(|| (f.x * dir.x + f.y * dir.y) / n)
    }
}

/// One resampled frame, scoring-annotated.
#[derive(Debug, Clone)]
pub struct FrameView {
    pub t: f32,
    pub ball: Option<Kin>,
    pub cars: Vec<CarView>,
}

impl FrameView {
    /// The car for `pri`, if present this frame.
    pub fn car(&self, pri: i32) -> Option<&CarView> {
        self.cars.iter().find(|c| c.pri == pri)
    }
    /// The other live car on `team` besides `pri` (teammate in 2v2).
    pub fn teammate(&self, team: i32, pri: i32) -> Option<&CarView> {
        self.cars.iter().find(|c| c.team == team && c.pri != pri)
    }
    /// Live cars on `team`.
    pub fn team_cars(&self, team: i32) -> impl Iterator<Item = &CarView> {
        self.cars.iter().filter(move |c| c.team == team)
    }
}

pub(crate) fn sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}
fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn norm(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

/// Build the per-frame scoring views for a match.
pub fn build_frames(m: &CanonicalMatch, cfg: &ScoreConfig) -> Vec<FrameView> {
    let signs = &m.resampled.team_attack_sign;

    // Kickoff exclusion intervals (global) and per-victim respawn intervals.
    let mut kickoff_windows: Vec<(f32, f32)> = Vec::new();
    let mut demo_windows: Vec<(i32, f32, f32)> = Vec::new();
    for e in &m.events {
        match e {
            Event::Kickoff { t } => kickoff_windows.push((*t, *t + cfg.kickoff_exclude_s)),
            Event::Demo {
                victim_pri: Some(v),
                t,
                ..
            } => demo_windows.push((*v, *t, *t + cfg.respawn_exclude_s)),
            _ => {}
        }
    }

    let in_kickoff = |t: f32| kickoff_windows.iter().any(|(a, b)| t >= *a && t <= *b);
    let in_respawn = |pri: i32, t: f32| {
        demo_windows
            .iter()
            .any(|(v, a, b)| *v == pri && t >= *a && t <= *b)
    };

    m.resampled
        .frames
        .iter()
        .map(|f| {
            let kickoff = in_kickoff(f.t);
            let cars = f
                .cars
                .iter()
                .map(|c| {
                    let team = c.team.unwrap_or(0);
                    let sign = signs.get(&team).copied().unwrap_or(1);
                    let pa = flip_xy(c.p, sign);

                    let (dist, closing, ttb, goalside, third) = match &f.ball {
                        Some(b) => {
                            let to_ball = sub(b.p, c.p);
                            let dist = norm(to_ball);
                            let dir = if dist > f32::EPSILON {
                                Vec3 {
                                    x: to_ball.x / dist,
                                    y: to_ball.y / dist,
                                    z: to_ball.z / dist,
                                }
                            } else {
                                Vec3 {
                                    x: 0.0,
                                    y: 0.0,
                                    z: 0.0,
                                }
                            };
                            let closing = dot(c.v, dir).max(0.0);
                            let ttb = dist / closing.max(50.0);
                            let ball_ya = flip_xy(b.p, sign).y;
                            let goalside = pa.y < ball_ya;
                            let third = if ball_ya < -cfg.third_y {
                                Third::Def
                            } else if ball_ya > cfg.third_y {
                                Third::Att
                            } else {
                                Third::Mid
                            };
                            (dist, closing, ttb, goalside, third)
                        }
                        None => (f32::INFINITY, 0.0, f32::INFINITY, false, Third::Mid),
                    };

                    let upright = c
                        .rot
                        .map(|r| {
                            r.pitch.abs() <= cfg.upright_max_rad
                                && r.roll.abs() <= cfg.upright_max_rad
                        })
                        .unwrap_or(false);

                    CarView {
                        pri: c.pri,
                        team,
                        p: c.p,
                        v: c.v,
                        pa,
                        boost: c.boost,
                        valid_pos: !kickoff && !in_respawn(c.pri, f.t) && f.ball.is_some(),
                        dist_to_ball: dist,
                        closing_speed: closing,
                        time_to_ball: ttb,
                        goalside,
                        ball_third: third,
                        rot: c.rot,
                        airborne: c.p.z > cfg.airborne_z_uu,
                        upright,
                        attack_sign: sign,
                    }
                })
                .collect();

            FrameView {
                t: f.t,
                ball: f.ball,
                cars,
            }
        })
        .collect()
}
