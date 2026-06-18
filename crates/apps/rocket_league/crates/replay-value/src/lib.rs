//! Outcome-grounded value model over the canonical match model.
//!
//! A second, learned consumer of [`replay_analyzer`]'s [`CanonicalMatch`],
//! parallel to (and independent of) the heuristic decision-discipline
//! `replay-scoring` crate. Where scoring maps kinematics to rule-based bands that
//! need a labeled rank corpus to calibrate, this crate learns directly from an
//! **objective** target — *did the team score the next goal within `T` seconds* —
//! which is extracted from the replay's own `Goal` events, so the numbers mean
//! something without any external corpus.
//!
//! Pipeline (all pure functions of `(CanonicalMatch, ValueConfig)`):
//! 1. [`features`] — symmetric per-team state vectors.
//! 2. [`dataset`] — objective labels + the training table (+ JSONL export for an
//!    external trainer).
//! 3. [`model`] — a dependency-free logistic value model `P(score next within T)`.
//! 4. [`eval`] — per-player ΔV: the scoring-probability swing each touch caused.

pub mod config;
pub mod dataset;
pub mod eval;
pub mod features;
pub mod gbt;
pub mod model;

use replay_analyzer::model::CanonicalMatch;

pub use config::{ValueConfig, VALUE_CONFIG_VERSION};
pub use dataset::{build_dataset, Dataset};
pub use eval::{per_player_delta_v, per_touch_delta_v, PlayerValue, TouchValue};
pub use gbt::{GbtConfig, GbtModel};
pub use model::ValueModel;

/// End-to-end value evaluation of one match: build the labeled dataset, fit the
/// value model, and credit per-player ΔV.
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub model: ValueModel,
    pub dataset_rows: usize,
    pub base_rate: f32,
    pub log_loss: f32,
    pub players: Vec<PlayerValue>,
}

/// Run the full pipeline with `cfg`.
pub fn evaluate(m: &CanonicalMatch, cfg: &ValueConfig) -> Evaluation {
    let dataset = build_dataset(m, cfg);
    let base_rate = if dataset.rows.is_empty() {
        0.0
    } else {
        dataset.rows.iter().map(|r| r.y).sum::<f32>() / dataset.rows.len() as f32
    };
    let model = ValueModel::train(&dataset, &cfg.train);
    let log_loss = model.log_loss(&dataset);
    let players = per_player_delta_v(m, &model, cfg);
    Evaluation {
        model,
        dataset_rows: dataset.rows.len(),
        base_rate,
        log_loss,
        players,
    }
}
