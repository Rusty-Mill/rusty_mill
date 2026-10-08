//! Gradient-boosted value model: the property that motivates the spike — it fits a
//! *non-monotonic* pattern a linear model cannot — plus determinism and the
//! empty-dataset contract.

use replay_value::config::TrainConfig;
use replay_value::dataset::{Dataset, Row};
use replay_value::features::N_FEATURES;
use replay_value::{GbtConfig, GbtModel, ValueModel};

/// A non-monotonic band over feature 0: `y = |x0| > 0.5` (high at both extremes,
/// low in the middle). A single linear threshold can't separate it, but shallow
/// trees split out the two tails — and unlike balanced XOR the greedy first split
/// has real gain, so it's actually learnable by a CART-style tree.
fn band_dataset() -> Dataset {
    let n = 400;
    let rows = (0..n)
        .map(|i| {
            let x0 = (i as f32 / (n - 1) as f32) * 2.0 - 1.0; // -1..1
            let mut x = [0.0f32; N_FEATURES];
            x[0] = x0;
            Row {
                t: 0.0,
                team: 0,
                x,
                y: if x0.abs() > 0.5 { 1.0 } else { 0.0 },
            }
        })
        .collect();
    Dataset {
        rows,
        horizon_s: 10.0,
    }
}

/// Mann–Whitney AUC of a prediction fn over the dataset.
fn auc(predict: impl Fn(&[f32; N_FEATURES]) -> f32, ds: &Dataset) -> f32 {
    let score = |want_pos: bool| -> Vec<f32> {
        ds.rows
            .iter()
            .filter(|r| (r.y > 0.5) == want_pos)
            .map(|r| predict(&r.x))
            .collect()
    };
    let (pos, neg) = (score(true), score(false));
    if pos.is_empty() || neg.is_empty() {
        return 0.5;
    }
    let mut concordant = 0.0;
    for &p in &pos {
        for &n in &neg {
            concordant += if p > n {
                1.0
            } else if p == n {
                0.5
            } else {
                0.0
            };
        }
    }
    concordant / (pos.len() * neg.len()) as f32
}

#[test]
fn gbt_learns_nonmonotonic_band_logistic_cannot() {
    let ds = band_dataset();
    let gbt = GbtModel::train(&ds, &GbtConfig::default());
    let logistic = ValueModel::train(&ds, &TrainConfig::default());

    let gbt_auc = auc(|x| gbt.predict(x), &ds);
    let logistic_auc = auc(|x| logistic.predict(x), &ds);
    assert!(
        gbt_auc > 0.9,
        "GBT should fit the non-monotonic band, got {gbt_auc}"
    );
    assert!(
        logistic_auc < 0.65,
        "a linear model is monotone, can't fit a band, got {logistic_auc}"
    );
}

#[test]
fn gbt_is_deterministic_and_serde_round_trips() {
    let ds = band_dataset();
    let a = GbtModel::train(&ds, &GbtConfig::default());
    let b = GbtModel::train(&ds, &GbtConfig::default());
    assert_eq!(a, b, "no randomness ⇒ identical models");

    let json = serde_json::to_vec(&a).unwrap();
    let back: GbtModel = serde_json::from_slice(&json).unwrap();
    assert_eq!(
        a, back,
        "model round-trips through JSON (the shipped format)"
    );
}

#[test]
fn gbt_empty_dataset_is_constant_half() {
    let ds = Dataset {
        rows: vec![],
        horizon_s: 10.0,
    };
    let m = GbtModel::train(&ds, &GbtConfig::default());
    assert_eq!(m.predict(&[0.3; N_FEATURES]), 0.5);
}

#[test]
fn feature_importance_concentrates_on_the_predictive_feature() {
    // band_dataset only varies feature 0, so every split must be on feature 0.
    let m = GbtModel::train(&band_dataset(), &GbtConfig::default());
    let imp = m.feature_importance();
    assert!(
        (imp[0] - 1.0).abs() < 1e-6,
        "feature 0 importance = {}",
        imp[0]
    );
    assert!(
        imp[1..].iter().all(|&w| w == 0.0),
        "other features should be unused"
    );
    assert!(
        (imp.iter().sum::<f32>() - 1.0).abs() < 1e-6,
        "importance sums to 1"
    );

    // An untrained model has no splits -> all-zero importance.
    let empty = GbtModel::train(
        &Dataset {
            rows: vec![],
            horizon_s: 10.0,
        },
        &GbtConfig::default(),
    );
    assert!(empty.feature_importance().iter().all(|&w| w == 0.0));
}
