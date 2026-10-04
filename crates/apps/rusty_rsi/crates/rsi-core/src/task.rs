//! The task ports: public scoring for the inner loop, private grading for
//! the outer loop (ADR-0005 §2, §4).
//!
//! The split is by type. The inner loop only ever holds a [`PublicTask`],
//! which has no way to reach private data; [`PrivateGrader`] is a separate
//! port that the outer loop calls after the inner run has ended.

use core::time::Duration;

use crate::error::CoreError;
use crate::lineage::TaskId;
use crate::rng::Seed;
use crate::score::Score;

/// The largest solution accepted, in bytes.
pub const MAX_SOLUTION_BYTES: usize = 1 << 20;

/// A candidate solution's source code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution(String);

impl Solution {
    /// Validates solution source.
    ///
    /// # Errors
    /// [`CoreError::InvalidId`] if the source is empty, larger than
    /// [`MAX_SOLUTION_BYTES`], or contains a NUL byte.
    pub fn new(source: impl Into<String>) -> Result<Self, CoreError> {
        let source = source.into();
        let valid = !source.trim().is_empty()
            && source.len() <= MAX_SOLUTION_BYTES
            && !source.contains('\0');
        if valid {
            return Ok(Self(source));
        }
        let preview: String = source.chars().take(40).collect();
        Err(CoreError::InvalidId {
            kind: "solution source",
            value: preview,
        })
    }

    /// The source code.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.0
    }
}

/// What the inner loop learns from running a solution on public data.
#[derive(Debug, Clone, PartialEq)]
pub struct Attempt {
    /// The public score, or `None` when the solution failed to produce a
    /// valid output (a "buggy" node in AIDE terms).
    pub score: Option<Score>,
    /// The start of the solution's output, for the agent to read.
    pub feedback: String,
    /// Wall-clock time the run took, charged to the inner budget.
    pub wall: Duration,
}

/// A task as the inner loop sees it: public data and a public scorer.
pub trait PublicTask {
    /// The adapter's error type (infrastructure failures only).
    type Error;

    /// The task this scores.
    fn id(&self) -> &TaskId;

    /// The task's starting solution (`x0` in AIDE²'s Algorithm 1).
    fn baseline(&self) -> &Solution;

    /// What the agent is told about the task: goal, data formats, the
    /// solution contract and the metric. Public by construction.
    fn description(&self) -> &str;

    /// Runs `solution` on the public split with `seed` and scores it.
    ///
    /// # Errors
    /// Only when the run could not happen (sandbox or I/O failure). A
    /// solution that crashes or prints nonsense is an [`Attempt`] with no
    /// score.
    fn public_score(&self, solution: &Solution, seed: Seed) -> Result<Attempt, Self::Error>;
}

/// Grades a chosen solution on held-out data the agents never see.
pub trait PrivateGrader {
    /// The adapter's error type (infrastructure failures only).
    type Error;

    /// The private score of `solution` on `task`, or the task's floor when
    /// there is no solution or it fails on the private split.
    ///
    /// # Errors
    /// Only when grading could not happen.
    fn private_score(
        &self,
        task: &TaskId,
        solution: Option<&Solution>,
        seed: Seed,
    ) -> Result<Score, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solution_validation() {
        assert_eq!(Solution::new("print(1)\n").map(|s| s.source().len()), Ok(9));
        for bad in [String::new(), "  \n".to_owned(), "a\0b".to_owned()] {
            assert!(Solution::new(bad.clone()).is_err(), "{bad:?}");
        }
        assert!(Solution::new("x".repeat(MAX_SOLUTION_BYTES)).is_ok());
        assert!(Solution::new("x".repeat(MAX_SOLUTION_BYTES + 1)).is_err());
    }
}
