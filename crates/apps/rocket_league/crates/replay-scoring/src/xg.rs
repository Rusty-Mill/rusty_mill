//! Expected goals: the chance a shot becomes a goal, from its geometry.
//!
//! A logistic model over six features of an [`Episode::Shot`], versioned as its own
//! artifact (`xg_model.json`, next to `rank_norms.json`). [`XgModel::default`] is a
//! hand-set **prior** (`n_train == 0`) so the numbers are usable before a corpus fit;
//! `xg-fit` replaces it with weights fitted on labelled shots. Calibration (Brier,
//! reliability) says how far to trust either.

use replay_analyzer::field::BACK_WALL_Y;
use serde::{Deserialize, Serialize};

use crate::calibrate::solve;
use crate::episodes::{Episode, Outcome};

/// Bias + distance, speed, on-target, defenders, edge.
pub const N: usize = 6;
/// Goal half-width (uu), to scale how far toward a post the ball is aimed.
const GOAL_HALF_W: f32 = 892.755;
/// A shot with no projection is treated as aimed this many half-widths wide.
const NO_AIM_EDGE: f32 = 2.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XgModel {
    pub version: String,
    /// Logistic weights over [`features`], bias first.
    pub w: [f32; N],
    /// Shots the weights were fitted on; 0 for the hand-set prior.
    pub n_train: usize,
}

impl Default for XgModel {
    /// The uncalibrated prior: closer, faster, on-target and less-defended shots are better.
    fn default() -> Self {
        Self {
            version: "xg-prior-v1".into(),
            w: [-1.6, -0.30, 0.30, 1.9, -0.35, -0.5],
            n_train: 0,
        }
    }
}

/// `[1, distance to goal (km), ball speed (km/s), on target, defenders, aim edge]`, or `None`
/// for anything but a shot.
pub fn features(e: &Episode) -> Option<[f32; N]> {
    let Episode::Shot {
        speed,
        aim,
        on_target,
        at,
        def,
        ..
    } = e
    else {
        return None;
    };
    let dist = at[0].hypot(BACK_WALL_Y - at[1]);
    let edge = aim.map_or(NO_AIM_EDGE, |a| (a[0].abs() / GOAL_HALF_W).min(NO_AIM_EDGE));
    Some([
        1.0,
        dist / 1000.0,
        speed / 1000.0,
        f32::from(*on_target),
        f32::from(*def),
        edge,
    ])
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

impl XgModel {
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

/// A labelled training example from a shot episode (goal = 1).
pub fn sample(e: &Episode) -> Option<([f32; N], bool)> {
    let Episode::Shot { outcome, .. } = e else {
        return None;
    };
    Some((features(e)?, *outcome == Outcome::Goal))
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
            w: [-1.0, -0.6, 0.0, 2.0, -0.5, 0.0],
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
}
