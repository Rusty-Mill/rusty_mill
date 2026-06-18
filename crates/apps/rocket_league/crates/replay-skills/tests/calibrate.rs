//! Unit tests for the pure skill-calibration helpers (no corpus needed).

use std::collections::BTreeMap;

use replay_skills::calibrate::{fit_band, pctl, refit_skill_config, spearman};
use replay_skills::{Skill, SkillConfig};

#[test]
fn fit_band_returns_the_requested_percentiles() {
    // 0..=100 inclusive: p10 ≈ 10, p90 ≈ 90 (linear interpolation).
    let xs: Vec<f32> = (0..=100).map(|i| i as f32).collect();
    let (lo, hi) = fit_band(&xs, 0.10, 0.90);
    assert!((lo - 10.0).abs() < 1.0, "p10 ≈ 10, got {lo}");
    assert!((hi - 90.0).abs() < 1.0, "p90 ≈ 90, got {hi}");
    // pctl works on an unsorted sample too.
    assert!((pctl(&[90.0, 0.0, 50.0, 10.0], 0.0) - 0.0).abs() < 1e-6);
}

#[test]
fn spearman_is_positive_for_a_monotone_relation() {
    let metric = [200.0, 250.0, 600.0, 800.0, 1000.0];
    let tier = [1.0, 2.0, 3.0, 4.0, 5.0];
    let rho = spearman(&metric, &tier).expect("non-degenerate");
    assert!(rho > 0.9, "monotone increasing ⇒ ρ≈1, got {rho}");
    // Degenerate / too-short inputs are None.
    assert!(spearman(&[1.0], &[1.0]).is_none());
    assert!(spearman(&[1.0, 1.0, 1.0], &[2.0, 3.0, 4.0]).is_none()); // zero variance
}

#[test]
fn refit_moves_the_aerial_anchor_and_keeps_floors() {
    let base = SkillConfig::default();
    let mut metrics: BTreeMap<Skill, Vec<f32>> = BTreeMap::new();
    // A spread of observed aerial peak heights with p90 well above the default 900.
    metrics.insert(
        Skill::Aerial,
        vec![
            400.0, 500.0, 600.0, 700.0, 800.0, 900.0, 1100.0, 1300.0, 1500.0, 1700.0,
        ],
    );
    let fitted = refit_skill_config(&base, &metrics);

    // The ramp top moved toward the observed p90; the floor is untouched.
    assert!(
        fitted.high_aerial_height > base.aerial_min_height,
        "ramp top stays above the floor"
    );
    assert!(
        (fitted.high_aerial_height - pctl(&metrics[&Skill::Aerial], 0.90)).abs() < 1.0,
        "high_aerial_height fit to the corpus p90"
    );
    assert_eq!(
        fitted.aerial_min_height, base.aerial_min_height,
        "hard floor unchanged"
    );
    assert_eq!(fitted.version, format!("{}-fitted", base.version));

    // Serde round-trips so `replay-skills --config` can consume it.
    let json = serde_json::to_vec(&fitted).unwrap();
    let back: SkillConfig = serde_json::from_slice(&json).unwrap();
    assert_eq!(back, fitted);
}

#[test]
fn refit_ignores_a_too_small_aerial_sample() {
    let base = SkillConfig::default();
    let mut metrics: BTreeMap<Skill, Vec<f32>> = BTreeMap::new();
    metrics.insert(Skill::Aerial, vec![1600.0, 1700.0, 1800.0]); // < MIN_FIT_SAMPLES
    let fitted = refit_skill_config(&base, &metrics);
    assert_eq!(
        fitted.high_aerial_height, base.high_aerial_height,
        "a handful of replays shouldn't move a threshold"
    );
}
