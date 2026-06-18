//! Unit tests for the pure skill-calibration helpers (no corpus needed).

use std::collections::BTreeMap;

use replay_skills::calibrate::{fit_aerial_floor, fit_band, pctl, refit_skill_config, spearman};
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
fn refit_without_candidates_moves_only_the_top_and_keeps_the_floor() {
    // No candidate-mode data → fall back to fitting only the ramp top from gated
    // peak heights, leaving the hard floor at its default (the B2 behavior).
    let base = SkillConfig::default();
    let mut metrics: BTreeMap<Skill, Vec<f32>> = BTreeMap::new();
    // A spread of observed aerial peak heights with p90 well above the default 900.
    metrics.insert(
        Skill::Aerial,
        vec![
            400.0, 500.0, 600.0, 700.0, 800.0, 900.0, 1100.0, 1300.0, 1500.0, 1700.0,
        ],
    );
    let fitted = refit_skill_config(&base, &metrics, &BTreeMap::new());

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
        "hard floor unchanged without candidate-mode data"
    );
    assert_eq!(fitted.version, format!("{}-fitted", base.version));

    // Serde round-trips so `replay-skills --config` can consume it.
    let json = serde_json::to_vec(&fitted).unwrap();
    let back: SkillConfig = serde_json::from_slice(&json).unwrap();
    assert_eq!(back, fitted);
}

#[test]
fn refit_with_candidates_fits_the_aerial_floor_and_top() {
    // Candidate-mode: car height at *every* touch. A dense ground-touch cluster
    // (~17uu) and an aerial tail (600..1680), with an empty valley between. The
    // floor should land in that valley (midpoint of 17 and 600 = ~308), robust to
    // how many of the touches were aerials.
    let base = SkillConfig::default();
    let mut candidates = vec![17.0_f32; 90]; // ground touches
    candidates.extend((0..10).map(|i| 600.0 + 120.0 * i as f32)); // aerial tail
    let mut cand_by_skill: BTreeMap<Skill, Vec<f32>> = BTreeMap::new();
    cand_by_skill.insert(Skill::Aerial, candidates.clone());

    let fitted = refit_skill_config(&base, &BTreeMap::new(), &cand_by_skill);

    let valley = fit_aerial_floor(&candidates).expect("a clear ground/aerial valley");
    assert!(
        (valley - 308.5).abs() < 1.0,
        "valley midpoint, got {valley}"
    );
    assert!(
        (fitted.aerial_min_height - valley).abs() < 1e-3,
        "floor fit to the valley, got {}",
        fitted.aerial_min_height
    );
    assert!(
        fitted.high_aerial_height > fitted.aerial_min_height + 99.0,
        "top stays clear of the fitted floor"
    );

    // A unimodal sample with no gap in the band keeps the default floor.
    let flat: Vec<f32> = (0..50).map(|i| 700.0 + i as f32).collect();
    let mut flat_map: BTreeMap<Skill, Vec<f32>> = BTreeMap::new();
    flat_map.insert(Skill::Aerial, flat);
    let flat_fit = refit_skill_config(&base, &BTreeMap::new(), &flat_map);
    assert_eq!(
        flat_fit.aerial_min_height, base.aerial_min_height,
        "no valley in band ⇒ floor unchanged"
    );
}

#[test]
fn refit_ignores_a_too_small_aerial_sample() {
    let base = SkillConfig::default();
    let mut metrics: BTreeMap<Skill, Vec<f32>> = BTreeMap::new();
    metrics.insert(Skill::Aerial, vec![1600.0, 1700.0, 1800.0]); // < MIN_FIT_SAMPLES
    let mut cand: BTreeMap<Skill, Vec<f32>> = BTreeMap::new();
    cand.insert(Skill::Aerial, vec![17.0, 18.0, 900.0]); // also < MIN_FIT_SAMPLES
    let fitted = refit_skill_config(&base, &metrics, &cand);
    assert_eq!(
        fitted.high_aerial_height, base.high_aerial_height,
        "a handful of replays shouldn't move a threshold"
    );
    assert_eq!(
        fitted.aerial_min_height, base.aerial_min_height,
        "nor the floor"
    );
}
