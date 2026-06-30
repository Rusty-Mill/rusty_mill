//! Rank-relative scoring: an **additive** layer over the absolute rubric.
//!
//! The absolute composite ([`crate::engine`]) grades every player against one
//! fixed ruler, so it *is* a rank estimate — great for "what rank does this
//! person play like", weak for coaching ("what's my biggest leak *for my
//! level*"). This module answers the second question without disturbing the
//! first: it leaves the [`Report`] composite/licence/leak untouched and attaches
//! an optional [`RelativeReport`] that places each metric against the
//! distribution of players *in the same rank bracket*.
//!
//! The reference distributions are a corpus artifact ([`RankNorms`], written by
//! the calibrate harness as `rank_norms.json`): per rank bucket, the percentile
//! sketch of every raw metric (and of the composite). At scoring time we pick the
//! bracket — by default from the *lobby's* mean composite (the natural peer
//! group, and self-contained from a single replay), or from an explicit override
//! when the caller knows the rank — and report where the player falls within it.
//!
//! Pure: no I/O, no clock. The binary reads/writes the JSON; everything here is
//! deterministic functions over `(reports, norms, config)`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::calibrate::percentile;
use crate::config::{Curve, Metric, ScoreConfig};
use crate::report::Report;

/// Number of points in a percentile sketch: p0, p5, …, p100.
const GRID: usize = 21;

/// A compact percentile sketch of a value distribution — 21 ascending samples at
/// p0,p5,…,p100. Enough to map a value to its percentile (and back) by linear
/// interpolation, without storing the raw population.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pctl {
    /// `GRID` ascending values; `grid[i]` is the `(i/20)`-quantile.
    pub grid: Vec<f32>,
}

impl Pctl {
    /// Build a sketch from raw samples (any order). `None` if empty.
    pub fn from_samples(vals: &[f32]) -> Option<Pctl> {
        let mut s: Vec<f32> = vals.iter().copied().filter(|v| v.is_finite()).collect();
        if s.is_empty() {
            return None;
        }
        s.sort_by(f32::total_cmp);
        let grid = (0..GRID)
            .map(|i| percentile(&s, i as f32 / (GRID - 1) as f32))
            .collect();
        Some(Pctl { grid })
    }

    /// The raw value at quantile `p∈[0,1]` (interpolated). `value_at(0.5)` is the
    /// bracket median — the "expected for your rank" reference.
    pub fn value_at(&self, p: f32) -> f32 {
        percentile(&self.grid, p)
    }

    /// Where `raw` falls in this distribution, as a quantile `0..=1`
    /// (0 = at/below the floor, 1 = at/above the ceiling). Monotone in `raw`.
    pub fn percentile_of(&self, raw: f32) -> f32 {
        let g = &self.grid;
        if g.is_empty() {
            return 0.5;
        }
        if raw <= g[0] {
            return 0.0;
        }
        if raw >= g[g.len() - 1] {
            return 1.0;
        }
        // Find the first grid point >= raw and interpolate the quantile.
        for i in 1..g.len() {
            if raw <= g[i] {
                let span = g[i] - g[i - 1];
                let frac = if span.abs() < f32::EPSILON {
                    0.0
                } else {
                    (raw - g[i - 1]) / span
                };
                return ((i - 1) as f32 + frac) / (g.len() - 1) as f32;
            }
        }
        1.0
    }
}

/// Per-bucket reference distributions for one rank bracket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BucketNorm {
    /// Bucket name (`"bronze"`..`"grand-champion"`), as in the manifest.
    pub name: String,
    /// Mean RL rank-tier id of the bucket's players (orders the brackets).
    pub tier_mean: f32,
    /// Players that contributed to this bucket.
    pub n: usize,
    /// Distribution of the (default-config) composite within the bucket — used to
    /// place a player's overall standing and to estimate a lobby's bracket.
    pub composite: Pctl,
    /// Per-metric raw-value distribution within the bucket.
    pub metrics: BTreeMap<Metric, Pctl>,
}

/// The corpus rank-norms artifact: ordered brackets, each with its reference
/// distributions. Serialized to `rank_norms.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankNorms {
    /// Stamp (scoring-config version + corpus size) for staleness checks.
    pub version: String,
    /// Brackets, ascending by `tier_mean`.
    pub buckets: Vec<BucketNorm>,
}

/// One player's contribution to the norms (what the calibrate harness already
/// computes per scored player).
#[derive(Debug, Clone)]
pub struct NormSample {
    pub bucket: String,
    pub tier: f32,
    pub composite: f32,
    pub raws: BTreeMap<Metric, Option<f32>>,
}

impl RankNorms {
    /// Build the artifact from per-player samples, grouped by bucket. Buckets with
    /// fewer than `min_n` players are dropped (too few to form a stable sketch).
    pub fn build(version: impl Into<String>, samples: &[NormSample], min_n: usize) -> RankNorms {
        let mut by_bucket: BTreeMap<&str, Vec<&NormSample>> = BTreeMap::new();
        for s in samples {
            by_bucket.entry(s.bucket.as_str()).or_default().push(s);
        }
        let mut buckets: Vec<BucketNorm> = by_bucket
            .into_iter()
            .filter(|(_, v)| v.len() >= min_n)
            .map(|(name, v)| {
                let n = v.len();
                let tier_mean = v.iter().map(|s| s.tier).sum::<f32>() / n as f32;
                let composite =
                    Pctl::from_samples(&v.iter().map(|s| s.composite).collect::<Vec<_>>())
                        .unwrap_or(Pctl { grid: vec![0.0] });
                // Per-metric sketches over the players where the metric computed.
                let mut metrics: BTreeMap<Metric, Pctl> = BTreeMap::new();
                let mut keys: BTreeMap<Metric, Vec<f32>> = BTreeMap::new();
                for s in &v {
                    for (m, raw) in &s.raws {
                        if let Some(x) = raw {
                            keys.entry(*m).or_default().push(*x);
                        }
                    }
                }
                for (m, vals) in keys {
                    if let Some(p) = Pctl::from_samples(&vals) {
                        metrics.insert(m, p);
                    }
                }
                BucketNorm {
                    name: name.to_string(),
                    tier_mean,
                    n,
                    composite,
                    metrics,
                }
            })
            .collect();
        buckets.sort_by(|a, b| a.tier_mean.total_cmp(&b.tier_mean));
        RankNorms {
            version: version.into(),
            buckets,
        }
    }

    /// The bracket whose composite median is nearest `composite` — used to infer a
    /// lobby's level from its mean composite. `None` if there are no buckets.
    pub fn nearest_bracket(&self, composite: f32) -> Option<&BucketNorm> {
        self.buckets.iter().min_by(|a, b| {
            (a.composite.value_at(0.5) - composite)
                .abs()
                .total_cmp(&(b.composite.value_at(0.5) - composite).abs())
        })
    }

    /// Look a bracket up by name (case-insensitive), for an explicit override.
    pub fn bracket_named(&self, name: &str) -> Option<&BucketNorm> {
        self.buckets
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(name))
    }
}

/// One metric placed against the player's rank bracket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelativeMetric {
    pub key: String,
    pub raw: Option<f32>,
    /// Oriented standing in the bracket, 0–100: **higher is always better**
    /// (a "lower-is-better" metric is flipped; a band metric peaks at the bracket
    /// median). 50 ≈ typical for your rank.
    pub within_rank_pct: f32,
    /// The bracket's median raw for this metric — "expected for your level".
    pub bracket_median: f32,
}

/// The rank-relative companion to a [`Report`]: how the player compares to peers
/// of the same bracket, and where they most over/under-perform *for their level*.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelativeReport {
    /// Bracket the player was graded against (`"diamond"`, …).
    pub bracket: String,
    /// Mean rank-tier id of that bracket.
    pub bracket_tier_mean: f32,
    /// How the bracket was chosen: `"lobby"` (inferred) or `"explicit"`.
    pub basis: String,
    /// Percentile of the player's composite within the bracket, 0–100
    /// ("a top-30% Diamond").
    pub composite_pct: f32,
    /// Per-metric standing within the bracket.
    pub metrics: Vec<RelativeMetric>,
    /// Metric where the player most lags the bracket (weighted) — the coaching
    /// callout *for their level*. `"none"` if nothing is computable.
    pub rank_relative_leak: String,
    /// Metric where the player most leads the bracket — their relative strength.
    pub rank_relative_strength: String,
}

/// Orient a within-bracket quantile to "higher is better" per the metric's curve.
fn orient(curve: &Curve, q: f32) -> f32 {
    match curve {
        Curve::Higher { .. } => q,
        Curve::Lower { .. } => 1.0 - q,
        // Band: closeness to the bracket median is "good".
        Curve::Band { .. } => 1.0 - 2.0 * (q - 0.5).abs(),
    }
}

/// Place a single report against `bucket`, producing its [`RelativeReport`].
/// `basis` records how the bracket was chosen. Only non-experimental metrics with
/// a computable raw and a bracket sketch contribute to the leak/strength picks.
pub fn relativize(
    report: &Report,
    bucket: &BucketNorm,
    cfg: &ScoreConfig,
    basis: &str,
) -> RelativeReport {
    let mut metrics = Vec::new();
    // (impact-toward-leak, within_pct, key, weight) for the leak/strength picks.
    let mut weighted: Vec<(f32, f32, String)> = Vec::new();

    for spec in &cfg.metrics {
        let Some(bd) = report.metrics.iter().find(|b| b.key == spec.metric.key()) else {
            continue;
        };
        let Some(norm) = bucket.metrics.get(&spec.metric) else {
            continue;
        };
        let bracket_median = norm.value_at(0.5);
        let within = match bd.raw {
            Some(raw) => orient(&spec.curve, norm.percentile_of(raw)) * 100.0,
            None => continue,
        };
        metrics.push(RelativeMetric {
            key: spec.metric.key().to_string(),
            raw: bd.raw,
            within_rank_pct: within,
            bracket_median,
        });
        // Leak/strength weigh by composite weight so trivial metrics don't win.
        if !spec.experimental && bd.effective_weight > 0.0 {
            weighted.push((bd.effective_weight * (50.0 - within), within, bd.key.clone()));
        }
    }

    let rank_relative_leak = weighted
        .iter()
        .filter(|(impact, _, _)| *impact > 0.0)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, _, k)| k.clone())
        .unwrap_or_else(|| "none".to_string());
    let rank_relative_strength = weighted
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(_, _, k)| k.clone())
        .unwrap_or_else(|| "none".to_string());

    RelativeReport {
        bracket: bucket.name.clone(),
        bracket_tier_mean: bucket.tier_mean,
        basis: basis.to_string(),
        composite_pct: bucket.composite.percentile_of(report.composite) * 100.0,
        metrics,
        rank_relative_leak,
        rank_relative_strength,
    }
}

/// Attach a [`RelativeReport`] to every report in a lobby.
///
/// The bracket is chosen once for the whole lobby: from `override_bracket` if it
/// names a known bucket (`basis = "explicit"`), else from the lobby's **mean
/// composite** matched to the nearest bracket (`basis = "lobby"`). Reports whose
/// bracket can't be resolved (empty norms) are left with `relative = None`, so the
/// absolute rubric is never disturbed.
pub fn attach_relative(
    reports: &mut [Report],
    norms: &RankNorms,
    cfg: &ScoreConfig,
    override_bracket: Option<&str>,
) {
    if reports.is_empty() {
        return;
    }
    let (bucket, basis) = match override_bracket.and_then(|n| norms.bracket_named(n)) {
        Some(b) => (Some(b), "explicit"),
        None => {
            let mean =
                reports.iter().map(|r| r.composite).sum::<f32>() / reports.len() as f32;
            (norms.nearest_bracket(mean), "lobby")
        }
    };
    let Some(bucket) = bucket else { return };
    for r in reports.iter_mut() {
        r.relative = Some(relativize(r, bucket, cfg, basis));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pctl_roundtrips_monotone() {
        let p = Pctl::from_samples(&[0.0, 1.0, 2.0, 3.0, 4.0]).unwrap();
        assert_eq!(p.percentile_of(-1.0), 0.0);
        assert_eq!(p.percentile_of(5.0), 1.0);
        assert!((p.percentile_of(2.0) - 0.5).abs() < 0.05);
        // value_at(0.5) is the median.
        assert!((p.value_at(0.5) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn orient_flips_lower_is_better() {
        let hi = Curve::Higher { zero: 0.0, full: 1.0 };
        let lo = Curve::Lower { zero: 1.0, full: 0.0 };
        // A top-quantile value: good for higher, bad for lower.
        assert!((orient(&hi, 0.9) - 0.9).abs() < 1e-6);
        assert!((orient(&lo, 0.9) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn nearest_bracket_picks_closest_median() {
        let mk = |name: &str, tier: f32, comps: &[f32]| NormSample {
            bucket: name.to_string(),
            tier,
            composite: comps[0],
            raws: BTreeMap::new(),
        };
        // Two buckets with separated composite medians (~30 vs ~70).
        let mut samples = Vec::new();
        for c in [28.0, 30.0, 32.0] {
            samples.push(NormSample {
                composite: c,
                ..mk("silver", 6.0, &[c])
            });
        }
        for c in [68.0, 70.0, 72.0] {
            samples.push(NormSample {
                composite: c,
                ..mk("diamond", 14.0, &[c])
            });
        }
        let norms = RankNorms::build("test", &samples, 1);
        assert_eq!(norms.nearest_bracket(31.0).unwrap().name, "silver");
        assert_eq!(norms.nearest_bracket(69.0).unwrap().name, "diamond");
    }
}
