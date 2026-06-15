//! A pure, dependency-free logistic value model.
//!
//! Predicts `P(team scores next within horizon | state)`. Trained by full-batch
//! gradient descent on z-scored features — deterministic (zero-initialized,
//! fixed epochs/lr, no shuffle), so a given `(dataset, TrainConfig)` always
//! yields identical weights. This is intentionally simple: with the objective
//! outcome label it already produces a meaningful value surface, and it is the
//! drop-in slot for a heavier learner (GBM/NN) behind the same interface once a
//! multi-replay corpus exists.

use serde::{Deserialize, Serialize};

use crate::config::TrainConfig;
use crate::dataset::Dataset;
use crate::features::N_FEATURES;

/// Per-feature standardizer (z-score), captured from the training set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scaler {
    pub mean: [f32; N_FEATURES],
    pub std: [f32; N_FEATURES],
}

impl Scaler {
    /// Fit column mean/std over the rows; std floored to avoid divide-by-zero on
    /// constant columns.
    fn fit(rows: &[[f32; N_FEATURES]]) -> Scaler {
        let n = rows.len().max(1) as f32;
        let mut mean = [0.0f32; N_FEATURES];
        for r in rows {
            for j in 0..N_FEATURES {
                mean[j] += r[j];
            }
        }
        for m in &mut mean {
            *m /= n;
        }
        let mut var = [0.0f32; N_FEATURES];
        for r in rows {
            for j in 0..N_FEATURES {
                let d = r[j] - mean[j];
                var[j] += d * d;
            }
        }
        let mut std = [0.0f32; N_FEATURES];
        for j in 0..N_FEATURES {
            std[j] = (var[j] / n).sqrt().max(1e-6);
        }
        Scaler { mean, std }
    }

    fn apply(&self, x: &[f32; N_FEATURES]) -> [f32; N_FEATURES] {
        let mut z = [0.0f32; N_FEATURES];
        for j in 0..N_FEATURES {
            z[j] = (x[j] - self.mean[j]) / self.std[j];
        }
        z
    }
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

/// Linear score `b + w·z` over standardized features.
fn linear(w: &[f32; N_FEATURES], b: f32, z: &[f32; N_FEATURES]) -> f32 {
    b + w.iter().zip(z).map(|(wi, zi)| wi * zi).sum::<f32>()
}

/// A trained logistic value model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueModel {
    pub w: [f32; N_FEATURES],
    pub b: f32,
    pub scaler: Scaler,
    /// Number of rows trained on (0 ⇒ untrained: [`ValueModel::predict`] = 0.5).
    pub n_train: usize,
}

impl ValueModel {
    /// Train by full-batch gradient descent on z-scored features. An empty
    /// dataset yields the constant-0.5 model.
    pub fn train(ds: &Dataset, cfg: &TrainConfig) -> ValueModel {
        let xs: Vec<[f32; N_FEATURES]> = ds.rows.iter().map(|r| r.x).collect();
        let ys: Vec<f32> = ds.rows.iter().map(|r| r.y).collect();
        let scaler = Scaler::fit(&xs);
        let m = xs.len();
        if m == 0 {
            return ValueModel {
                w: [0.0; N_FEATURES],
                b: 0.0,
                scaler,
                n_train: 0,
            };
        }
        let zs: Vec<[f32; N_FEATURES]> = xs.iter().map(|x| scaler.apply(x)).collect();

        let mut w = [0.0f32; N_FEATURES];
        let mut b = 0.0f32;
        let inv_m = 1.0 / m as f32;
        for _ in 0..cfg.epochs {
            let mut gw = [0.0f32; N_FEATURES];
            let mut gb = 0.0f32;
            for (z, &y) in zs.iter().zip(&ys) {
                let err = sigmoid(linear(&w, b, z)) - y;
                gb += err;
                for j in 0..N_FEATURES {
                    gw[j] += err * z[j];
                }
            }
            b -= cfg.lr * gb * inv_m;
            for j in 0..N_FEATURES {
                w[j] -= cfg.lr * (gw[j] * inv_m + cfg.l2 * w[j]);
            }
        }
        ValueModel {
            w,
            b,
            scaler,
            n_train: m,
        }
    }

    /// `P(team scores next within horizon | state)` for raw (unscaled) features.
    pub fn predict(&self, x: &[f32; N_FEATURES]) -> f32 {
        if self.n_train == 0 {
            return 0.5;
        }
        let z = self.scaler.apply(x);
        sigmoid(linear(&self.w, self.b, &z))
    }

    /// Mean binary cross-entropy over a dataset (training diagnostic).
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
