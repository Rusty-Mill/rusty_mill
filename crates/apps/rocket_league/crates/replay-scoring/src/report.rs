//! The scoring output: a per-player decision-discipline [`Report`].

use crate::config::Role;
use serde::{Deserialize, Serialize};

/// Report confidence; `LowConfidence` suppresses leaderboard eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Ok,
    LowConfidence,
}

/// Per-metric breakdown (raw signal + normalized 0–100 + effective weight).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricBreakdown {
    pub key: String,
    pub role: Role,
    /// Raw metric value, or `None` if the player had no frames to compute it.
    pub raw: Option<f32>,
    /// Calibrated 0–100 score.
    pub normalized: f32,
    /// Effective composite weight = top_weight[role] · within-role weight.
    pub effective_weight: f32,
}

/// A full decision-discipline report for one target player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub replay_id: String,
    pub score_config_version: String,
    pub parser_version: String,
    pub analyzer_version: String,

    pub target_player: String,
    pub target_pri: i32,
    pub target_team: Option<i32>,

    /// Composite decision-discipline score, 0–100.
    pub composite: f32,
    pub first_man: f32,
    pub second_man: f32,
    pub general: f32,

    pub licence: String,
    pub player_type: String,

    /// Highest-impact deficit (metric key) and the chapter it maps to.
    pub main_leak: String,
    pub focus_chapter: String,

    pub confidence: Confidence,
    pub metrics: Vec<MetricBreakdown>,
}
