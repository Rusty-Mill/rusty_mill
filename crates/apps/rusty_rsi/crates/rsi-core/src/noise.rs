//! Run-to-run noise calibration and the accept margin (ADR-0005 §5).
//!
//! `rsi calibrate` grades the baseline harness several times on disjoint
//! seeds. The spread of those grades is the noise floor: a candidate must
//! beat the incumbent by more than that spread can explain.

use crate::error::CoreError;
use crate::score::Grade;

/// Default one-sided z-value: 95% confidence that a winning delta is real.
pub const DEFAULT_Z: f64 = 1.645;

/// Summary statistics of repeated grades of one agent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseBand {
    samples: usize,
    mean: f64,
    std_dev: f64,
    min: f64,
    max: f64,
}

impl NoiseBand {
    /// Summarises `grades` with the sample (`n - 1`) standard deviation.
    ///
    /// # Errors
    /// [`CoreError::TooFewGrades`] with fewer than two grades.
    pub fn from_grades(grades: &[Grade]) -> Result<Self, CoreError> {
        let samples = grades.len();
        if samples < 2 {
            return Err(CoreError::TooFewGrades(samples));
        }
        let values = || grades.iter().map(|grade| grade.get());
        let n = samples as f64;
        let mean = values().sum::<f64>() / n;
        let variance = values().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let min = values().fold(f64::INFINITY, f64::min);
        let max = values().fold(f64::NEG_INFINITY, f64::max);
        Ok(Self {
            samples,
            mean,
            std_dev: variance.sqrt(),
            min,
            max,
        })
    }

    /// The accept margin `z · √2 · σ̂`.
    ///
    /// `√2 · σ̂` is the standard deviation of the difference between two
    /// independent grades, so `z` sets the one-sided confidence that a delta
    /// larger than the margin is not noise.
    ///
    /// # Errors
    /// [`CoreError::InvalidParameter`] if `z` is negative or not finite.
    pub fn margin(&self, z: f64) -> Result<Margin, CoreError> {
        if !(z.is_finite() && z >= 0.0) {
            return Err(CoreError::InvalidParameter {
                name: "z",
                value: z,
            });
        }
        Margin::new(z * core::f64::consts::SQRT_2 * self.std_dev)
    }

    /// How many grades were summarised.
    #[must_use]
    pub const fn samples(&self) -> usize {
        self.samples
    }

    /// The mean grade.
    #[must_use]
    pub const fn mean(&self) -> f64 {
        self.mean
    }

    /// The sample standard deviation `σ̂`.
    #[must_use]
    pub const fn std_dev(&self) -> f64 {
        self.std_dev
    }

    /// The lowest grade.
    #[must_use]
    pub const fn min(&self) -> f64 {
        self.min
    }

    /// The highest grade.
    #[must_use]
    pub const fn max(&self) -> f64 {
        self.max
    }
}

/// How far a candidate's fresh grade must exceed the incumbent's to be
/// accepted. Finite and non-negative.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Margin(f64);

impl Margin {
    /// No margin: any strict improvement wins. Only for tests and ablations.
    pub const ZERO: Self = Self(0.0);

    /// Validates a margin, for example one read from config.
    ///
    /// # Errors
    /// [`CoreError::InvalidParameter`] if `value` is negative or not finite.
    pub fn new(value: f64) -> Result<Self, CoreError> {
        if value.is_finite() && value >= 0.0 {
            return Ok(Self(value));
        }
        Err(CoreError::InvalidParameter {
            name: "margin",
            value,
        })
    }

    /// The margin as a plain number.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grades(values: &[f64]) -> Vec<Grade> {
        values
            .iter()
            .map(|&v| Grade::new(v).expect("valid grade"))
            .collect()
    }

    #[test]
    fn summarises_known_sample() {
        // Mean 0.5; squared deviations 0.01+0+0.01 = 0.02; /(n-1)=0.01; σ̂=0.1.
        let band = NoiseBand::from_grades(&grades(&[0.4, 0.5, 0.6])).expect("enough grades");
        assert_eq!(band.samples(), 3);
        assert!((band.mean() - 0.5).abs() < 1e-12);
        assert!((band.std_dev() - 0.1).abs() < 1e-12);
        assert_eq!((band.min(), band.max()), (0.4, 0.6));
    }

    #[test]
    fn margin_is_z_root_two_sigma() {
        let band = NoiseBand::from_grades(&grades(&[0.4, 0.5, 0.6])).expect("enough grades");
        let margin = band.margin(DEFAULT_Z).expect("valid z");
        assert!((margin.get() - DEFAULT_Z * 2f64.sqrt() * 0.1).abs() < 1e-12);
        assert_eq!(band.margin(0.0), Ok(Margin::ZERO));
    }

    #[test]
    fn identical_grades_give_zero_noise() {
        let band = NoiseBand::from_grades(&grades(&[0.3; 5])).expect("enough grades");
        assert_eq!(band.std_dev(), 0.0);
        assert_eq!(band.margin(DEFAULT_Z), Ok(Margin::ZERO));
    }

    #[test]
    fn needs_two_grades() {
        assert_eq!(NoiseBand::from_grades(&[]), Err(CoreError::TooFewGrades(0)));
        assert_eq!(
            NoiseBand::from_grades(&grades(&[0.5])),
            Err(CoreError::TooFewGrades(1))
        );
    }

    #[test]
    fn rejects_bad_z_and_bad_margins() {
        let band = NoiseBand::from_grades(&grades(&[0.1, 0.2])).expect("enough grades");
        assert!(band.margin(-0.1).is_err());
        assert!(band.margin(f64::NAN).is_err());
        assert!(band.margin(f64::INFINITY).is_err());
        assert!(Margin::new(-1e-9).is_err());
        assert!(Margin::new(f64::NAN).is_err());
        assert_eq!(Margin::new(0.05).map(Margin::get), Ok(0.05));
    }
}
