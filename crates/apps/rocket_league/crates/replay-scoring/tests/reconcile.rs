//! Tests for the rubric↔value-model reconciliation math.

use std::collections::BTreeMap;

use replay_scoring::config::{Curve, Metric, ScoreConfig};
use replay_scoring::reconcile::{promote_candidates, reconcile, CrossSample, MetricAgreement};

fn sample(metric: Metric, raw: f32, composite: f32, rank: Option<f32>, dv: f32) -> CrossSample {
    let mut raws = BTreeMap::new();
    raws.insert(metric, Some(raw));
    CrossSample {
        raws,
        composite,
        rank,
        dv_sum: dv,
        dv_mean: dv,
    }
}

#[test]
fn agreement_sign_logic() {
    let mk = |r, v| MetricAgreement {
        metric: Metric::BoostManagement,
        good_dir: "higher",
        rho_rank: r,
        rho_value: v,
    };
    assert_eq!(mk(Some(0.5), Some(0.4)).agrees(), Some(true)); // both positive
    assert_eq!(mk(Some(-0.5), Some(-0.4)).agrees(), Some(true)); // both negative
    assert_eq!(mk(Some(0.5), Some(-0.4)).agrees(), Some(false)); // opposite signs
    assert_eq!(mk(Some(0.5), Some(0.02)).agrees(), None); // value within neutral band
    assert_eq!(mk(None, Some(0.4)).agrees(), None); // a correlation is missing
}

#[test]
fn flags_sign_disagreement() {
    // rank rises while ΔV falls, so a raw that tracks rank tracks ΔV oppositely.
    let cfg = ScoreConfig::default();
    let n = 6;
    let samples: Vec<CrossSample> = (0..n)
        .map(|i| {
            let raw = i as f32;
            sample(
                Metric::BoostManagement,
                raw,
                raw,
                Some(i as f32),
                (n - 1 - i) as f32,
            )
        })
        .collect();

    let rec = reconcile(&cfg, &samples);
    assert_eq!(rec.n, 6);
    assert_eq!(rec.n_ranked, 6);

    let bm = rec
        .metrics
        .iter()
        .find(|m| m.metric == Metric::BoostManagement)
        .unwrap();
    assert!(bm.rho_rank.unwrap() > 0.9);
    assert!(bm.rho_value.unwrap() < -0.9);
    assert_eq!(bm.agrees(), Some(false));
    assert!(rec
        .disagreements()
        .any(|m| m.metric == Metric::BoostManagement));

    // composite == raw, so it tracks rank up and ΔV down.
    assert!(rec.rho_composite_rank.unwrap() > 0.9);
    assert!(rec.rho_composite_dv_sum.unwrap() < -0.9);
}

#[test]
fn promotes_candidates_that_track_value() {
    // Three metrics forced experimental across players, vs ΔV = i:
    //  facing_ball_share  raw=+i, curve Higher  -> ρ_value ≈ +1, sign OK   -> promote
    //  reverse_driving    raw=+i, curve Lower   -> ρ_value ≈ +1, wrong way -> keep
    //  support_spacing    raw=-i, curve Band    -> ρ_value ≈ -1, band OK   -> promote
    // Force a controlled candidate scenario (curves + experimental) so the test
    // is independent of which metrics the shipped default config graduates.
    let mut cfg = ScoreConfig::default();
    for s in &mut cfg.metrics {
        match s.metric {
            Metric::FacingBallShare => {
                s.experimental = true;
                s.curve = Curve::Higher { zero: 0.0, full: 1.0 };
            }
            Metric::ReverseDriving => {
                s.experimental = true;
                s.curve = Curve::Lower { zero: 1.0, full: 0.0 };
            }
            Metric::SupportSpacing => {
                s.experimental = true;
                s.curve = Curve::Band { lo: 0.3, hi: 0.6, falloff: 0.3 };
            }
            _ => {}
        }
    }
    let samples: Vec<CrossSample> = (0..8)
        .map(|i| {
            let raw = i as f32;
            let mut raws = BTreeMap::new();
            raws.insert(Metric::FacingBallShare, Some(raw));
            raws.insert(Metric::ReverseDriving, Some(raw));
            raws.insert(Metric::SupportSpacing, Some(-raw));
            CrossSample {
                raws,
                composite: raw,
                rank: None,
                dv_sum: raw,
                dv_mean: raw,
            }
        })
        .collect();

    let rec = reconcile(&cfg, &samples);
    let promoted = promote_candidates(&cfg, &rec, 0.5);
    let spec = |c: &ScoreConfig, m: Metric| c.metrics.iter().find(|s| s.metric == m).unwrap().clone();

    // Candidates were experimental to begin with.
    assert!(spec(&cfg, Metric::FacingBallShare).experimental);

    // Correctly-signed candidates graduate with a positive weight…
    let f = spec(&promoted, Metric::FacingBallShare);
    assert!(!f.experimental && f.weight > 0.5);
    let l = spec(&promoted, Metric::SupportSpacing);
    assert!(!l.experimental && l.weight > 0.5);

    // …the wrong-signed one (higher reverse ⇒ higher ΔV contradicts "lower is
    // better") stays a candidate, out of the composite.
    assert!(spec(&promoted, Metric::ReverseDriving).experimental);

    // The input config is untouched.
    assert!(spec(&cfg, Metric::FacingBallShare).experimental);
}

#[test]
fn agreement_with_some_unranked_players() {
    // rank and ΔV aligned; odd-index players carry no rank but still count for ΔV.
    let cfg = ScoreConfig::default();
    let samples: Vec<CrossSample> = (0..6)
        .map(|i| {
            let raw = i as f32;
            let rank = (i % 2 == 0).then_some(i as f32);
            sample(Metric::GoalsideDiscipline1st, raw, raw, rank, raw)
        })
        .collect();

    let rec = reconcile(&cfg, &samples);
    assert_eq!(rec.n, 6);
    assert_eq!(rec.n_ranked, 3);

    let g = rec
        .metrics
        .iter()
        .find(|m| m.metric == Metric::GoalsideDiscipline1st)
        .unwrap();
    assert_eq!(g.agrees(), Some(true));
    assert!(g.rho_value.unwrap() > 0.9);
    assert!(rec.rho_composite_dv_sum.unwrap() > 0.9);
    assert_eq!(rec.disagreements().count(), 0);
}
