//! Per-player evaluation via ΔV (an xT-style credit).
//!
//! A touch is credited the change it caused in its team's scoring probability:
//! `ΔV = V(state shortly after the touch) − V(state at the touch)`, where
//! `V = P(team scores next within horizon)` from the value model, evaluated in
//! the *toucher's* team frame. Summing ΔV per player ranks who moves the needle.

use std::collections::BTreeMap;

use replay_analyzer::model::{CanonicalMatch, Event};
use serde::Serialize;

use crate::config::ValueConfig;
use crate::features::{frame_index_at, state_features};
use crate::model::ValueModel;

/// One player's aggregated value contribution.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlayerValue {
    pub pri: i32,
    pub player: Option<String>,
    pub team: Option<i32>,
    /// Touches that had a computable before/after value.
    pub touches: usize,
    /// Total ΔV across the player's credited touches.
    pub sum_dv: f32,
    /// Mean ΔV per credited touch.
    pub mean_dv: f32,
}

/// Compute per-player ΔV over all touches with a known toucher and team.
pub fn per_player_delta_v(
    m: &CanonicalMatch,
    model: &ValueModel,
    cfg: &ValueConfig,
) -> Vec<PlayerValue> {
    let frames = &m.resampled.frames;
    let signs = &m.resampled.team_attack_sign;
    let names: BTreeMap<i32, String> = m.tracks.iter().map(|t| (t.pri, t.player.clone())).collect();
    let teams: BTreeMap<i32, Option<i32>> = m.tracks.iter().map(|t| (t.pri, t.team)).collect();

    // pri -> (touch count, ΔV sum).
    let mut acc: BTreeMap<i32, (usize, f32)> = BTreeMap::new();
    for e in &m.events {
        let Event::Touch {
            t,
            pri,
            team: Some(team),
            ..
        } = e
        else {
            continue;
        };
        let (Some(i0), Some(i1)) = (
            frame_index_at(frames, *t),
            frame_index_at(frames, *t + cfg.touch_post_delay_s),
        ) else {
            continue;
        };
        let (Some(s0), Some(s1)) = (
            state_features(&frames[i0], *team, signs),
            state_features(&frames[i1], *team, signs),
        ) else {
            continue;
        };
        let dv = model.predict(&s1.x) - model.predict(&s0.x);
        let entry = acc.entry(*pri).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += dv;
    }

    let mut out: Vec<PlayerValue> = acc
        .into_iter()
        .map(|(pri, (touches, sum_dv))| PlayerValue {
            pri,
            player: names.get(&pri).cloned(),
            team: teams.get(&pri).copied().flatten(),
            touches,
            sum_dv,
            mean_dv: if touches > 0 {
                sum_dv / touches as f32
            } else {
                0.0
            },
        })
        .collect();
    // Most impactful first.
    out.sort_by(|a, b| b.sum_dv.total_cmp(&a.sum_dv));
    out
}
