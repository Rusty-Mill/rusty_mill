//! The shipped value-model surface: a [`Predict`] trait both learners implement,
//! and a tagged [`ValuePredictor`] enum that serializes either to
//! `value_model.json`.
//!
//! The production corpus model is gradient-boosted ([`crate::gbt::GbtModel`], which
//! beats the logistic model on the held-out corpus — see `train_corpus`); the
//! per-match [`crate::evaluate`] path stays logistic since one replay's few hundred
//! rows are too few to boost. Consumers predict through `Predict` / `ValuePredictor`
//! and don't care which learner produced the surface.

use serde::{Deserialize, Serialize};

use crate::dataset::Dataset;
use crate::features::N_FEATURES;
use crate::gbt::GbtModel;
use crate::model::ValueModel;

/// Maps a state feature vector to `P(team scores next within horizon | state)`.
/// Lets the ΔV evaluators work over any value model without caring about its kind.
pub trait Predict {
    fn predict(&self, x: &[f32; N_FEATURES]) -> f32;
}

impl Predict for ValueModel {
    fn predict(&self, x: &[f32; N_FEATURES]) -> f32 {
        ValueModel::predict(self, x)
    }
}

impl Predict for GbtModel {
    fn predict(&self, x: &[f32; N_FEATURES]) -> f32 {
        GbtModel::predict(self, x)
    }
}

/// A trained value model of either kind — the form stored in `value_model.json`
/// (`{"kind": "gbt"|"logistic", "model": { … }}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "model", rename_all = "snake_case")]
pub enum ValuePredictor {
    Logistic(ValueModel),
    Gbt(GbtModel),
}

impl ValuePredictor {
    /// `P(team scores next within horizon | state)`.
    pub fn predict(&self, x: &[f32; N_FEATURES]) -> f32 {
        match self {
            ValuePredictor::Logistic(m) => m.predict(x),
            ValuePredictor::Gbt(m) => m.predict(x),
        }
    }

    /// Rows the model was trained on (0 ⇒ untrained constant-0.5).
    pub fn n_train(&self) -> usize {
        match self {
            ValuePredictor::Logistic(m) => m.n_train,
            ValuePredictor::Gbt(m) => m.n_train,
        }
    }

    /// Mean binary cross-entropy over a dataset (a diagnostic; either kind).
    pub fn log_loss(&self, ds: &Dataset) -> f32 {
        if ds.rows.is_empty() {
            return 0.0;
        }
        let eps = 1e-7;
        let sum: f32 = ds
            .rows
            .iter()
            .map(|r| {
                let p = self.predict(&r.x).clamp(eps, 1.0 - eps);
                -(r.y * p.ln() + (1.0 - r.y) * (1.0 - p).ln())
            })
            .sum();
        sum / ds.rows.len() as f32
    }
}

impl Predict for ValuePredictor {
    fn predict(&self, x: &[f32; N_FEATURES]) -> f32 {
        ValuePredictor::predict(self, x)
    }
}
