//! A dudect-style timing test: time a closure on two input classes and ask
//! whether the means differ (Welch's t). A large `|t|` means a measurable
//! leak. A small one means only that none was detected on this machine, with
//! these classes, this many samples.

use std::time::Instant;

/// `|t|` above this is treated as a detected leak (dudect's usual threshold).
pub const THRESHOLD: f64 = 4.5;

/// Welch's t statistic for two samples. `None` if either has fewer than two
/// values or both variances are zero.
pub fn welch_t(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.len() < 2 || b.len() < 2 {
        return None;
    }
    let (ma, va) = mean_var(a);
    let (mb, vb) = mean_var(b);
    let se = (va / a.len() as f64 + vb / b.len() as f64).sqrt();
    if se == 0.0 {
        return None;
    }
    Some((ma - mb) / se)
}

fn mean_var(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, var)
}

/// Drops the slowest `percent` of samples (interrupts, preemption).
pub fn crop_slowest(mut v: Vec<f64>, percent: usize) -> Vec<f64> {
    v.sort_by(|x, y| x.total_cmp(y));
    let keep = v.len() - v.len() * percent.min(100) / 100;
    v.truncate(keep);
    v
}

/// Times `run(class)` `samples` times with classes chosen by a seeded
/// xorshift generator, and returns `|t|` after cropping the slowest 5%.
/// `run` must do the work for one input of the given class (`false` / `true`)
/// and return something to keep the optimiser honest.
pub fn leak_statistic<T>(samples: usize, seed: u64, mut run: impl FnMut(bool) -> T) -> Option<f64> {
    let mut state = seed | 1;
    let (mut zero, mut one) = (Vec::new(), Vec::new());
    for _ in 0..samples {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let class = state & 1 == 1;
        let start = Instant::now();
        core::hint::black_box(run(class));
        let ns = start.elapsed().as_nanos() as f64;
        if class {
            one.push(ns)
        } else {
            zero.push(ns)
        }
    }
    welch_t(&crop_slowest(zero, 5), &crop_slowest(one, 5)).map(f64::abs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_samples_have_no_statistic() {
        assert_eq!(welch_t(&[1.0, 1.0, 1.0], &[1.0, 1.0, 1.0]), None);
    }

    #[test]
    fn too_few_samples_have_no_statistic() {
        assert_eq!(welch_t(&[1.0], &[1.0, 2.0]), None);
    }

    #[test]
    fn separated_samples_exceed_threshold() {
        let a: Vec<f64> = (0..100).map(|i| 10.0 + (i % 3) as f64 * 0.1).collect();
        let b: Vec<f64> = (0..100).map(|i| 20.0 + (i % 3) as f64 * 0.1).collect();
        assert!(welch_t(&a, &b).unwrap().abs() > THRESHOLD);
    }

    #[test]
    fn same_distribution_stays_under_threshold() {
        let a: Vec<f64> = (0..200).map(|i| 10.0 + (i % 7) as f64).collect();
        let b: Vec<f64> = (0..200).map(|i| 10.0 + ((i + 3) % 7) as f64).collect();
        assert!(welch_t(&a, &b).unwrap().abs() < THRESHOLD);
    }

    #[test]
    fn crop_drops_the_slowest() {
        let v = vec![5.0, 1.0, 100.0, 2.0];
        assert_eq!(crop_slowest(v, 25), vec![1.0, 2.0, 5.0]);
    }

    /// Planted leak must be detected; slow, so run by the scheduled job:
    /// `cargo test -p rusty_ct_check -- --ignored`.
    #[test]
    #[ignore = "timing-sensitive; scheduled job only"]
    fn planted_early_exit_is_detected() {
        let secret = [0x5au8; 512];
        let t = leak_statistic(100_000, 7, |differs| {
            let mut guess = secret;
            if differs {
                guess[0] ^= 1;
            }
            secret
                .iter()
                .zip(core::hint::black_box(&guess))
                .all(|(a, b)| a == b)
        })
        .unwrap();
        assert!(t > THRESHOLD, "t = {t}");
    }
}
