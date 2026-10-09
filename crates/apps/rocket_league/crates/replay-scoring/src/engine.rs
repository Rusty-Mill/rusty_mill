//! The scoring engine: calibrate raw metrics, aggregate into sub-scores and a
//! composite, then derive licence band, player type, and the highest-impact
//! leak. Pure functions over `(raws, ScoreConfig)`.

use std::collections::BTreeMap;

use crate::config::{Metric, Role, ScoreConfig};
use crate::report::MetricBreakdown;

/// Calibrate every configured metric to 0–100 with its effective weight.
pub fn breakdowns(cfg: &ScoreConfig, raws: &BTreeMap<Metric, Option<f32>>) -> Vec<MetricBreakdown> {
    cfg.metrics
        .iter()
        .map(|spec| {
            let raw = raws.get(&spec.metric).copied().flatten();
            let normalized = raw.map(|r| spec.curve.normalize(r)).unwrap_or(0.0);
            MetricBreakdown {
                key: spec.metric.key().to_string(),
                role: spec.role,
                raw,
                normalized,
                // Candidates never contribute to the composite.
                effective_weight: if spec.experimental {
                    0.0
                } else {
                    top_weight(cfg, spec.role) * spec.weight
                },
                experimental: spec.experimental,
            }
        })
        .collect()
}

fn top_weight(cfg: &ScoreConfig, role: Role) -> f32 {
    match role {
        Role::First => cfg.top_weights[0],
        Role::Second => cfg.top_weights[1],
        Role::General => cfg.top_weights[2],
    }
}

/// Weighted mean (by within-role weight) of the computable metrics in a role,
/// or `None` if the role has no computable metric (e.g. no teammate ⇒ no
/// support metrics).
fn sub_score(cfg: &ScoreConfig, bds: &[MetricBreakdown], role: Role) -> Option<f32> {
    let (mut num, mut den) = (0.0f32, 0.0f32);
    for (spec, bd) in cfg.metrics.iter().zip(bds) {
        if spec.role == role && !spec.experimental && bd.raw.is_some() {
            num += bd.normalized * spec.weight;
            den += spec.weight;
        }
    }
    (den > 0.0).then(|| num / den)
}

/// `(first, second, general)` sub-scores; `None` where uncomputable.
pub fn sub_scores(cfg: &ScoreConfig, bds: &[MetricBreakdown]) -> [Option<f32>; 3] {
    [
        sub_score(cfg, bds, Role::First),
        sub_score(cfg, bds, Role::Second),
        sub_score(cfg, bds, Role::General),
    ]
}

/// Top-weighted composite, renormalized over the computable sub-scores so a
/// missing sub-score (no data) neither counts as zero nor is fabricated.
pub fn composite(cfg: &ScoreConfig, subs: [Option<f32>; 3]) -> f32 {
    let (mut num, mut den) = (0.0f32, 0.0f32);
    for (w, s) in cfg.top_weights.iter().zip(subs) {
        if let Some(s) = s {
            num += w * s;
            den += w;
        }
    }
    if den > 0.0 {
        num / den
    } else {
        0.0
    }
}
/// Highest licence band whose threshold the composite clears.
pub fn band(cfg: &ScoreConfig, composite: f32) -> String {
    cfg.tiers
        .iter()
        .filter(|t| composite >= t.min_composite)
        .max_by(|a, b| a.min_composite.total_cmp(&b.min_composite))
        .map(|t| t.name.clone())
        .unwrap_or_else(|| "Unranked".to_string())
}

/// Nearest-centroid player-type over normalized behavioral features
/// `[overcommit, central_support, ball_chase, double_commit, boost_hoard]`.
pub fn classify(cfg: &ScoreConfig, raws: &BTreeMap<Metric, Option<f32>>) -> String {
    let feat = |m: Metric, scale: f32| {
        raws.get(&m)
            .copied()
            .flatten()
            .map(|v| (v / scale).clamp(0.0, 1.0))
            .unwrap_or(0.5)
    };
    let v = [
        feat(Metric::OvercommitRate, 1.0),
        feat(Metric::CentralSupportFraction, 1.0),
        feat(Metric::BallChaseIndex, 1.0),
        feat(Metric::DoubleCommitRate, 1.0),
        feat(Metric::BoostManagement, 100.0),
    ];
    cfg.centroids
        .iter()
        .min_by(|a, b| l2(&a.vector, &v).total_cmp(&l2(&b.vector, &v)))
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "Unclassified".to_string())
}

fn l2(a: &[f32; 5], b: &[f32; 5]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f32>()
        .sqrt()
}

/// The computable metric with the greatest `weight · (100 − normalized)` — both
/// heavily weighted and far from target — plus its book chapter.
pub fn pick_leak(cfg: &ScoreConfig, bds: &[MetricBreakdown]) -> (String, String) {
    cfg.metrics
        .iter()
        .zip(bds)
        .filter(|(spec, bd)| !spec.experimental && bd.raw.is_some())
        .map(|(spec, bd)| {
            let impact = bd.effective_weight * (100.0 - bd.normalized);
            (impact, bd.key.clone(), spec.chapter.clone())
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, key, chapter)| (key, chapter))
        .unwrap_or_else(|| ("none".to_string(), "Fundamentals".to_string()))
}
