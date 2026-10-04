//! Scores and grades.
//!
//! Every task normalises its metric to a higher-is-better [`Score`] in
//! `[0, 1]`, so a mean across heterogeneous tasks is meaningful. A
//! [`Grade`] is AIDE²'s `g(a)`: the mean over tasks of each task's mean
//! private score across seeds.

use crate::error::CoreError;

/// A task score in `[0, 1]`, higher is better. NaN and infinities are
/// unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Score(f64);

impl Score {
    /// The lowest possible score.
    pub const ZERO: Self = Self(0.0);
    /// The highest possible score.
    pub const ONE: Self = Self(1.0);

    /// Validates `value` as a score.
    ///
    /// # Errors
    /// [`CoreError::ScoreOutOfRange`] if `value` is not finite or outside `[0, 1]`.
    pub fn new(value: f64) -> Result<Self, CoreError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            return Ok(Self(value));
        }
        Err(CoreError::ScoreOutOfRange(value))
    }

    /// The score as a plain number.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// An agent's grade `g(a)` in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Grade(Score);

impl Grade {
    /// Wraps an already-aggregated value, for example one read back from lineage.
    ///
    /// # Errors
    /// [`CoreError::ScoreOutOfRange`] if `value` is not finite or outside `[0, 1]`.
    pub fn new(value: f64) -> Result<Self, CoreError> {
        Score::new(value).map(Self)
    }

    /// Aggregates per-task scores into a grade: the mean of each task's mean.
    ///
    /// Averaging per task first keeps a task run on more seeds from
    /// outweighing the others, matching `g(a) = (1/T) Σ_t r_priv(x̂_t)`.
    ///
    /// # Errors
    /// [`CoreError::EmptyGrade`] with no tasks, [`CoreError::EmptyTask`] when a
    /// task has no scores.
    pub fn from_tasks<'a, I>(tasks: I) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = (&'a str, &'a [Score])>,
    {
        let mut task_means = Vec::new();
        for (task, scores) in tasks {
            let mean = mean(scores.iter().map(|score| score.get()))
                .ok_or_else(|| CoreError::EmptyTask(task.to_owned()))?;
            task_means.push(mean);
        }
        let grade = mean(task_means.into_iter()).ok_or(CoreError::EmptyGrade)?;
        Self::new(grade)
    }

    /// The grade as a plain number.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0.get()
    }

    /// `self - other`: positive when `self` is better.
    #[must_use]
    pub fn delta(self, other: Self) -> f64 {
        self.get() - other.get()
    }
}

/// The arithmetic mean, or `None` for an empty input.
///
/// For inputs in `[0, 1]` the result stays in `[0, 1]`: IEEE rounding is
/// monotonic, so a running sum never exceeds the (exactly representable)
/// element count.
fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, count) = values.fold((0.0, 0u64), |(sum, count), v| (sum + v, count + 1));
    (count > 0).then(|| sum / count as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scores(values: &[f64]) -> Vec<Score> {
        values
            .iter()
            .map(|&v| Score::new(v).expect("valid score"))
            .collect()
    }

    #[test]
    fn score_accepts_closed_unit_interval() {
        assert_eq!(Score::new(0.0), Ok(Score::ZERO));
        assert_eq!(Score::new(1.0), Ok(Score::ONE));
        assert_eq!(Score::new(0.25).map(Score::get), Ok(0.25));
    }

    #[test]
    fn score_rejects_out_of_range_and_non_finite() {
        for bad in [
            -f64::EPSILON,
            1.0 + f64::EPSILON,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert_eq!(Score::new(bad), Err(CoreError::ScoreOutOfRange(bad)));
        }
        assert!(matches!(
            Score::new(f64::NAN),
            Err(CoreError::ScoreOutOfRange(_))
        ));
    }

    #[test]
    fn grade_is_mean_of_task_means() {
        let a = scores(&[1.0, 0.0, 1.0, 0.0]); // mean 0.5 over 4 seeds
        let b = scores(&[1.0]); // mean 1.0 over 1 seed
        let grade = Grade::from_tasks([("a", a.as_slice()), ("b", b.as_slice())]);
        assert_eq!(grade.map(Grade::get), Ok(0.75));
    }

    #[test]
    fn grade_of_all_ones_is_exactly_one() {
        let ones = vec![Score::ONE; 1_000];
        let grade = Grade::from_tasks([("t", ones.as_slice())]);
        assert_eq!(grade.map(Grade::get), Ok(1.0));
    }

    #[test]
    fn grade_rejects_empty_inputs() {
        let none: [(&str, &[Score]); 0] = [];
        assert_eq!(Grade::from_tasks(none), Err(CoreError::EmptyGrade));
        let empty: &[Score] = &[];
        assert_eq!(
            Grade::from_tasks([("tsp", empty)]),
            Err(CoreError::EmptyTask("tsp".to_owned()))
        );
    }

    #[test]
    fn delta_is_signed() {
        let hi = Grade::new(0.6).expect("valid");
        let lo = Grade::new(0.4).expect("valid");
        assert!((hi.delta(lo) - 0.2).abs() < 1e-12);
        assert!(lo.delta(hi) < 0.0);
    }
}
