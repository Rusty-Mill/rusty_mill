//! Unit tests for the calibration math: Spearman rank correlation (incl. ties),
//! percentile interpolation, and distribution-fit curves.

use replay_scoring::calibrate::{fit_curve, percentile, spearman};
use replay_scoring::config::Curve;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn spearman_signs_and_value() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    // Perfectly monotone increasing / decreasing.
    assert!(close(
        spearman(&xs, &[10.0, 20.0, 30.0, 40.0, 50.0]).unwrap(),
        1.0
    ));
    assert!(close(
        spearman(&xs, &[5.0, 4.0, 3.0, 2.0, 1.0]).unwrap(),
        -1.0
    ));
    // A single adjacent swap: ρ = 1 − 6·Σd²/(n(n²−1)) = 1 − 6·2/(4·15) = 0.8.
    assert!(close(
        spearman(&[1.0, 2.0, 3.0, 4.0], &[1.0, 3.0, 2.0, 4.0]).unwrap(),
        0.8
    ));
    // Zero variance (all equal) is undefined.
    assert!(spearman(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]).is_none());
    assert!(spearman(&[1.0], &[1.0]).is_none());
}

#[test]
fn spearman_handles_ties_with_average_ranks() {
    // y has a tie at the top two; still perfectly consistent with x ordering.
    let r = spearman(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 3.0]).unwrap();
    assert!(r > 0.9, "tie-aware ρ = {r}");
}

#[test]
fn percentile_interpolates() {
    let v = [0.0, 1.0, 2.0, 3.0, 4.0];
    assert!(close(percentile(&v, 0.0), 0.0));
    assert!(close(percentile(&v, 1.0), 4.0));
    assert!(close(percentile(&v, 0.5), 2.0));
    assert!(close(percentile(&v, 0.25), 1.0));
    assert!(close(percentile(&[7.0], 0.5), 7.0)); // single value
}

#[test]
fn fit_curve_anchors_at_percentiles_and_keeps_kind() {
    // Distribution 0..=100 → p10=10, p25=25, p75=75, p90=90.
    let raws: Vec<f32> = (0..=100).map(|i| i as f32).collect();

    match fit_curve(
        &raws,
        &Curve::Higher {
            zero: 0.0,
            full: 0.0,
        },
    ) {
        Curve::Higher { zero, full } => {
            assert!(close(zero, 10.0) && close(full, 90.0), "{zero},{full}");
        }
        other => panic!("kind changed: {other:?}"),
    }
    match fit_curve(
        &raws,
        &Curve::Lower {
            zero: 0.0,
            full: 0.0,
        },
    ) {
        Curve::Lower { zero, full } => {
            assert!(close(zero, 90.0) && close(full, 10.0), "{zero},{full}");
        }
        other => panic!("kind changed: {other:?}"),
    }
    match fit_curve(
        &raws,
        &Curve::Band {
            lo: 0.0,
            hi: 0.0,
            falloff: 0.0,
        },
    ) {
        Curve::Band { lo, hi, falloff } => {
            assert!(close(lo, 25.0) && close(hi, 75.0), "{lo},{hi}");
            assert!(close(falloff, 40.0), "falloff {falloff}"); // (90−10)/2
        }
        other => panic!("kind changed: {other:?}"),
    }

    // Too few points → unchanged template.
    let one = [5.0];
    assert!(matches!(
        fit_curve(&one, &Curve::Higher { zero: 1.0, full: 2.0 }),
        Curve::Higher { zero, full } if close(zero, 1.0) && close(full, 2.0)
    ));
}

#[test]
fn fit_weights_favors_rank_correlated_metrics() {
    use replay_scoring::calibrate::{fit_weights, refit_config};
    use replay_scoring::config::{Metric, ScoreConfig};
    use std::collections::BTreeMap;

    let base = ScoreConfig::default();
    let mut samples = Vec::new();
    let mut pooled: BTreeMap<Metric, Vec<f32>> = BTreeMap::new();
    for k in 0..50 {
        let tier = (k % 10) as f32;
        let boost = tier / 9.0; // perfectly rank-correlated signal
        let mut raws: BTreeMap<Metric, Option<f32>> = BTreeMap::new();
        raws.insert(Metric::BoostManagement, Some(boost));
        raws.insert(Metric::OvercommitRate, Some(0.2)); // constant noise
        pooled
            .entry(Metric::BoostManagement)
            .or_default()
            .push(boost);
        pooled.entry(Metric::OvercommitRate).or_default().push(0.2);
        samples.push((raws, tier));
    }
    let cfg = refit_config(&base, &pooled);
    let fitted = fit_weights(&cfg, &samples);
    let w = |m: Metric| {
        fitted
            .metrics
            .iter()
            .find(|s| s.metric == m)
            .unwrap()
            .weight
    };
    assert!(w(Metric::BoostManagement) > 0.1, "signal weight too low");
    assert!(
        w(Metric::OvercommitRate) < 1e-6,
        "noise metric should drop out"
    );
}

#[test]
fn join_ranks_recovers_mangled_names_via_team_anchor() {
    use replay_scoring::calibrate::join_ranks;
    use std::collections::BTreeMap;

    // Analyzer reports keep the true in-replay names; ballchasing mangled Bób→Bob.
    let players = vec![
        ("Alice".to_string(), Some(0)),
        ("Bób".to_string(), Some(0)),
        ("Carol".to_string(), Some(1)),
        ("Dave".to_string(), Some(1)),
    ];
    // The manifest keys ranks by the mangled names.
    let ranks: BTreeMap<String, i32> = [
        ("Alice".to_string(), 12),
        ("Bob".to_string(), 13),
        ("Carol".to_string(), 14),
        ("Dave".to_string(), 15),
    ]
    .into_iter()
    .collect();
    // Ground-truth stats carry the same mangled names *with team*.
    let gt = vec![
        ("Alice".to_string(), 0),
        ("Bob".to_string(), 0),
        ("Carol".to_string(), 1),
        ("Dave".to_string(), 1),
    ];

    // Without ground truth, the mangled "Bób" can't be resolved (exact-only).
    assert_eq!(
        join_ranks(&players, &ranks, None),
        vec![Some(12), None, Some(14), Some(15)]
    );
    // With ground truth, the unique team-0 residual pairs Bób→Bob→tier 13.
    assert_eq!(
        join_ranks(&players, &ranks, Some(&gt)),
        vec![Some(12), Some(13), Some(14), Some(15)]
    );
}

#[test]
fn fit_tiers_spreads_and_is_monotonic() {
    use replay_scoring::calibrate::fit_tiers;
    use replay_scoring::config::ScoreConfig;

    let base = ScoreConfig::default();
    let comps: Vec<f32> = (0..=100).map(|i| i as f32).collect();
    let tiers = fit_tiers(&base, &comps);
    assert_eq!(tiers.len(), base.tiers.len());
    assert_eq!(tiers[0].min_composite, 0.0);
    for w in tiers.windows(2) {
        assert!(
            w[1].min_composite >= w[0].min_composite,
            "tiers must ascend"
        );
    }
    assert!(tiers.last().unwrap().min_composite > 50.0);
}

#[test]
fn fit_weights_ridge_favors_predictive_metric() {
    use replay_scoring::calibrate::{fit_weights_ridge, refit_config};
    use replay_scoring::config::{Metric, ScoreConfig};
    use std::collections::BTreeMap;

    let base = ScoreConfig::default();
    let metrics: Vec<Metric> = base.metrics.iter().map(|s| s.metric).collect();
    // Every metric present each row (ridge needs complete rows). BoostManagement
    // is perfectly linear with tier; all others are uncorrelated noise.
    let noise = |k: usize, j: usize| (((k * 7 + j * 13) % 100) as f32) / 100.0;
    let mut samples = Vec::new();
    let mut pooled: BTreeMap<Metric, Vec<f32>> = BTreeMap::new();
    for k in 0..80 {
        let tier = (k % 8) as f32;
        let mut raws: BTreeMap<Metric, Option<f32>> = BTreeMap::new();
        for (j, &mt) in metrics.iter().enumerate() {
            let v = if mt == Metric::BoostManagement {
                tier / 7.0
            } else {
                noise(k, j)
            };
            raws.insert(mt, Some(v));
            pooled.entry(mt).or_default().push(v);
        }
        samples.push((raws, tier));
    }
    let cfg = refit_config(&base, &pooled);
    let fitted = fit_weights_ridge(&cfg, &samples, 1.0);
    let w = |m: Metric| {
        fitted
            .metrics
            .iter()
            .find(|s| s.metric == m)
            .unwrap()
            .weight
    };
    let maxw = fitted
        .metrics
        .iter()
        .map(|s| s.weight)
        .fold(0.0f32, f32::max);
    assert!(
        w(Metric::BoostManagement) > 0.0,
        "predictive metric must score"
    );
    assert!(
        (w(Metric::BoostManagement) - maxw).abs() < 1e-6,
        "the predictive metric should carry the most weight"
    );
}

#[test]
fn experimental_metrics_are_excluded_from_weight_fitting() {
    // Both weight fitters must leave experimental candidates at weight 0 (they're
    // not in the composite, so they mustn't steal coefficient mass from it).
    use replay_scoring::calibrate::{fit_weights, fit_weights_ridge};
    use replay_scoring::config::{Metric, ScoreConfig};
    use std::collections::BTreeMap;

    let cfg = ScoreConfig::default();
    let metrics: Vec<Metric> = cfg.metrics.iter().map(|s| s.metric).collect();
    // 40 players, every metric raw = tier (well-determined, rank-correlated).
    let samples: Vec<(BTreeMap<Metric, Option<f32>>, f32)> = (0..40)
        .map(|i| {
            let tier = i as f32;
            (metrics.iter().map(|&m| (m, Some(tier))).collect(), tier)
        })
        .collect();

    for fitted in [
        fit_weights(&cfg, &samples),
        fit_weights_ridge(&cfg, &samples, 5.0),
    ] {
        for spec in &fitted.metrics {
            if spec.experimental {
                assert_eq!(spec.weight, 0.0, "{:?} must stay weight 0", spec.metric);
            }
        }
        assert!(
            fitted.metrics.iter().any(|s| !s.experimental && s.weight > 0.0),
            "a real metric should still earn weight"
        );
    }
}
