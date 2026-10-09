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
use crate::features::{features_at, frame_index_at};
use crate::predictor::Predict;

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
    /// Touches whose ΔV was negative — **giveaways** (a touch that moved the
    /// player's own team away from scoring next): the outcome-weighted turnover.
    pub giveaways: usize,
    /// Total scoring probability bled on those giveaways — the **magnitude** of the
    /// negative swings, reported as a positive number (`Σ −min(dv, 0)`).
    pub lost_dv: f32,
}

/// One touch's value swing — the building block [`per_player_delta_v`] sums.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TouchValue {
    /// Match time of the touch (s).
    pub t: f32,
    /// Toucher (PRI).
    pub pri: i32,
    /// Toucher's team — the frame ΔV was evaluated in.
    pub team: Option<i32>,
    /// `ΔV = V(after) − V(before)`, signed: positive moved the toucher's team
    /// toward scoring next; negative away (a giveaway).
    pub dv: f32,
}

/// Per-touch ΔV for every touch with a known toucher/team and a computable
/// before/after state, in event (time) order. A touch is credited the change it
/// caused in its team's scoring probability over `cfg.touch_post_delay_s`,
/// evaluated in the toucher's attacking frame. [`per_player_delta_v`] aggregates
/// this; consumers (e.g. linking skills to outcomes) can use the per-touch swings
/// directly.
pub fn per_touch_delta_v<P: Predict>(
    m: &CanonicalMatch,
    model: &P,
    cfg: &ValueConfig,
) -> Vec<TouchValue> {
    let frames = &m.resampled.frames;
    let signs = &m.resampled.team_attack_sign;
    let mut out = Vec::new();
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
            features_at(frames, i0, *team, signs, &m.events),
            features_at(frames, i1, *team, signs, &m.events),
        ) else {
            continue;
        };
        let dv = model.predict(&s1.x) - model.predict(&s0.x);
        out.push(TouchValue {
            t: *t,
            pri: *pri,
            team: Some(*team),
            dv,
        });
    }
    out
}

/// Compute per-player ΔV over all touches with a known toucher and team, by
/// summing the [`per_touch_delta_v`] credits per player.
pub fn per_player_delta_v<P: Predict>(
    m: &CanonicalMatch,
    model: &P,
    cfg: &ValueConfig,
) -> Vec<PlayerValue> {
    let names: BTreeMap<i32, String> = m.tracks.iter().map(|t| (t.pri, t.player.clone())).collect();
    let teams: BTreeMap<i32, Option<i32>> = m.tracks.iter().map(|t| (t.pri, t.team)).collect();

    // pri -> (touch count, ΔV sum, giveaway count, bled magnitude), from the
    // per-touch credits. A negative-ΔV touch is a giveaway; its magnitude is the
    // scoring probability handed to the opponent.
    let mut acc: BTreeMap<i32, (usize, f32, usize, f32)> = BTreeMap::new();
    for tv in per_touch_delta_v(m, model, cfg) {
        let entry = acc.entry(tv.pri).or_insert((0, 0.0, 0, 0.0));
        entry.0 += 1;
        entry.1 += tv.dv;
        if tv.dv < 0.0 {
            entry.2 += 1;
            entry.3 += -tv.dv;
        }
    }

    let mut out: Vec<PlayerValue> = acc
        .into_iter()
        .map(|(pri, (touches, sum_dv, giveaways, lost_dv))| PlayerValue {
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
            giveaways,
            lost_dv,
        })
        .collect();
    // Most impactful first.
    out.sort_by(|a, b| b.sum_dv.total_cmp(&a.sum_dv));
    out
}
