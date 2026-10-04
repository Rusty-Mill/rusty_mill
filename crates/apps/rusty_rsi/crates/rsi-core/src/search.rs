//! Selection helpers for choosing which candidate to build on.
//!
//! AIDE²'s outer loop is greedy: it always rewrites the incumbent. UCB1 and
//! softmax sampling are the configurable alternatives (ADR-0005 §7). These
//! are plain functions rather than a `SearchPolicy` trait, because the outer
//! loop's parent selection is their only caller.

use crate::error::CoreError;
use crate::rng::SplitMix64;

/// The index of the largest value, ties going to the lowest index.
///
/// Returns `Ok(None)` for an empty slice.
///
/// # Errors
/// [`CoreError::InvalidParameter`] if any value is not finite.
pub fn argmax(values: &[f64]) -> Result<Option<usize>, CoreError> {
    check_finite("value", values)?;
    let best =
        values
            .iter()
            .enumerate()
            .fold(None, |best: Option<(usize, f64)>, (i, &v)| match best {
                Some((_, b)) if b >= v => best,
                _ => Some((i, v)),
            });
    Ok(best.map(|(i, _)| i))
}

/// One bandit arm: how often it was chosen and the total reward it earned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arm {
    /// Times this arm was chosen.
    pub pulls: u64,
    /// Sum of the rewards observed on this arm.
    pub reward_sum: f64,
}

/// UCB1 (Auer, Cesa-Bianchi and Fischer, 2002): the arm maximising
/// `mean + c · √(ln N / n)`.
///
/// An arm that has never been pulled is chosen first (lowest index), since
/// its bound is infinite. Ties go to the lowest index. Returns `Ok(None)`
/// for no arms.
///
/// # Errors
/// [`CoreError::InvalidParameter`] if `exploration` is negative or not
/// finite, or a reward sum is not finite.
pub fn ucb1(arms: &[Arm], exploration: f64) -> Result<Option<usize>, CoreError> {
    if !(exploration.is_finite() && exploration >= 0.0) {
        return Err(CoreError::InvalidParameter {
            name: "exploration",
            value: exploration,
        });
    }
    let sums: Vec<f64> = arms.iter().map(|arm| arm.reward_sum).collect();
    check_finite("reward_sum", &sums)?;
    if let Some(unpulled) = arms.iter().position(|arm| arm.pulls == 0) {
        return Ok(Some(unpulled));
    }
    let total = arms.iter().map(|arm| arm.pulls as f64).sum::<f64>();
    let bounds: Vec<f64> = arms
        .iter()
        .map(|arm| {
            let n = arm.pulls as f64;
            arm.reward_sum / n + exploration * (total.ln() / n).sqrt()
        })
        .collect();
    argmax(&bounds)
}

/// Softmax probabilities `exp(logit / temperature) / Σ exp(logit_j / temperature)`.
///
/// Lower temperatures approach greedy; higher ones approach uniform. Each
/// exponent is computed as `(logit/2 - max/2) / temperature * 2`, which is
/// exact for ordinary inputs and never overflows to `inf - inf` or NaN for
/// finite extremes such as `[f64::MAX, -f64::MAX]`. Returns an empty vector
/// for no logits.
///
/// # Errors
/// [`CoreError::InvalidParameter`] if `temperature` is not finite and
/// positive, or a logit is not finite.
pub fn softmax(logits: &[f64], temperature: f64) -> Result<Vec<f64>, CoreError> {
    if !(temperature.is_finite() && temperature > 0.0) {
        return Err(CoreError::InvalidParameter {
            name: "temperature",
            value: temperature,
        });
    }
    let Some(top) = argmax(logits)? else {
        return Ok(Vec::new());
    };
    let half_max = logits[top] / 2.0;
    // `l/2 - max/2` lies in [-f64::MAX, 0], so it cannot overflow; dividing by
    // a positive temperature can only push it towards -inf, where exp is 0.
    let weights: Vec<f64> = logits
        .iter()
        .map(|&l| ((l / 2.0 - half_max) / temperature * 2.0).exp())
        .collect();
    // The maximum's weight is exactly exp(0) = 1, so the total is at least 1.
    let total: f64 = weights.iter().sum();
    Ok(weights.into_iter().map(|w| w / total).collect())
}

/// Samples an index with the [`softmax`] probabilities of `logits`.
///
/// Returns `Ok(None)` for no logits.
///
/// # Errors
/// As [`softmax`].
pub fn softmax_sample(
    logits: &[f64],
    temperature: f64,
    rng: &mut SplitMix64,
) -> Result<Option<usize>, CoreError> {
    let probabilities = softmax(logits, temperature)?;
    let Some(mode) = argmax(&probabilities)? else {
        return Ok(None);
    };
    let mut target = rng.next_f64();
    for (i, p) in probabilities.iter().enumerate() {
        if target < *p {
            return Ok(Some(i));
        }
        target -= p;
    }
    // Rounding can leave a sliver past the last bucket; fall back to the mode.
    Ok(Some(mode))
}

fn check_finite(name: &'static str, values: &[f64]) -> Result<(), CoreError> {
    match values.iter().find(|v| !v.is_finite()) {
        Some(&value) => Err(CoreError::InvalidParameter { name, value }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Seed;

    #[test]
    fn argmax_basics() {
        assert_eq!(argmax(&[]), Ok(None));
        assert_eq!(argmax(&[0.1, 0.9, 0.3]), Ok(Some(1)));
        assert_eq!(
            argmax(&[0.5, 0.5]),
            Ok(Some(0)),
            "ties go to the lowest index"
        );
        assert_eq!(argmax(&[-3.0, -1.0]), Ok(Some(1)));
        assert!(argmax(&[0.1, f64::NAN]).is_err());
    }

    #[test]
    fn ucb1_tries_unpulled_arms_first() {
        let arms = [
            Arm {
                pulls: 10,
                reward_sum: 9.0,
            },
            Arm {
                pulls: 0,
                reward_sum: 0.0,
            },
            Arm {
                pulls: 0,
                reward_sum: 0.0,
            },
        ];
        assert_eq!(ucb1(&arms, 1.0), Ok(Some(1)));
    }

    #[test]
    fn ucb1_without_exploration_is_greedy_on_means() {
        let arms = [
            Arm {
                pulls: 10,
                reward_sum: 5.0,
            },
            Arm {
                pulls: 2,
                reward_sum: 1.2,
            },
        ];
        assert_eq!(ucb1(&arms, 0.0), Ok(Some(1)));
    }

    #[test]
    fn ucb1_exploration_favours_rarely_pulled_arm() {
        // Means 0.6 vs 0.5, but arm 1 has 1 pull against 100.
        let arms = [
            Arm {
                pulls: 100,
                reward_sum: 60.0,
            },
            Arm {
                pulls: 1,
                reward_sum: 0.5,
            },
        ];
        assert_eq!(ucb1(&arms, 0.0), Ok(Some(0)));
        assert_eq!(ucb1(&arms, 2f64.sqrt()), Ok(Some(1)));
    }

    #[test]
    fn ucb1_edge_cases() {
        assert_eq!(ucb1(&[], 1.0), Ok(None));
        assert_eq!(
            ucb1(
                &[Arm {
                    pulls: 1,
                    reward_sum: 0.3
                }],
                1.0
            ),
            Ok(Some(0))
        );
        assert!(ucb1(&[], -1.0).is_err());
        assert!(ucb1(&[], f64::NAN).is_err());
        assert!(ucb1(
            &[Arm {
                pulls: 1,
                reward_sum: f64::INFINITY
            }],
            1.0
        )
        .is_err());
    }

    #[test]
    fn softmax_edge_cases() {
        let mut rng = SplitMix64::new(Seed::new(0));
        assert_eq!(softmax_sample(&[], 1.0, &mut rng), Ok(None));
        assert_eq!(softmax_sample(&[0.3], 1.0, &mut rng), Ok(Some(0)));
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(softmax_sample(&[1.0], bad, &mut rng).is_err());
        }
        assert!(softmax_sample(&[1.0, f64::NAN], 1.0, &mut rng).is_err());
    }

    #[test]
    fn softmax_does_not_overflow_on_large_logits() {
        let mut rng = SplitMix64::new(Seed::new(1));
        let pick = softmax_sample(&[1e300, 1e300 - 1.0], 1.0, &mut rng);
        assert!(matches!(pick, Ok(Some(0 | 1))));
    }

    #[test]
    fn softmax_handles_finite_extremes() {
        // Exponents MAX/MAX = 1 and -MAX/MAX = -1: p = 1/(1+e^-2), not [1, 0].
        let p = softmax(&[f64::MAX, -f64::MAX], f64::MAX).expect("finite inputs");
        assert!((p[0] - 0.880_797).abs() < 1e-6, "{p:?}");
        assert!((p[1] - 0.119_203).abs() < 1e-6, "{p:?}");
        assert!(((p[0] + p[1]) - 1.0).abs() < 1e-15);
    }

    #[test]
    fn softmax_extremes_never_produce_nan() {
        let cases: [(&[f64], f64); 4] = [
            (&[f64::MAX, -f64::MAX], f64::MIN_POSITIVE),
            (&[f64::MAX, f64::MAX], f64::MAX),
            (&[-f64::MAX, -f64::MAX, 0.0], 1.0),
            (&[1e-320, -1e-320], 1e-300),
        ];
        for (logits, temperature) in cases {
            let p = softmax(logits, temperature).expect("finite inputs");
            assert!(
                p.iter().all(|x| x.is_finite() && (0.0..=1.0).contains(x)),
                "{p:?}"
            );
            assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-12, "{p:?}");
        }
        let greedy = softmax(&[f64::MAX, -f64::MAX], f64::MIN_POSITIVE).expect("finite");
        assert_eq!(greedy, vec![1.0, 0.0]);
        let tie = softmax(&[f64::MAX, f64::MAX], f64::MAX).expect("finite");
        assert_eq!(tie, vec![0.5, 0.5]);
    }

    #[test]
    fn softmax_matches_textbook_values() {
        let p = softmax(&[0.0, 2f64.ln()], 1.0).expect("finite");
        assert!((p[0] - 1.0 / 3.0).abs() < 1e-15 && (p[1] - 2.0 / 3.0).abs() < 1e-15);
        assert_eq!(softmax(&[], 1.0), Ok(Vec::new()));
    }

    #[test]
    fn softmax_sample_at_finite_extremes_follows_probabilities() {
        let mut rng = SplitMix64::new(Seed::new(4));
        let draws = 40_000;
        let firsts = (0..draws)
            .filter(|_| softmax_sample(&[f64::MAX, -f64::MAX], f64::MAX, &mut rng) == Ok(Some(0)))
            .count();
        let share = firsts as f64 / draws as f64;
        assert!((share - 0.880_797).abs() < 0.01, "{share}");
    }

    #[test]
    fn softmax_low_temperature_is_nearly_greedy() {
        let mut rng = SplitMix64::new(Seed::new(2));
        for _ in 0..1_000 {
            assert_eq!(
                softmax_sample(&[0.1, 0.9, 0.5], 1e-3, &mut rng),
                Ok(Some(1))
            );
        }
    }

    #[test]
    fn softmax_frequencies_match_probabilities() {
        // Logits ln 1, ln 3 at T = 1 give probabilities 0.25 and 0.75.
        let mut rng = SplitMix64::new(Seed::new(3));
        let logits = [0.0, 3f64.ln()];
        let draws = 40_000;
        let ones = (0..draws)
            .filter(|_| softmax_sample(&logits, 1.0, &mut rng) == Ok(Some(1)))
            .count();
        let share = ones as f64 / draws as f64;
        assert!((share - 0.75).abs() < 0.01, "{share}");
    }

    #[test]
    fn softmax_is_reproducible_from_seed() {
        let draw = |seed| {
            let mut rng = SplitMix64::new(Seed::new(seed));
            (0..50)
                .map(|_| softmax_sample(&[0.2, 0.4, 0.6], 0.5, &mut rng))
                .collect::<Vec<_>>()
        };
        assert_eq!(draw(9), draw(9));
    }
}
