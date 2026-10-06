//! Expected goals: the chance a shot becomes a goal, from its geometry.
//!
//! A logistic model over seven features of an [`Episode::Shot`] — six of geometry plus the
//! lobby's rank level (corpus tiers shot 10–15% differently, see [`level_of`]) — versioned as its own
//! artifact (`xg_model.json`, next to `rank_norms.json`). [`XgModel::default`] is a
//! hand-set **prior** (`n_train == 0`) so the numbers are usable before a corpus fit;
//! `xg-fit` replaces it with weights fitted on labelled shots. Calibration (Brier,
//! reliability) says how far to trust either.

use replay_analyzer::field::{BACK_WALL_Y, BALL_RADIUS};
use serde::{Deserialize, Serialize};

use crate::calibrate::solve;
use crate::episodes::{Episode, Outcome};

/// Bias + distance, speed, on-target, defenders, edge, rank level.
pub const N: usize = 7;
/// The rank brackets, lowest first (the names `RankNorms` and the corpus manifest use).
const BRACKETS: [&str; 7] = [
    "bronze",
    "silver",
    "gold",
    "platinum",
    "diamond",
    "champion",
    "grand-champion",
];

/// A bracket's position as a rank level in `[-1, 1]` (platinum, the middle, is 0), the xG model's
/// seventh feature. Unknown brackets are `None`; callers treat that as 0, the neutral level.
pub fn level_of(bracket: &str) -> Option<f32> {
    BRACKETS
        .iter()
        .position(|b| *b == bracket)
        .map(|i| (i as f32 - 3.0) / 3.0)
}
/// Goal half-width (uu), to scale how far toward a post the ball is aimed.
const GOAL_HALF_W: f32 = 892.755;
/// Goal height (uu).
const GOAL_H: f32 = 642.775;
/// A shot with no projection is treated as aimed this many half-widths wide.
const NO_AIM_EDGE: f32 = 2.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XgModel {
    pub version: String,
    /// Logistic weights over [`features`], bias first. Models saved before the rank feature have
    /// six weights; the missing one reads as 0, which reproduces them exactly.
    #[serde(deserialize_with = "weights")]
    pub w: [f32; N],
    /// Shots the weights were fitted on; 0 for the hand-set prior.
    pub n_train: usize,
    /// Monotone `[raw p, calibrated p]` knots mapping the logistic output onto the observed
    /// goal rate (see [`XgModel::fit_calibrated`]); empty means the output is used as is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calib: Vec<[f32; 2]>,
}

impl Default for XgModel {
    /// The uncalibrated prior: closer, faster, on-target and less-defended shots are better.
    fn default() -> Self {
        Self {
            version: "xg-prior-v1".into(),
            w: [-1.6, -0.30, 0.30, 1.9, -0.35, -0.5, 0.0],
            n_train: 0,
            calib: Vec::new(),
        }
    }
}

fn weights<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[f32; N], D::Error> {
    let v = Vec::<f32>::deserialize(d)?;
    if !(N - 1..=N).contains(&v.len()) {
        return Err(serde::de::Error::custom(format!(
            "expected {} or {N} weights, got {}",
            N - 1,
            v.len()
        )));
    }
    let mut w = [0.0; N];
    w[..v.len()].copy_from_slice(&v);
    Ok(w)
}

/// `[1, distance to goal (km), ball speed (km/s), on target, defenders, aim edge, rank level]`,
/// or `None` for anything but a shot. The rank level is 0 (neutral); [`sample`] and
/// [`XgModel::at_level`] supply a real one.
///
/// Only what is known before the shot resolves: `on target` is the projected path alone, never
/// the episode's `on_target` flag, which is also set for goals and saves once their outcome is known.
pub fn features(e: &Episode) -> Option<[f32; N]> {
    let Episode::Shot {
        speed,
        aim,
        at,
        def,
        ..
    } = e
    else {
        return None;
    };
    let on_target = aim
        .is_some_and(|a| a[0].abs() <= GOAL_HALF_W - BALL_RADIUS && a[1] <= GOAL_H - BALL_RADIUS);
    let dist = at[0].hypot(BACK_WALL_Y - at[1]);
    let edge = aim.map_or(NO_AIM_EDGE, |a| (a[0].abs() / GOAL_HALF_W).min(NO_AIM_EDGE));
    Some([
        1.0,
        dist / 1000.0,
        speed / 1000.0,
        f32::from(on_target),
        f32::from(*def),
        edge,
        0.0,
    ])
}

/// Folds for the out-of-fold predictions the calibration map is learned from.
const CALIB_FOLDS: usize = 5;
/// Shots per bin before pooling: enough that a bin's goal rate is not noise.
const CALIB_BIN: usize = 200;

/// Monotone `[mean prediction, goal rate]` knots from `(prediction, goal)` pairs: equal-count
/// bins, then pool-adjacent-violators so the rate never falls as the prediction rises. Empty
/// (identity) with too few pairs to calibrate on.
fn isotonic(mut pairs: Vec<(f32, bool)>) -> Vec<[f32; 2]> {
    if pairs.len() < 2 * CALIB_BIN {
        return Vec::new();
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let bins = pairs.len() / CALIB_BIN;
    // (sum of predictions, goals, shots) per block, pooled left to right.
    let mut blocks: Vec<(f64, f64, f64)> = Vec::new();
    for b in 0..bins {
        let end = if b + 1 == bins {
            pairs.len()
        } else {
            (b + 1) * CALIB_BIN
        };
        let chunk = &pairs[b * CALIB_BIN..end];
        let mut cur = (
            chunk.iter().map(|p| f64::from(p.0)).sum::<f64>(),
            chunk.iter().filter(|p| p.1).count() as f64,
            chunk.len() as f64,
        );
        while let Some(prev) = blocks.last().filter(|p| p.1 / p.2 > cur.1 / cur.2) {
            cur = (prev.0 + cur.0, prev.1 + cur.1, prev.2 + cur.2);
            blocks.pop();
        }
        blocks.push(cur);
    }
    blocks
        .iter()
        .map(|b| [(b.0 / b.2) as f32, (b.1 / b.2) as f32])
        .collect()
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

impl XgModel {
    /// This model for a lobby at rank `level` (see [`level_of`]): the rank term folded into the
    /// bias, so [`predict`](Self::predict) needs no extra input.
    pub fn at_level(&self, level: f32) -> Self {
        let mut m = self.clone();
        m.w[0] += m.w[N - 1] * level;
        m.w[N - 1] = 0.0;
        m
    }

    /// P(goal) for a shot; 0 for other episodes.
    pub fn predict(&self, e: &Episode) -> f32 {
        features(e).map_or(0.0, |x| self.p(&x) as f32)
    }

    /// The raw logistic output, before calibration.
    fn raw(&self, x: &[f32; N]) -> f64 {
        sigmoid(x.iter().zip(&self.w).map(|(a, b)| f64::from(a * b)).sum())
    }

    fn p(&self, x: &[f32; N]) -> f64 {
        self.calibrated(self.raw(x))
    }

    /// `raw` mapped through the calibration knots (linear between them, flat past the ends).
    fn calibrated(&self, raw: f64) -> f64 {
        let k = &self.calib;
        let (Some(first), Some(last)) = (k.first(), k.last()) else {
            return raw;
        };
        if raw <= f64::from(first[0]) {
            return f64::from(first[1]);
        }
        match k.windows(2).find(|w| raw <= f64::from(w[1][0])) {
            Some(w) => {
                let (x0, x1) = (f64::from(w[0][0]), f64::from(w[1][0]));
                let t = if x1 > x0 { (raw - x0) / (x1 - x0) } else { 1.0 };
                f64::from(w[0][1]) + t * (f64::from(w[1][1]) - f64::from(w[0][1]))
            }
            None => f64::from(last[1]),
        }
    }

    /// Ridge-regularised logistic regression by Newton's method (the bias is not
    /// penalised). `None` with too few shots, or when the system is singular.
    pub fn fit(samples: &[([f32; N], bool)], ridge: f64) -> Option<Self> {
        if samples.len() < N {
            return None;
        }
        let mut w = [0.0f64; N];
        for _ in 0..25 {
            let (mut grad, mut hess) = ([0.0f64; N], vec![vec![0.0f64; N]; N]);
            for (x, y) in samples {
                let z: f64 = x.iter().zip(&w).map(|(a, b)| f64::from(*a) * b).sum();
                let p = sigmoid(z);
                for i in 0..N {
                    grad[i] += (f64::from(u8::from(*y)) - p) * f64::from(x[i]);
                    for j in 0..N {
                        hess[i][j] += p * (1.0 - p) * f64::from(x[i]) * f64::from(x[j]);
                    }
                }
            }
            for i in 1..N {
                grad[i] -= ridge * w[i];
                hess[i][i] += ridge;
            }
            hess[0][0] += 1e-9;
            let step = solve(hess, grad.to_vec())?;
            w.iter_mut().zip(&step).for_each(|(a, s)| *a += s);
            if step.iter().all(|s| s.abs() < 1e-6) {
                break;
            }
        }
        Some(Self {
            version: format!("xg-fit-n{}", samples.len()),
            w: w.map(|v| v as f32),
            n_train: samples.len(),
            calib: Vec::new(),
        })
    }

    /// [`fit`](Self::fit), then a calibration map learned from out-of-fold predictions: every
    /// shot is predicted by a model that never saw its fold, so the map corrects the model's
    /// real (not memorised) bias — here a mid-range overshoot and an underrated top.
    pub fn fit_calibrated(samples: &[([f32; N], bool)], ridge: f64) -> Option<Self> {
        let mut model = Self::fit(samples, ridge)?;
        let mut oof = Vec::with_capacity(samples.len());
        for fold in 0..CALIB_FOLDS {
            let (held, rest): (Vec<_>, Vec<_>) = samples
                .iter()
                .enumerate()
                .partition(|(i, _)| i % CALIB_FOLDS == fold);
            let rest: Vec<_> = rest.into_iter().map(|(_, s)| *s).collect();
            let m = Self::fit(&rest, ridge)?;
            oof.extend(held.into_iter().map(|(_, (x, y))| (m.raw(x) as f32, *y)));
        }
        model.calib = isotonic(oof);
        model.version.push_str("-iso");
        Some(model)
    }

    /// Mean squared error of the predicted probabilities (lower is better).
    pub fn brier(&self, samples: &[([f32; N], bool)]) -> f32 {
        let se: f64 = samples
            .iter()
            .map(|(x, y)| (self.p(x) - f64::from(u8::from(*y))).powi(2))
            .sum();
        (se / samples.len().max(1) as f64) as f32
    }

    /// `(mean predicted, observed goal rate, shots)` per predicted-probability decile.
    pub fn reliability(&self, samples: &[([f32; N], bool)]) -> Vec<(f32, f32, usize)> {
        let mut bins = [(0.0f64, 0usize, 0usize); 10]; // (sum p, goals, n)
        for (x, y) in samples {
            let p = self.p(x);
            let b = &mut bins[((p * 10.0) as usize).min(9)];
            *b = (b.0 + p, b.1 + usize::from(*y), b.2 + 1);
        }
        bins.iter()
            .filter(|b| b.2 > 0)
            .map(|b| ((b.0 / b.2 as f64) as f32, b.1 as f32 / b.2 as f32, b.2))
            .collect()
    }
}

/// A labelled training example from a shot episode (goal = 1) in a lobby at rank `level`.
pub fn sample(e: &Episode, level: f32) -> Option<([f32; N], bool)> {
    let Episode::Shot { outcome, .. } = e else {
        return None;
    };
    let mut x = features(e)?;
    x[N - 1] = level;
    Some((x, *outcome == Outcome::Goal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(dist_y: f32, on_target: bool, def: u8) -> Episode {
        Episode::Shot {
            pri: 1,
            t: 0.0,
            speed: 1500.0,
            aim: on_target.then_some([0.0, 100.0]),
            tta: None,
            on_target,
            outcome: Outcome::Off,
            at: [0.0, BACK_WALL_Y - dist_y],
            def,
            xg: 0.0,
        }
    }

    #[test]
    fn the_prior_prefers_close_on_target_undefended_shots() {
        let m = XgModel::default();
        let good = m.predict(&shot(1500.0, true, 0));
        assert!(good > m.predict(&shot(4000.0, true, 0)), "closer is better");
        assert!(
            good > m.predict(&shot(1500.0, false, 0)),
            "on target is better"
        );
        assert!(
            good > m.predict(&shot(1500.0, true, 2)),
            "fewer defenders is better"
        );
        assert!(
            (0.0..=1.0).contains(&good)
                && m.predict(&Episode::Recovery {
                    pri: 1,
                    t0: 0.0,
                    dur: 1.0,
                    done: true
                }) == 0.0
        );
    }

    /// Labels drawn from a known logistic model are recovered, and the fit beats the
    /// constant base-rate predictor on the same data.
    #[test]
    fn fit_recovers_a_known_model() {
        let truth = XgModel {
            version: "t".into(),
            w: [-1.0, -0.6, 0.0, 2.0, -0.5, 0.0, 0.4],
            n_train: 0,
            calib: Vec::new(),
        };
        let mut rng = 12345u64;
        let mut next = || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) as f32 / (1u64 << 31) as f32
        };
        let data: Vec<_> = (0..4000)
            .map(|_| {
                let x = [
                    1.0,
                    0.5 + 4.0 * next(),
                    1.0 + next(),
                    f32::from(next() > 0.5),
                    (next() * 3.0).floor(),
                    next(),
                    2.0 * next() - 1.0,
                ];
                (x, f64::from(next()) < truth.p(&x))
            })
            .collect();
        let fit = XgModel::fit(&data, 1e-3).expect("fit");
        for (i, (a, b)) in fit.w.iter().zip(&truth.w).enumerate() {
            assert!((a - b).abs() < 0.35, "w[{i}]: {a} vs {b}");
        }
        let rate = data.iter().filter(|d| d.1).count() as f32 / data.len() as f32;
        let base = data
            .iter()
            .map(|d| (rate - f32::from(d.1)).powi(2))
            .sum::<f32>()
            / data.len() as f32;
        assert!(
            fit.brier(&data) < base,
            "{} vs base {base}",
            fit.brier(&data)
        );
        let rel = fit.reliability(&data);
        assert!(
            rel.iter()
                .filter(|b| b.2 >= 100)
                .all(|b| (b.0 - b.1).abs() < 0.1),
            "{rel:?}"
        );
    }

    #[test]
    fn too_few_shots_is_not_a_fit() {
        assert!(XgModel::fit(&[([1.0; N], true)], 1.0).is_none());
    }

    /// Predictions that are systematically wrong in the middle and at the top are pulled onto the
    /// observed rate, the map stays monotone, and an uncalibrated model is unchanged.
    #[test]
    fn calibration_corrects_a_distorted_model_and_stays_monotone() {
        let mut rng = 99u64;
        let mut next = || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) as f32 / (1u64 << 31) as f32
        };
        // True goal rate is the prediction squashed toward 0.5 in the middle, lifted at the top.
        let pairs: Vec<(f32, bool)> = (0..6000)
            .map(|_| {
                let p = next();
                let truth = if p > 0.8 { 0.95 } else { 0.1 + 0.6 * p * p };
                (p, next() < truth)
            })
            .collect();
        let knots = isotonic(pairs.clone());
        assert!(
            knots
                .windows(2)
                .all(|w| w[0][0] <= w[1][0] && w[0][1] <= w[1][1]),
            "{knots:?}"
        );
        let m = XgModel {
            calib: knots,
            ..XgModel::default()
        };
        let err = |f: &dyn Fn(f32) -> f32| {
            pairs
                .iter()
                .map(|(p, y)| (f(*p) - f32::from(*y)).powi(2))
                .sum::<f32>()
                / pairs.len() as f32
        };
        let raw = err(&|p| p);
        let cal = err(&|p| m.calibrated(f64::from(p)) as f32);
        assert!(cal < raw - 0.01, "calibrated {cal} vs raw {raw}");
        assert!((m.calibrated(0.9) - 0.95).abs() < 0.05);
        assert_eq!(
            XgModel::default().calibrated(0.37),
            0.37,
            "no knots = identity"
        );
        assert!(
            isotonic(vec![(0.5, true); 10]).is_empty(),
            "too little data to calibrate"
        );
    }

    #[test]
    fn calibrated_fit_is_out_of_fold_and_a_saved_model_without_knots_loads() {
        let mut rng = 7u64;
        let mut next = || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) as f32 / (1u64 << 31) as f32
        };
        let data: Vec<_> = (0..3000)
            .map(|_| {
                let x = [
                    1.0,
                    0.5 + 4.0 * next(),
                    1.0 + next(),
                    f32::from(next() > 0.5),
                    0.0,
                    next(),
                    0.0,
                ];
                let p = 1.0 / (1.0 + (-(1.0 - 0.6 * x[1] + 2.0 * x[3])).exp());
                (x, next() < p)
            })
            .collect();
        let m = XgModel::fit_calibrated(&data, 1.0).expect("fit");
        assert!(m.version.ends_with("-iso") && !m.calib.is_empty());
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<XgModel>(&json).unwrap(), m);
        let plain = r#"{"version":"v","w":[0,0,0,0,0,0,0],"n_train":1}"#;
        assert!(serde_json::from_str::<XgModel>(plain)
            .unwrap()
            .calib
            .is_empty());
    }

    #[test]
    fn rank_levels_run_bronze_to_grand_champion_with_platinum_neutral() {
        assert_eq!(level_of("bronze"), Some(-1.0));
        assert_eq!(level_of("platinum"), Some(0.0));
        assert_eq!(level_of("grand-champion"), Some(1.0));
        assert_eq!(level_of("unranked"), None);
    }

    #[test]
    fn at_level_folds_the_rank_term_into_the_bias() {
        let mut m = XgModel::default();
        m.w[N - 1] = 0.5;
        let x = features(&shot(1500.0, true, 0)).unwrap();
        for level in [-1.0, 0.0, 0.7, 1.0] {
            let mut with_level = x;
            with_level[N - 1] = level;
            let folded = m.at_level(level);
            assert!((m.p(&with_level) - folded.p(&x)).abs() < 1e-6, "{level}");
        }
        let e = shot(1500.0, true, 0);
        assert!(m.at_level(1.0).predict(&e) > m.at_level(-1.0).predict(&e));
    }

    #[test]
    fn a_model_saved_with_six_weights_loads_and_keeps_its_predictions() {
        let old = r#"{"version":"xg-fit-n1","w":[-1.0,-0.5,0.2,1.0,-0.3,-0.4],"n_train":1}"#;
        let m: XgModel = serde_json::from_str(old).expect("old model loads");
        assert_eq!(m.w, [-1.0, -0.5, 0.2, 1.0, -0.3, -0.4, 0.0]);
        let e = shot(2000.0, true, 1);
        assert_eq!(m.predict(&e), m.at_level(1.0).predict(&e), "no rank effect");
        assert!(
            serde_json::from_str::<XgModel>(r#"{"version":"x","w":[1.0],"n_train":0}"#).is_err()
        );
    }

    /// The model must not see the outcome: a shot's features are the same whether it ended as
    /// a goal (which forces the episode's `on_target` flag) or as a miss.
    #[test]
    fn features_do_not_depend_on_the_outcome() {
        let as_miss = shot(1500.0, false, 1);
        let mut as_goal = shot(1500.0, false, 1);
        if let Episode::Shot {
            outcome, on_target, ..
        } = &mut as_goal
        {
            (*outcome, *on_target) = (Outcome::Goal, true);
        }
        assert_eq!(features(&as_goal), features(&as_miss));
        // ...while a path that really crosses the mouth is on target.
        assert_eq!(features(&shot(1500.0, true, 1)).unwrap()[3], 1.0);
        assert_eq!(features(&as_miss).unwrap()[3], 0.0);
    }
}
