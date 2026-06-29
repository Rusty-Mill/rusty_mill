//! Corpus calibration: the routines that turn a labeled replay corpus into
//! evidence about whether the rubric tracks skill, and into fitted curves.
//!
//! Pure and testable. The companion binary (`src/bin/calibrate.rs`) does the I/O
//! — parse the corpus, score it, join each player's composite to their rank —
//! and leans on these functions for the statistics and curve fitting.
//!
//! The headline metric is **Spearman rank correlation** between a player's rank
//! tier and their composite (and each raw metric): the spec's "does the score
//! mean anything" check (§13). Curve fitting is population-relative — map each
//! metric's observed distribution across `0..=100` by percentiles — which is the
//! reference-distribution calibration §0 calls for.

use std::collections::BTreeMap;

use replay_analyzer::analyze::roster_match::{team_anchored_pairs, RosterSlot};

use crate::config::{Curve, Metric, Role, ScoreConfig, Tier};
use crate::engine;
use crate::report::Report;

/// Join each scored player to their rank tier, **team-anchored** so a ballchasing-
/// mangled rank key still resolves to the analyzer's true name.
///
/// `players` are the scored reports as `(true_name, team)`; `ranks` is the
/// manifest's `mangled_name → tier`; `gt` (optional) supplies each ranked
/// (mangled) name's team from the ballchasing stats fixture, which is what lets a
/// residual mangled name pair by team. Returns a tier per player, aligned to
/// `players`, `None` where unresolved. Without `gt` it degrades to exact-name
/// matching (the prior behavior) — so a missing fixture never regresses, it just
/// can't recover the ~5% of players whose ballchasing name was mangled.
pub fn join_ranks(
    players: &[(String, Option<i32>)],
    ranks: &BTreeMap<String, i32>,
    gt: Option<&[(String, i32)]>,
) -> Vec<Option<i32>> {
    // Exact-name match: the common case, and the only path when `gt` is absent.
    let mut tiers: Vec<Option<i32>> = players
        .iter()
        .map(|(name, _)| ranks.get(name).copied())
        .collect();

    // Team-anchor the residuals against the ranked roster (names carry team here).
    if let Some(gt) = gt {
        let left: Vec<RosterSlot> = players
            .iter()
            .map(|(name, team)| RosterSlot::new(*team, name))
            .collect();
        let right: Vec<RosterSlot> = gt
            .iter()
            .map(|(name, team)| RosterSlot::new(Some(*team), name))
            .collect();
        for (li, ri) in team_anchored_pairs(&left, &right) {
            if tiers[li].is_none() {
                tiers[li] = ranks.get(gt[ri].0.as_str()).copied();
            }
        }
    }
    tiers
}

/// Average-rank transform (ties share the mean of their rank span), 1-based.
fn rankdata(v: &[f32]) -> Vec<f32> {
    let n = v.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&i, &j| v[i].total_cmp(&v[j]));
    let mut ranks = vec![0.0f32; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && v[idx[j + 1]] == v[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f32 / 2.0 + 1.0;
        for &k in &idx[i..=j] {
            ranks[k] = avg;
        }
        i = j + 1;
    }
    ranks
}

/// Pearson correlation, or `None` if either input has zero variance.
fn pearson(a: &[f32], b: &[f32]) -> Option<f32> {
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let (mut cov, mut va, mut vb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        let (dx, dy) = (x - ma, y - mb);
        cov += dx * dy;
        va += dx * dx;
        vb += dy * dy;
    }
    (va > 0.0 && vb > 0.0).then(|| cov / (va.sqrt() * vb.sqrt()))
}

/// Spearman rank correlation of paired samples, `None` if too few or degenerate.
pub fn spearman(xs: &[f32], ys: &[f32]) -> Option<f32> {
    if xs.len() != ys.len() || xs.len() < 2 {
        return None;
    }
    pearson(&rankdata(xs), &rankdata(ys))
}

/// Linear-interpolated percentile `p∈[0,1]` of an ascending-sorted slice.
pub fn percentile(sorted: &[f32], p: f32) -> f32 {
    match sorted {
        [] => 0.0,
        [only] => *only,
        _ => {
            let rank = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f32;
            let lo = rank.floor() as usize;
            let hi = rank.ceil() as usize;
            sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f32)
        }
    }
}

/// Refit a curve's parameters to an observed raw distribution, preserving the
/// curve *kind* (and its good-direction): `Higher`/`Lower` anchor at the 10th/90th
/// percentiles, `Band` keeps the central 25–75% at full and falls off over the
/// 10–90% spread.
pub fn fit_curve(raws: &[f32], template: &Curve) -> Curve {
    let mut s: Vec<f32> = raws.iter().copied().filter(|v| v.is_finite()).collect();
    s.sort_by(f32::total_cmp);
    if s.len() < 2 {
        return *template;
    }
    let p = |q| percentile(&s, q);
    match template {
        Curve::Higher { .. } => Curve::Higher {
            zero: p(0.10),
            full: p(0.90),
        },
        Curve::Lower { .. } => Curve::Lower {
            zero: p(0.90),
            full: p(0.10),
        },
        Curve::Band { .. } => Curve::Band {
            lo: p(0.25),
            hi: p(0.75),
            falloff: ((p(0.90) - p(0.10)).abs() / 2.0).max(1e-3),
        },
    }
}

/// Reconstruct the per-metric raw map for a report, keyed by [`Metric`] (the
/// report stores them string-keyed).
pub fn raws_from_report(cfg: &ScoreConfig, r: &Report) -> BTreeMap<Metric, Option<f32>> {
    cfg.metrics
        .iter()
        .map(|s| {
            let raw = r
                .metrics
                .iter()
                .find(|b| b.key == s.metric.key())
                .and_then(|b| b.raw);
            (s.metric, raw)
        })
        .collect()
}

/// Composite (and the three sub-scores) for a raw map under `cfg` — lets a
/// recalibrated config be re-scored from cached raws without re-parsing.
pub fn composite_with(
    cfg: &ScoreConfig,
    raws: &BTreeMap<Metric, Option<f32>>,
) -> (f32, [Option<f32>; 3]) {
    let bds = engine::breakdowns(cfg, raws);
    let subs = engine::sub_scores(cfg, &bds);
    (engine::composite(cfg, subs), subs)
}

/// Clone `base`, refitting every metric's curve from the pooled raw values.
pub fn refit_config(
    base: &ScoreConfig,
    raws_by_metric: &BTreeMap<Metric, Vec<f32>>,
) -> ScoreConfig {
    let mut cfg = base.clone();
    for spec in &mut cfg.metrics {
        if let Some(vals) = raws_by_metric.get(&spec.metric) {
            spec.curve = fit_curve(vals, &spec.curve);
        }
    }
    cfg
}

/// Spearman of each metric's *normalized* (curve-applied) score against rank,
/// over the players where it is computable. Curves should already be fitted, so
/// the sign reflects whether the metric's good-direction matches reality.
pub fn metric_rank_rho(
    cfg: &ScoreConfig,
    samples: &[(BTreeMap<Metric, Option<f32>>, f32)],
) -> BTreeMap<Metric, f32> {
    let mut out = BTreeMap::new();
    for spec in &cfg.metrics {
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        for (raws, tier) in samples {
            if let Some(v) = raws.get(&spec.metric).and_then(|o| *o) {
                xs.push(spec.curve.normalize(v));
                ys.push(*tier);
            }
        }
        out.insert(spec.metric, spearman(&xs, &ys).unwrap_or(0.0));
    }
    out
}

/// Evidence-based reweighting: set each within-role weight to `max(0, ρ)²` (ρ =
/// the metric's normalized-vs-rank Spearman) so wrong-direction or noise metrics
/// drop out and strong ones dominate, and set role top-weights to each role's
/// weight mass. Only relative weights matter (the engine renormalizes).
pub fn fit_weights(
    cfg: &ScoreConfig,
    samples: &[(BTreeMap<Metric, Option<f32>>, f32)],
) -> ScoreConfig {
    let rho = metric_rank_rho(cfg, samples);
    let mut cfg = cfg.clone();
    for spec in &mut cfg.metrics {
        // Experimental candidates don't feed the composite — keep them out of the
        // weighting too (weight 0) until promoted.
        if spec.experimental {
            spec.weight = 0.0;
            continue;
        }
        let r = rho.get(&spec.metric).copied().unwrap_or(0.0).max(0.0);
        spec.weight = r * r;
    }
    let mass = |role: Role| {
        cfg.metrics
            .iter()
            .filter(|s| s.role == role)
            .map(|s| s.weight)
            .sum::<f32>()
    };
    cfg.top_weights = [mass(Role::First), mass(Role::Second), mass(Role::General)];
    cfg
}

/// Solve `A x = b` for a small dense system (Gaussian elimination, partial
/// pivot). `None` if singular.
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let mut piv = col;
        for r in (col + 1)..n {
            if a[r][col].abs() > a[piv][col].abs() {
                piv = r;
            }
        }
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        let pivot = a[col].clone();
        let bp = b[col];
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = a[r][col] / pivot[col];
            for (arc, &pc) in a[r].iter_mut().zip(&pivot).skip(col) {
                *arc -= f * pc;
            }
            b[r] -= f * bp;
        }
    }
    Some((0..n).map(|i| b[i] / a[i][i]).collect())
}

/// Ridge-regression reweighting: fit `rank ~ Σ βⱼ·normalizedⱼ` (on the players
/// where every metric is computable, features standardized, `λ` shrinkage), and
/// use `max(0, βⱼ/stdⱼ)` as each metric's weight. Unlike `fit_weights`, this
/// shares weight correctly across *collinear* signals (e.g. supersonic ↔ aerial)
/// instead of double-counting them. Falls back to `fit_weights` if under-determined.
pub fn fit_weights_ridge(
    cfg: &ScoreConfig,
    samples: &[(BTreeMap<Metric, Option<f32>>, f32)],
    lambda: f64,
) -> ScoreConfig {
    // Fit only over the metrics that actually feed the composite. Experimental
    // candidates are excluded (they'd otherwise steal coefficient mass and, via
    // the all-computable row filter, shrink the usable sample).
    let active: Vec<usize> = (0..cfg.metrics.len())
        .filter(|&i| !cfg.metrics[i].experimental)
        .collect();
    let m = active.len();
    let (mut rows, mut ys) = (Vec::<Vec<f64>>::new(), Vec::<f64>::new());
    for (raws, tier) in samples {
        let vals: Option<Vec<f64>> = active
            .iter()
            .map(|&i| {
                let spec = &cfg.metrics[i];
                raws.get(&spec.metric)
                    .and_then(|o| *o)
                    .map(|v| spec.curve.normalize(v) as f64)
            })
            .collect();
        if let Some(v) = vals {
            rows.push(v);
            ys.push(*tier as f64);
        }
    }
    let n = rows.len();
    if n < m + 2 {
        return fit_weights(cfg, samples);
    }
    // Standardize columns; center y.
    let mut mean = vec![0.0; m];
    for r in &rows {
        for j in 0..m {
            mean[j] += r[j];
        }
    }
    for me in &mut mean {
        *me /= n as f64;
    }
    let mut std = vec![0.0; m];
    for r in &rows {
        for j in 0..m {
            std[j] += (r[j] - mean[j]).powi(2);
        }
    }
    for s in &mut std {
        *s = (*s / n as f64).sqrt().max(1e-9);
    }
    for r in &mut rows {
        for j in 0..m {
            r[j] = (r[j] - mean[j]) / std[j];
        }
    }
    let ymean = ys.iter().sum::<f64>() / n as f64;
    for y in &mut ys {
        *y -= ymean;
    }
    // Normal equations (XᵀX + λI) β = Xᵀy.
    let mut xtx = vec![vec![0.0; m]; m];
    let mut xty = vec![0.0; m];
    for (r, &y) in rows.iter().zip(&ys) {
        for i in 0..m {
            xty[i] += r[i] * y;
            for j in 0..m {
                xtx[i][j] += r[i] * r[j];
            }
        }
    }
    for (i, row) in xtx.iter_mut().enumerate() {
        row[i] += lambda;
    }
    let Some(beta) = solve(xtx, xty) else {
        return fit_weights(cfg, samples);
    };
    let mut cfg = cfg.clone();
    for spec in &mut cfg.metrics {
        spec.weight = 0.0; // experimental metrics stay 0; active ones set below
    }
    for (k, &i) in active.iter().enumerate() {
        // β is per standardized unit; /std gives the coefficient on the normalized
        // score. Keep only positive (rank-helping) contributions.
        cfg.metrics[i].weight = (beta[k] / std[k]).max(0.0) as f32;
    }
    let mass = |role: Role| {
        cfg.metrics
            .iter()
            .filter(|s| s.role == role)
            .map(|s| s.weight)
            .sum::<f32>()
    };
    cfg.top_weights = [mass(Role::First), mass(Role::Second), mass(Role::General)];
    cfg
}

/// Refit licence-tier thresholds to the fitted composite distribution, keeping
/// the tier names. The first tier stays at 0 (Unranked floor); the rest spread
/// across the 8th–92nd percentiles so the population populates every licence.
pub fn fit_tiers(cfg: &ScoreConfig, composites: &[f32]) -> Vec<Tier> {
    let mut s: Vec<f32> = composites
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    s.sort_by(f32::total_cmp);
    if s.len() < 2 || cfg.tiers.len() < 2 {
        return cfg.tiers.clone();
    }
    let upper = (cfg.tiers.len() - 1) as f32;
    cfg.tiers
        .iter()
        .enumerate()
        .map(|(i, t)| Tier {
            name: t.name.clone(),
            min_composite: if i == 0 {
                0.0
            } else {
                percentile(&s, 0.08 + (i as f32 - 1.0) / upper * 0.84)
            },
        })
        .collect()
}
