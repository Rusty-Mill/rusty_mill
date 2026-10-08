//! Reconcile the rule-based rubric against the outcome-grounded value model.
//!
//! Two independent tracks consume the same canonical match:
//! * this crate's **rubric** — heuristic decision-discipline metrics, calibrated
//!   to *rank* (a sparse, cross-match skill label: one tier per player);
//! * `replay-value`'s **ΔV** — the scoring-probability swing a player caused,
//!   summed over their touches. Dense, objective, and corpus-free.
//!
//! They measure *different* things — rank is cross-match skill, ΔV is in-match
//! impact within a same-rank lobby — so they will never agree perfectly. But a
//! discipline signal that genuinely matters should move *both*: agreement is
//! evidence the metric is real and not a rank-cohort artifact, and disagreement
//! flags a metric that tracks the ranked population without paying off in play.
//!
//! Pure: ΔV enters as a plain `f32`, so this carries no dependency on the value
//! crate (the binary computes ΔV and feeds it here). Spearman is reused from
//! [`crate::calibrate`].

use std::collections::BTreeMap;

use crate::calibrate::spearman;
use crate::config::{Curve, Metric, ScoreConfig};

/// Below this |ρ| a correlation is treated as "no signal" (sign meaningless).
const NEUTRAL: f32 = 0.05;

/// One player joined across both tracks (same player, same replay).
#[derive(Debug, Clone)]
pub struct CrossSample {
    /// Raw rubric metric values for the player.
    pub raws: BTreeMap<Metric, Option<f32>>,
    /// The player's composite rubric score.
    pub composite: f32,
    /// Rank tier, if the corpus manifest had one for this player.
    pub rank: Option<f32>,
    /// Total ΔV (scoring-probability swing) the player generated.
    pub dv_sum: f32,
    /// Mean ΔV per credited touch.
    pub dv_mean: f32,
}

/// Per-metric agreement between the two ground truths.
#[derive(Debug, Clone, Copy)]
pub struct MetricAgreement {
    pub metric: Metric,
    /// The metric's assumed good-direction (from its curve kind).
    pub good_dir: &'static str,
    /// Spearman(raw, rank) over players that have a rank.
    pub rho_rank: Option<f32>,
    /// Spearman(raw, ΔV_sum) over all players.
    pub rho_value: Option<f32>,
}

impl MetricAgreement {
    /// Do rank and ΔV point the *same* way for this metric? `None` when either
    /// correlation is missing or within the neutral band (no signal to compare).
    pub fn agrees(&self) -> Option<bool> {
        let (r, v) = (self.rho_rank?, self.rho_value?);
        if r.abs() < NEUTRAL || v.abs() < NEUTRAL {
            return None;
        }
        Some((r > 0.0) == (v > 0.0))
    }
}

/// Whole-corpus reconciliation between the rubric and the value model.
#[derive(Debug, Clone)]
pub struct Reconciliation {
    pub n: usize,
    pub n_ranked: usize,
    /// Do the two ground truths themselves agree? Spearman(rank, ΔV).
    pub rho_rank_dv_sum: Option<f32>,
    pub rho_rank_dv_mean: Option<f32>,
    /// The rubric's two report cards: vs rank (its calibration target) and vs ΔV
    /// (the independent objective signal).
    pub rho_composite_rank: Option<f32>,
    pub rho_composite_dv_sum: Option<f32>,
    pub rho_composite_dv_mean: Option<f32>,
    pub metrics: Vec<MetricAgreement>,
}

impl Reconciliation {
    /// Metrics whose rank- and value-correlations disagree in sign — the ones
    /// worth a human look (rank-cohort signals that do not pay off in impact).
    pub fn disagreements(&self) -> impl Iterator<Item = &MetricAgreement> {
        self.metrics.iter().filter(|m| m.agrees() == Some(false))
    }
}

/// Promote experimental candidate metrics that the value model vindicates.
///
/// Any metric flagged `experimental` whose ΔV correlation (`rho_value`) is at
/// least `min_rho` in magnitude **and** points the way its curve assumes
/// (higher-is-better ⇒ positive, lower-is-better ⇒ negative; band accepts either)
/// graduates into the composite: its `experimental` flag is cleared and its
/// within-role weight is set to `rho_value²` — the same evidence rule
/// [`crate::calibrate::fit_weights`] uses against rank, here driven by in-match
/// impact instead. Candidates with no ΔV signal (or the wrong sign) stay
/// experimental and out of the composite. Returns a new config; the input is
/// unchanged. `top_weights` are left as-is (the engine renormalizes within role).
pub fn promote_candidates(cfg: &ScoreConfig, recon: &Reconciliation, min_rho: f32) -> ScoreConfig {
    let mut cfg = cfg.clone();
    for ma in &recon.metrics {
        let Some(rv) = ma.rho_value else { continue };
        if rv.abs() < min_rho {
            continue;
        }
        let dir_ok = match ma.good_dir {
            "higher" => rv > 0.0,
            "lower" => rv < 0.0,
            _ => true, // band: either direction can carry signal
        };
        if !dir_ok {
            continue;
        }
        if let Some(spec) = cfg
            .metrics
            .iter_mut()
            .find(|s| s.metric == ma.metric && s.experimental)
        {
            spec.experimental = false;
            spec.weight = rv * rv;
        }
    }
    cfg
}

fn good_dir(curve: &Curve) -> &'static str {
    match curve {
        Curve::Higher { .. } => "higher",
        Curve::Lower { .. } => "lower",
        Curve::Band { .. } => "band",
    }
}

/// Correlate every rubric metric and the composite against both ground truths.
pub fn reconcile(cfg: &ScoreConfig, samples: &[CrossSample]) -> Reconciliation {
    let metrics = cfg
        .metrics
        .iter()
        .map(|spec| {
            // raw vs ΔV over all players; raw vs rank over the ranked subset.
            let (mut vx, mut vy) = (Vec::new(), Vec::new());
            let (mut rx, mut ry) = (Vec::new(), Vec::new());
            for s in samples {
                if let Some(Some(raw)) = s.raws.get(&spec.metric).copied() {
                    vx.push(raw);
                    vy.push(s.dv_sum);
                    if let Some(rk) = s.rank {
                        rx.push(raw);
                        ry.push(rk);
                    }
                }
            }
            MetricAgreement {
                metric: spec.metric,
                good_dir: good_dir(&spec.curve),
                rho_rank: spearman(&rx, &ry),
                rho_value: spearman(&vx, &vy),
            }
        })
        .collect();

    let comp: Vec<f32> = samples.iter().map(|s| s.composite).collect();
    let dv_sum: Vec<f32> = samples.iter().map(|s| s.dv_sum).collect();
    let dv_mean: Vec<f32> = samples.iter().map(|s| s.dv_mean).collect();

    let ranked: Vec<&CrossSample> = samples.iter().filter(|s| s.rank.is_some()).collect();
    let rk: Vec<f32> = ranked.iter().map(|s| s.rank.unwrap()).collect();
    let rk_comp: Vec<f32> = ranked.iter().map(|s| s.composite).collect();
    let rk_dv: Vec<f32> = ranked.iter().map(|s| s.dv_sum).collect();
    let rk_dvm: Vec<f32> = ranked.iter().map(|s| s.dv_mean).collect();

    Reconciliation {
        n: samples.len(),
        n_ranked: ranked.len(),
        rho_rank_dv_sum: spearman(&rk, &rk_dv),
        rho_rank_dv_mean: spearman(&rk, &rk_dvm),
        rho_composite_rank: spearman(&rk_comp, &rk),
        rho_composite_dv_sum: spearman(&comp, &dv_sum),
        rho_composite_dv_mean: spearman(&comp, &dv_mean),
        metrics,
    }
}
