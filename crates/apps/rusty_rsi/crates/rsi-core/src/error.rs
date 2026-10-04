//! The error type for invalid domain values.

use rusty_err::Error;

/// A value was rejected at construction or a domain rule was violated.
///
/// Budget exhaustion and hash-chain failures have their own types
/// ([`crate::BudgetExhausted`], [`crate::ChainError`]) because callers
/// handle them as outcomes, not as bugs.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CoreError {
    /// A score or grade was not finite or fell outside `[0, 1]`.
    #[error("score {0} is not a finite value in [0, 1]")]
    ScoreOutOfRange(f64),
    /// A grade was requested over no tasks.
    #[error("cannot grade an empty set of tasks")]
    EmptyGrade,
    /// A task contributed no scores to a grade.
    #[error("task `{0}` has no scores")]
    EmptyTask(String),
    /// Noise calibration needs at least two grades for a sample deviation.
    #[error("noise calibration needs at least 2 grades, got {0}")]
    TooFewGrades(usize),
    /// A numeric parameter was out of its documented range.
    #[error("{name} is out of range: {value}")]
    InvalidParameter {
        /// The parameter's name.
        name: &'static str,
        /// The rejected value.
        value: f64,
    },
    /// A budget had a zero token or wall-clock limit.
    #[error("budget {0} limit must be greater than zero")]
    EmptyBudget(&'static str),
    /// A re-evaluation reused a seed from the round it is meant to confirm.
    #[error("seed {0} was reused; re-evaluation seeds must be fresh")]
    SeedReused(u64),
    /// An evaluation carried no seeds.
    #[error("an evaluation must carry at least one seed")]
    NoSeeds,
    /// An identifier failed validation.
    #[error("invalid {kind}: `{value}`")]
    InvalidId {
        /// What kind of identifier it was meant to be.
        kind: &'static str,
        /// The rejected text.
        value: String,
    },
}
