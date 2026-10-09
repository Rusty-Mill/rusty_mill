//! Possession chains: a team's run of touches, with what it gained and how it ended.
//!
//! The unit is explicit (every number below depends on it): a chain is a maximal run
//! of **one team's touches** where each touch follows the last by at most
//! [`CHAIN_GAP_S`]. It ends when the other team touches the ball (`Lost`), when the
//! ball is loose for longer, or at a kickoff or goal (`Dead`) — unless the team
//! scored (`Goal`) or took a shot (`Shot`) from it within [`END_S`] of its last touch.
//! Value is the sum of the chain's per-touch ΔV (`replay-value`) and the xG of its shots.

use replay_analyzer::model::{CanonicalMatch, Event};
use replay_value::TouchValue;
use serde::Serialize;

use crate::episodes::Episode;

/// The ball may be loose this long between a team's touches without ending its chain.
pub const CHAIN_GAP_S: f32 = 3.0;
/// A goal or shot this soon after the last touch still belongs to the chain.
const END_S: f32 = 2.0;

/// How a chain ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainEnd {
    Goal,
    Shot,
    /// The other team touched the ball.
    Lost,
    /// A stoppage, or the ball went loose for too long.
    Dead,
}

struct Open {
    team: i32,
    pri: i32,
    t0: f32,
    last: f32,
    touches: u8,
    y0: f32,
    y1: f32,
}

/// Ball y in `team`'s attacking frame at `t` (+ toward the goal it attacks).
fn attack_y(m: &CanonicalMatch, t: f32, team: i32) -> f32 {
    let frames = &m.resampled.frames;
    let i = frames
        .partition_point(|f| f.t < t)
        .min(frames.len().saturating_sub(1));
    let sign = m
        .resampled
        .team_attack_sign
        .get(&team)
        .copied()
        .unwrap_or(1) as f32;
    frames
        .get(i)
        .and_then(|f| f.ball)
        .map_or(0.0, |b| b.p.y * sign)
}

fn close(
    m: &CanonicalMatch,
    c: Open,
    reason: ChainEnd,
    dv: &[TouchValue],
    shots: &[Episode],
) -> Episode {
    let until = c.last + END_S;
    let mine = |t: f32| (c.t0..=until).contains(&t);
    let scored = m
        .events
        .iter()
        .any(|e| matches!(e, Event::Goal { t, team: Some(g), .. } if *g == c.team && mine(*t)));
    let team_shots = shots.iter().filter(|s| {
        matches!(s, Episode::Shot { .. }) && mine(s.t()) && team_of(m, s.pri()) == Some(c.team)
    });
    let xg = team_shots.clone().map(Episode::xg).sum();
    let end = if scored {
        ChainEnd::Goal
    } else if team_shots.count() > 0 {
        ChainEnd::Shot
    } else {
        reason
    };
    Episode::Chain {
        pri: c.pri,
        team: c.team,
        t: c.t0,
        dur: c.last - c.t0,
        touches: c.touches,
        gained: c.y1 - c.y0,
        end,
        dv: dv
            .iter()
            .filter(|v| v.team == Some(c.team) && v.t >= c.t0 - 1e-4 && v.t <= c.last + 1e-4)
            .map(|v| v.dv)
            .sum(),
        xg,
    }
}

fn team_of(m: &CanonicalMatch, pri: i32) -> Option<i32> {
    m.tracks.iter().find(|t| t.pri == pri).and_then(|t| t.team)
}

/// Every possession chain in the match, in time order. `dv` is the match's per-touch ΔV
/// and `shots` its shot episodes (for xG and the shot/goal endings).
pub fn chains(m: &CanonicalMatch, dv: &[TouchValue], shots: &[Episode]) -> Vec<Episode> {
    let mut out = Vec::new();
    let mut cur: Option<Open> = None;
    for e in &m.events {
        match e {
            Event::Touch {
                t,
                pri,
                team: Some(team),
                ..
            } => {
                match cur.as_mut() {
                    Some(c) if c.team == *team && t - c.last <= CHAIN_GAP_S => {
                        c.last = *t;
                        c.touches = c.touches.saturating_add(1);
                        c.y1 = attack_y(m, *t, *team);
                        continue;
                    }
                    _ => {}
                }
                if let Some(c) = cur.take() {
                    let reason = if c.team != *team && t - c.last <= CHAIN_GAP_S {
                        ChainEnd::Lost
                    } else {
                        ChainEnd::Dead
                    };
                    out.push(close(m, c, reason, dv, shots));
                }
                let y = attack_y(m, *t, *team);
                cur = Some(Open {
                    team: *team,
                    pri: *pri,
                    t0: *t,
                    last: *t,
                    touches: 1,
                    y0: y,
                    y1: y,
                });
            }
            Event::Kickoff { .. } | Event::Goal { .. } => {
                if let Some(c) = cur.take() {
                    out.push(close(m, c, ChainEnd::Dead, dv, shots));
                }
            }
            _ => {}
        }
    }
    if let Some(c) = cur.take() {
        out.push(close(m, c, ChainEnd::Dead, dv, shots));
    }
    out
}
