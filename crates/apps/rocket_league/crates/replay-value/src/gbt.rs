//! A dependency-free, deterministic gradient-boosted regression-trees value model.
//!
//! The heavier-learner counterpart to the logistic [`crate::model::ValueModel`],
//! behind the same `predict(&[f32; N_FEATURES]) -> f32` shape. Second-order
//! (XGBoost-style) boosting of the log-loss: each round fits a depth-limited
//! regression tree to the per-row gradient/hessian and shrinks it by the learning
//! rate. Trees capture feature *interactions* a linear model cannot. No randomness
//! (no subsampling, deterministic split search) — a given `(dataset, GbtConfig)`
//! always yields a byte-identical model, matching the crate's determinism ethos.

use serde::{Deserialize, Serialize};

use crate::dataset::Dataset;
use crate::features::N_FEATURES;

/// Hyper-parameters for [`GbtModel::train`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GbtConfig {
    /// Number of boosting rounds (trees).
    pub rounds: usize,
    /// Maximum tree depth (interaction order).
    pub max_depth: usize,
    /// Shrinkage applied to each tree's contribution.
    pub learning_rate: f32,
    /// Minimum rows in any leaf (regularizes splits).
    pub min_leaf: usize,
    /// L2 regularization on leaf weights (the `λ` in the gain).
    pub lambda: f32,
}

impl Default for GbtConfig {
    fn default() -> Self {
        GbtConfig {
            rounds: 150,
            max_depth: 3,
            learning_rate: 0.1,
            min_leaf: 50,
            lambda: 1.0,
        }
    }
}

/// A regression-tree node: a leaf weight, or a `< threshold` split on one feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Node {
    Leaf(f32),
    Split {
        feature: usize,
        threshold: f32,
        left: Box<Node>,
        right: Box<Node>,
    },
}

impl Node {
    fn eval(&self, x: &[f32; N_FEATURES]) -> f32 {
        match self {
            Node::Leaf(v) => *v,
            Node::Split {
                feature,
                threshold,
                left,
                right,
            } => {
                if x[*feature] < *threshold {
                    left.eval(x)
                } else {
                    right.eval(x)
                }
            }
        }
    }
}

/// A trained gradient-boosted value model. Predicts `P(team scores next within
/// horizon | state)` as `sigmoid(base + lr · Σ tree(x))`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GbtModel {
    /// Base score (log-odds of the training base rate).
    pub base: f32,
    pub learning_rate: f32,
    trees: Vec<Node>,
    /// Rows trained on (0 ⇒ untrained: [`GbtModel::predict`] = 0.5).
    pub n_train: usize,
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

/// Optimal leaf weight for the second-order objective: `−Σg / (Σh + λ)`.
fn leaf_value(gsum: f32, hsum: f32, lambda: f32) -> f32 {
    -gsum / (hsum + lambda)
}

/// Greedily grow one regression tree over `rows` against the current `(g, h)`.
fn build_node(
    xs: &[&[f32; N_FEATURES]],
    g: &[f32],
    h: &[f32],
    rows: Vec<usize>,
    depth: usize,
    cfg: &GbtConfig,
) -> Node {
    let gsum: f32 = rows.iter().map(|&i| g[i]).sum();
    let hsum: f32 = rows.iter().map(|&i| h[i]).sum();
    let leaf = || Node::Leaf(leaf_value(gsum, hsum, cfg.lambda));
    if depth >= cfg.max_depth || rows.len() < 2 * cfg.min_leaf {
        return leaf();
    }

    // Best split = the largest gain over the parent's score `Σg²/(Σh+λ)`. Scan
    // every feature × candidate threshold (midpoint of adjacent distinct values).
    let parent = gsum * gsum / (hsum + cfg.lambda);
    let mut best: Option<(f32, usize, f32)> = None; // (gain, feature, threshold)
                                                    // `j` is a feature-dimension index into each row's `[f32; N_FEATURES]`, so the
                                                    // range loop is the natural form (enumerate doesn't apply).
    #[allow(clippy::needless_range_loop)]
    for j in 0..N_FEATURES {
        let mut order = rows.clone();
        order.sort_by(|&a, &b| xs[a][j].total_cmp(&xs[b][j]));
        let (mut gl, mut hl) = (0.0f32, 0.0f32);
        for k in 0..order.len() - 1 {
            gl += g[order[k]];
            hl += h[order[k]];
            let (left_n, right_n) = (k + 1, order.len() - k - 1);
            if left_n < cfg.min_leaf || right_n < cfg.min_leaf {
                continue;
            }
            let (vk, vk1) = (xs[order[k]][j], xs[order[k + 1]][j]);
            if vk == vk1 {
                continue; // can't split between equal values
            }
            let (gr, hr) = (gsum - gl, hsum - hl);
            let gain = gl * gl / (hl + cfg.lambda) + gr * gr / (hr + cfg.lambda) - parent;
            if best.is_none_or(|(bg, _, _)| gain > bg) {
                best = Some((gain, j, 0.5 * (vk + vk1)));
            }
        }
    }

    let Some((gain, feature, threshold)) = best else {
        return leaf();
    };
    if gain <= 0.0 {
        return leaf();
    }
    let (mut left_rows, mut right_rows) = (Vec::new(), Vec::new());
    for i in rows {
        if xs[i][feature] < threshold {
            left_rows.push(i);
        } else {
            right_rows.push(i);
        }
    }
    Node::Split {
        feature,
        threshold,
        left: Box::new(build_node(xs, g, h, left_rows, depth + 1, cfg)),
        right: Box::new(build_node(xs, g, h, right_rows, depth + 1, cfg)),
    }
}

impl GbtModel {
    /// Fit by second-order gradient boosting on the log-loss. An empty dataset
    /// yields the constant-0.5 model (mirrors [`crate::model::ValueModel`]).
    pub fn train(ds: &Dataset, cfg: &GbtConfig) -> GbtModel {
        let n = ds.rows.len();
        if n == 0 {
            return GbtModel {
                base: 0.0,
                learning_rate: cfg.learning_rate,
                trees: Vec::new(),
                n_train: 0,
            };
        }
        let xs: Vec<&[f32; N_FEATURES]> = ds.rows.iter().map(|r| &r.x).collect();
        let ys: Vec<f32> = ds.rows.iter().map(|r| r.y).collect();
        let ybar = (ys.iter().sum::<f32>() / n as f32).clamp(1e-6, 1.0 - 1e-6);
        let base = (ybar / (1.0 - ybar)).ln();

        let mut f = vec![base; n];
        let mut trees = Vec::with_capacity(cfg.rounds);
        for _ in 0..cfg.rounds {
            let mut g = vec![0.0f32; n];
            let mut h = vec![0.0f32; n];
            for i in 0..n {
                let p = sigmoid(f[i]);
                g[i] = p - ys[i];
                h[i] = (p * (1.0 - p)).max(1e-6);
            }
            let tree = build_node(&xs, &g, &h, (0..n).collect(), 0, cfg);
            for (i, fi) in f.iter_mut().enumerate() {
                *fi += cfg.learning_rate * tree.eval(xs[i]);
            }
            trees.push(tree);
        }
        GbtModel {
            base,
            learning_rate: cfg.learning_rate,
            trees,
            n_train: n,
        }
    }

    /// `P(team scores next within horizon | state)` for raw (unscaled) features.
    pub fn predict(&self, x: &[f32; N_FEATURES]) -> f32 {
        if self.n_train == 0 {
            return 0.5;
        }
        let fx = self.base + self.learning_rate * self.trees.iter().map(|t| t.eval(x)).sum::<f32>();
        sigmoid(fx)
    }
}
