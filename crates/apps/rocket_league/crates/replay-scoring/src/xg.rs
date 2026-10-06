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
}

impl Default for XgModel {
    /// The uncalibrated prior: closer, faster, on-target and less-defended shots are better.
    fn default() -> Self {
        Self {
            version: "xg-prior-v1".into(),
            w: [-1.6, -0.30, 0.30, 1.9, -0.35, -0.5, 0.0],
            n_train: 0,
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

    fn p(&self, x: &[f32; N]) -> f64 {
        sigmoid(x.iter().zip(&self.w).map(|(a, b)| f64::from(a * b)).sum())
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
        })
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
