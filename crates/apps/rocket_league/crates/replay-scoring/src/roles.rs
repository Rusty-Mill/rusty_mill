//! Per-frame 1st/2nd-man role assignment with hysteresis.
//!
//! The committed/pressuring car (lowest `time_to_ball`) is **1st man**; the other
//! is **2nd man** (support). Naive per-frame assignment flickers during
//! rotations, so a role swap only registers after the new leader persists for
//! `role_min_persist_s` (spec §4.4).

use std::collections::BTreeMap;

use crate::config::ScoreConfig;
use crate::features::FrameView;

/// Which man the target is this frame, on its own team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManRole {
    First,
    Second,
}

/// Per-frame 1st-man PRI for each team.
pub struct Roles {
    /// `frames[i][team] = pri of the 1st man on that team at frame i`.
    first_by_team: Vec<BTreeMap<i32, i32>>,
}

impl Roles {
    /// The target's role on `team` at frame `i`, or `None` if the target has no
    /// live car there (e.g. demolished) so no role applies.
    pub fn role_of(&self, i: usize, team: i32, pri: i32, frame: &FrameView) -> Option<ManRole> {
        frame.car(pri)?; // target must be live
        match self.first_by_team.get(i).and_then(|m| m.get(&team)) {
            Some(first) if *first == pri => Some(ManRole::First),
            Some(_) => Some(ManRole::Second),
            None => None,
        }
    }

    /// PRI of the 1st man on `team` at frame `i`.
    pub fn first(&self, i: usize, team: i32) -> Option<i32> {
        self.first_by_team
            .get(i)
            .and_then(|m| m.get(&team))
            .copied()
    }
}

/// Assign roles across all frames for every team present.
pub fn assign(frames: &[FrameView], teams: &[i32], cfg: &ScoreConfig) -> Roles {
    // Per-team hysteresis state: committed 1st-man pri + a pending challenger.
    let mut committed: BTreeMap<i32, i32> = BTreeMap::new();
    let mut pending: BTreeMap<i32, (i32, f32)> = BTreeMap::new();

    let mut out: Vec<BTreeMap<i32, i32>> = Vec::with_capacity(frames.len());

    for f in frames {
        let mut frame_first = BTreeMap::new();
        for &team in teams {
            // Raw leader = live car on this team with the lowest time_to_ball.
            let raw = f
                .team_cars(team)
                .min_by(|a, b| a.time_to_ball.total_cmp(&b.time_to_ball))
                .map(|c| c.pri);

            let Some(raw) = raw else {
                // No live car: keep last committed (if any) for continuity.
                if let Some(c) = committed.get(&team) {
                    frame_first.insert(team, *c);
                }
                continue;
            };

            match committed.get(&team).copied() {
                None => {
                    committed.insert(team, raw);
                    pending.remove(&team);
                }
                Some(cur) if cur == raw => {
                    pending.remove(&team);
                }
                Some(_cur) => match pending.get(&team).copied() {
                    Some((p, since)) if p == raw => {
                        if f.t - since >= cfg.role_min_persist_s {
                            committed.insert(team, raw);
                            pending.remove(&team);
                        }
                    }
                    _ => {
                        pending.insert(team, (raw, f.t));
                    }
                },
            }

            if let Some(c) = committed.get(&team) {
                frame_first.insert(team, *c);
            }
        }
        out.push(frame_first);
    }

    Roles { first_by_team: out }
}
