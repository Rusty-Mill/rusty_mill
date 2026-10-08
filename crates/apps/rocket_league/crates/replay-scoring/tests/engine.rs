//! Pure-function tests for the scoring engine: calibration curves, sub-score /
//! composite aggregation (including missing-sub-score renormalization), licence
//! banding, nearest-centroid classification, and leak selection.

use replay_scoring::config::{Curve, Metric, ScoreConfig};
use replay_scoring::engine;
use std::collections::BTreeMap;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.5
}

#[test]
fn calibration_curves_map_to_0_100() {
    let higher = Curve::Higher {
        zero: 0.5,
        full: 0.9,
    };
    assert!(near(higher.normalize(0.5), 0.0));
    assert!(near(higher.normalize(0.9), 100.0));
    assert!(near(higher.normalize(0.7), 50.0));
    assert!(near(higher.normalize(0.2), 0.0)); // clamped
    assert!(near(higher.normalize(2.0), 100.0)); // clamped

    let lower = Curve::Lower {
        zero: 0.5,
        full: 0.1,
    };
    assert!(near(lower.normalize(0.5), 0.0));
    assert!(near(lower.normalize(0.1), 100.0));
    assert!(near(lower.normalize(0.3), 50.0));

    let band = Curve::Band {
        lo: 1200.0,
        hi: 2600.0,
        falloff: 1400.0,
    };
    assert!(near(band.normalize(2000.0), 100.0)); // inside band
    assert!(near(band.normalize(1200.0), 100.0)); // edge
    assert!(near(band.normalize(-200.0), 0.0)); // lo - falloff
    assert!(near(band.normalize(4000.0), 0.0)); // hi + falloff
    assert!(near(band.normalize(500.0), 50.0)); // halfway below band
}

#[test]
fn composite_renormalizes_over_missing_subscores() {
    let cfg = ScoreConfig::default(); // top weights [0.35, 0.30, 0.35]
                                      // 2nd-man uncomputable (None): composite over first+general only.
    let c = engine::composite(&cfg, [Some(60.0), None, Some(80.0)]);
    assert!(near(c, 70.0), "renormalized composite = {c}");
    // All present.
    let c2 = engine::composite(&cfg, [Some(100.0), Some(100.0), Some(100.0)]);
    assert!(near(c2, 100.0));
    // All missing -> 0.
    assert!(near(engine::composite(&cfg, [None, None, None]), 0.0));
}

#[test]
fn banding_picks_highest_cleared_tier() {
    let cfg = ScoreConfig::default();
    assert_eq!(engine::band(&cfg, 0.0), "Unranked");
    assert_eq!(engine::band(&cfg, 50.0), "Silver Licence");
    assert_eq!(engine::band(&cfg, 70.0), "Platinum Licence");
    assert_eq!(engine::band(&cfg, 99.0), "Pacifist Master");
}

#[test]
fn all_normalized_scores_within_bounds() {
    let cfg = ScoreConfig::default();
    // Extreme raws in both directions must still normalize into [0,100].
    for raw in [-5.0, 0.0, 0.1, 0.5, 0.9, 1.0, 50.0, 5000.0] {
        let mut raws = BTreeMap::new();
        for spec in &cfg.metrics {
            raws.insert(spec.metric, Some(raw));
        }
        for bd in engine::breakdowns(&cfg, &raws) {
            assert!(
                (0.0..=100.0).contains(&bd.normalized),
                "{} -> {}",
                bd.key,
                bd.normalized
            );
        }
    }
}

#[test]
fn leak_is_the_heaviest_weighted_deficit() {
    let cfg = ScoreConfig::default();
    // Everything perfect except a 1st-man metric (heaviest top weight) is awful.
    let mut raws = BTreeMap::new();
    raws.insert(Metric::OvercommitRate, Some(0.10)); // -> ~100 (good)
    raws.insert(Metric::GoalsideDiscipline1st, Some(0.50)); // -> ~0 (bad, weighted 0.35*0.5)
    raws.insert(Metric::SupportSpacing, Some(2000.0)); // band -> 100
    raws.insert(Metric::CentralSupportFraction, Some(0.75)); // -> 100
    raws.insert(Metric::DoubleCommitRate, Some(0.08)); // -> 100
    raws.insert(Metric::BoostManagement, Some(55.0)); // -> 100
    raws.insert(Metric::PossessionRetention, Some(0.60)); // -> 100
    raws.insert(Metric::BallChaseIndex, Some(0.20)); // -> 100
    raws.insert(Metric::GoalsideDisciplineTeam, Some(0.97)); // -> 100

    let bds = engine::breakdowns(&cfg, &raws);
    let (leak, chapter) = engine::pick_leak(&cfg, &bds);
    assert_eq!(leak, "goalside_discipline_1st");
    assert_eq!(chapter, "Core game states / defence");
}

#[test]
fn classifier_picks_nearest_archetype() {
    let cfg = ScoreConfig::default();
    // A clear Diver: high overcommit + ball-chase.
    let mut raws = BTreeMap::new();
    raws.insert(Metric::OvercommitRate, Some(0.70));
    raws.insert(Metric::CentralSupportFraction, Some(0.30));
    raws.insert(Metric::BallChaseIndex, Some(0.75));
    raws.insert(Metric::DoubleCommitRate, Some(0.40));
    raws.insert(Metric::BoostManagement, Some(45.0));
    assert_eq!(engine::classify(&cfg, &raws), "Diver");

    // A clear Calm Controller: high support, low overcommit/chase.
    let mut calm = BTreeMap::new();
    calm.insert(Metric::OvercommitRate, Some(0.15));
    calm.insert(Metric::CentralSupportFraction, Some(0.75));
    calm.insert(Metric::BallChaseIndex, Some(0.20));
    calm.insert(Metric::DoubleCommitRate, Some(0.10));
    calm.insert(Metric::BoostManagement, Some(55.0));
    assert_eq!(engine::classify(&cfg, &calm), "Calm Controller");
}
