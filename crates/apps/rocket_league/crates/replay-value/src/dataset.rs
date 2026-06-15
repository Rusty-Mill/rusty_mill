//! Outcome labeling and dataset assembly.
//!
//! The label is an **objective outcome**, not a calibration guess: for a state
//! at time `t` viewed from `team`, `y = 1` iff `team` scores the next goal within
//! `horizon_s`, else `0`. Censored states — no goal yet *and* less than a full
//! horizon of match remaining — are dropped, so every retained label is certain.
//! This is exactly the "team-scores-next-within-T" target, and it needs no
//! labeled rank corpus: it is extracted from the replay's own `Goal` events.

use std::collections::BTreeSet;

use replay_analyzer::model::{CanonicalMatch, Event};
use serde::Serialize;

use crate::config::ValueConfig;
use crate::features::{state_features, N_FEATURES};

/// One labeled training row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    pub t: f32,
    pub team: i32,
    pub x: [f32; N_FEATURES],
    pub y: f32,
}

/// A labeled dataset over one match.
#[derive(Debug, Clone, PartialEq)]
pub struct Dataset {
    pub rows: Vec<Row>,
    pub horizon_s: f32,
}

/// Objective label for a state: `Some(1.0)` if `team` scores the next goal within
/// `horizon`, `Some(0.0)` if it is certain `team` does **not** score within the
/// horizon, `None` if the outcome is censored (match ends inside the horizon
/// before any goal).
pub fn next_goal_label(
    events: &[Event],
    t: f32,
    team: i32,
    horizon: f32,
    match_end: f32,
) -> Option<f32> {
    let next = events
        .iter()
        .filter_map(|e| match e {
            Event::Goal {
                t: gt, team: gteam, ..
            } if *gt > t => Some((*gt, *gteam)),
            _ => None,
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));

    match next {
        // A goal within the horizon settles the label either way.
        Some((gt, gteam)) if gt - t <= horizon => Some((gteam == Some(team)) as u8 as f32),
        // The next goal is beyond the horizon ⇒ nobody scored within it ⇒ 0.
        Some(_) => Some(0.0),
        // No future goal: only a 0 once we have observed a full horizon.
        None => (match_end - t >= horizon).then_some(0.0),
    }
}

/// Build the labeled dataset: stride over grid frames, emit one row per team per
/// sampled frame, keeping only rows with a certain (uncensored) label.
pub fn build_dataset(m: &CanonicalMatch, cfg: &ValueConfig) -> Dataset {
    let frames = &m.resampled.frames;
    let match_end = frames.last().map(|f| f.t).unwrap_or(0.0);
    let teams: Vec<i32> = m
        .tracks
        .iter()
        .filter_map(|t| t.team)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let stride = cfg.sample_stride.max(1);
    let mut rows = Vec::new();
    for f in frames.iter().step_by(stride) {
        for &team in &teams {
            let Some(y) = next_goal_label(&m.events, f.t, team, cfg.horizon_s, match_end) else {
                continue;
            };
            let Some(feat) = state_features(f, team, &m.resampled.team_attack_sign) else {
                continue;
            };
            rows.push(Row {
                t: feat.t,
                team,
                x: feat.x,
                y,
            });
        }
    }
    Dataset {
        rows,
        horizon_s: cfg.horizon_s,
    }
}

/// Serialize the dataset as newline-delimited JSON (one [`Row`] per line) — the
/// hand-off format for an external trainer (numpy/torch `json.loads` per line).
pub fn to_jsonl(ds: &Dataset) -> String {
    let mut out = String::new();
    for r in &ds.rows {
        // Row is plain data; serialization cannot fail.
        out.push_str(&serde_json::to_string(r).expect("serialize row"));
        out.push('\n');
    }
    out
}
