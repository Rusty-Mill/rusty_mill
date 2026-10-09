//! Lobby report assembly: every player in a match plus the §4.10 comparison
//! table — the assembled, product-facing artifact built on the pure scoring core.

use std::collections::BTreeMap;

use serde::Serialize;

use replay_analyzer::model::CanonicalMatch;

use crate::config::ScoreConfig;
use crate::report::Report;
use crate::score_all;

/// One metric across every player in the lobby, index-aligned to
/// [`LobbyReport::players`], with the leading player flagged.
#[derive(Debug, Clone, Serialize)]
pub struct MetricRow {
    pub key: String,
    /// Normalized 0–100 score per player; index-aligned to `players`.
    pub normalized: Vec<f32>,
    /// Index of the player with the highest normalized score, if any.
    pub leader: Option<usize>,
}

/// A full single-match report: every player scored, ordered by team then
/// composite, plus a side-by-side per-metric comparison table.
#[derive(Debug, Clone, Serialize)]
pub struct LobbyReport {
    pub replay_id: String,
    pub map: Option<String>,
    pub duration_s: f32,
    pub score_config_version: String,
    /// Final score per team id (0 = blue, 1 = orange).
    pub team_scores: BTreeMap<i32, i32>,
    /// Players, ordered by team then composite (best first within a team).
    pub players: Vec<Report>,
    /// Per-metric comparison across players.
    pub comparison: Vec<MetricRow>,
}

/// Score every (named) player and build the comparison table.
pub fn assemble(m: &CanonicalMatch, cfg: &ScoreConfig) -> LobbyReport {
    let mut players: Vec<Report> = score_all(m, cfg)
        .into_iter()
        .filter(|r| r.target_player != "<unknown>")
        .collect();
    players.sort_by(|a, b| {
        a.target_team
            .cmp(&b.target_team)
            .then(b.composite.total_cmp(&a.composite))
    });

    let comparison = cfg
        .metrics
        .iter()
        .map(|spec| {
            let key = spec.metric.key();
            let normalized: Vec<f32> = players
                .iter()
                .map(|p| {
                    p.metrics
                        .iter()
                        .find(|b| b.key == key)
                        .map(|b| b.normalized)
                        .unwrap_or(0.0)
                })
                .collect();
            let leader = normalized
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
                .map(|(i, _)| i);
            MetricRow {
                key: key.to_string(),
                normalized,
                leader,
            }
        })
        .collect();

    LobbyReport {
        replay_id: m.replay_id.clone(),
        map: m.map.clone(),
        duration_s: m.duration_s,
        score_config_version: cfg.version.clone(),
        team_scores: m.team_scores.clone(),
        players,
        comparison,
    }
}
