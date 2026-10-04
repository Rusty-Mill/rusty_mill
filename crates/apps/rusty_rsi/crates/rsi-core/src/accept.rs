//! The two-stage accept gate (ADR-0005 §5, invariant 3).
//!
//! 1. [`screen`]: a candidate whose first grade does not beat the incumbent
//!    is rejected outright.
//! 2. [`confirm`]: a candidate that does beat it is re-graded on fresh seeds
//!    and accepted only if that fresh grade beats the incumbent by more than
//!    the calibrated [`Margin`].
//!
//! Only the fresh grade is compared in stage 2. Reusing the first grade, the
//! one selected for being high, would bias the gate toward lucky candidates.
//! [`Challenger`] can only come from [`screen`], so stage 2 cannot run on a
//! candidate that never passed stage 1.

use crate::error::CoreError;
use crate::noise::Margin;
use crate::rng::Seed;
use crate::score::Grade;

/// A grade together with the seeds that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    grade: Grade,
    seeds: Vec<Seed>,
}

impl Evaluation {
    /// Pairs a grade with its seeds.
    ///
    /// # Errors
    /// [`CoreError::NoSeeds`] if `seeds` is empty: a grade nobody can
    /// reproduce is not evidence.
    pub fn new(grade: Grade, seeds: Vec<Seed>) -> Result<Self, CoreError> {
        if seeds.is_empty() {
            return Err(CoreError::NoSeeds);
        }
        Ok(Self { grade, seeds })
    }

    /// The grade.
    #[must_use]
    pub const fn grade(&self) -> Grade {
        self.grade
    }

    /// The seeds the grade was computed on.
    #[must_use]
    pub fn seeds(&self) -> &[Seed] {
        &self.seeds
    }
}

/// A candidate that beat the incumbent on its first evaluation and now
/// needs a fresh-seed re-evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct Challenger {
    first: Evaluation,
}

impl Challenger {
    /// The evaluation that let the candidate through stage 1.
    #[must_use]
    pub const fn first(&self) -> &Evaluation {
        &self.first
    }
}

/// The outcome of stage 1.
#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    /// Rejected without re-evaluation.
    Reject(Rejection),
    /// Beat the incumbent once; re-evaluate on fresh seeds.
    Reevaluate(Challenger),
}

/// Why a candidate was not accepted.
#[derive(Debug, Clone, PartialEq)]
pub enum Rejection {
    /// The first grade did not exceed the incumbent's.
    NotBetter {
        /// The first grade.
        first: Grade,
    },
    /// The fresh grade's lead over the incumbent was within the noise margin.
    WithinNoise {
        /// The fresh grade.
        fresh: Grade,
        /// `fresh - incumbent`.
        delta: f64,
        /// The margin it had to exceed.
        margin: Margin,
    },
    /// The candidate did not build or did not produce a grade.
    Buggy {
        /// What went wrong, for the outer agent to read.
        reason: String,
    },
    /// The proposed diff touched paths outside the mutable surface.
    PathViolation {
        /// The offending paths.
        paths: Vec<String>,
    },
}

/// The recorded verdict on a lineage entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// The initial agent `a0`, accepted by definition.
    Baseline,
    /// Became the new incumbent.
    Accepted {
        /// The fresh grade it was accepted on.
        fresh: Grade,
        /// `fresh - incumbent`.
        delta: f64,
        /// The margin it exceeded.
        margin: Margin,
    },
    /// Not accepted.
    Rejected(Rejection),
}

impl Decision {
    /// Whether this entry became (or started as) the incumbent.
    #[must_use]
    pub const fn is_incumbent(&self) -> bool {
        matches!(self, Self::Baseline | Self::Accepted { .. })
    }
}

/// Stage 1: lets a candidate through only if its first grade strictly beats
/// the incumbent's.
#[must_use]
pub fn screen(incumbent: Grade, first: Evaluation) -> Screen {
    if first.grade > incumbent {
        return Screen::Reevaluate(Challenger { first });
    }
    Screen::Reject(Rejection::NotBetter { first: first.grade })
}

/// Stage 2: accepts iff the fresh grade beats the incumbent by strictly more
/// than `margin`.
///
/// # Errors
/// [`CoreError::SeedReused`] if any fresh seed also appears in the first
/// evaluation; the re-evaluation would then not be independent.
pub fn confirm(
    incumbent: Grade,
    challenger: Challenger,
    fresh: Evaluation,
    margin: Margin,
) -> Result<Decision, CoreError> {
    if let Some(seed) = fresh
        .seeds
        .iter()
        .find(|seed| challenger.first.seeds.contains(seed))
    {
        return Err(CoreError::SeedReused(seed.get()));
    }
    let delta = fresh.grade.delta(incumbent);
    if delta > margin.get() {
        return Ok(Decision::Accepted {
            fresh: fresh.grade,
            delta,
            margin,
        });
    }
    Ok(Decision::Rejected(Rejection::WithinNoise {
        fresh: fresh.grade,
        delta,
        margin,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grade(value: f64) -> Grade {
        Grade::new(value).expect("valid grade")
    }

    fn eval(value: f64, seeds: &[u64]) -> Evaluation {
        Evaluation::new(grade(value), seeds.iter().copied().map(Seed::new).collect())
            .expect("has seeds")
    }

    fn margin(value: f64) -> Margin {
        Margin::new(value).expect("valid margin")
    }

    fn challenger(incumbent: f64, first: f64) -> Challenger {
        match screen(grade(incumbent), eval(first, &[1, 2])) {
            Screen::Reevaluate(challenger) => challenger,
            Screen::Reject(rejection) => panic!("expected re-evaluation, got {rejection:?}"),
        }
    }

    #[test]
    fn evaluation_requires_seeds() {
        assert_eq!(
            Evaluation::new(grade(0.5), Vec::new()),
            Err(CoreError::NoSeeds)
        );
    }

    #[test]
    fn screen_rejects_worse_and_equal() {
        for first in [0.4, 0.5] {
            assert_eq!(
                screen(grade(0.5), eval(first, &[1])),
                Screen::Reject(Rejection::NotBetter {
                    first: grade(first)
                })
            );
        }
    }

    #[test]
    fn screen_passes_strictly_better() {
        let challenger = challenger(0.5, 0.51);
        assert_eq!(challenger.first().grade(), grade(0.51));
    }

    #[test]
    fn confirm_accepts_lead_beyond_margin() {
        let decision = confirm(
            grade(0.5),
            challenger(0.5, 0.7),
            eval(0.6, &[3, 4]),
            margin(0.05),
        );
        match decision {
            Ok(Decision::Accepted {
                fresh,
                delta,
                margin: m,
            }) => {
                assert_eq!(fresh, grade(0.6));
                assert!((delta - 0.1).abs() < 1e-12);
                assert_eq!(m, margin(0.05));
            }
            other => panic!("expected acceptance, got {other:?}"),
        }
    }

    #[test]
    fn confirm_uses_fresh_grade_not_first() {
        // First grade 0.9 was lucky; fresh 0.52 is within a 0.05 margin.
        let decision = confirm(
            grade(0.5),
            challenger(0.5, 0.9),
            eval(0.52, &[3]),
            margin(0.05),
        );
        assert!(matches!(
            decision,
            Ok(Decision::Rejected(Rejection::WithinNoise { fresh, .. })) if fresh == grade(0.52)
        ));
    }

    #[test]
    fn lead_equal_to_margin_is_rejected() {
        // 0.75 - 0.5 = 0.25 exactly in binary floating point.
        let decision = confirm(
            grade(0.5),
            challenger(0.5, 0.8),
            eval(0.75, &[3]),
            margin(0.25),
        );
        assert!(matches!(
            decision,
            Ok(Decision::Rejected(Rejection::WithinNoise { .. }))
        ));
    }

    #[test]
    fn zero_margin_still_requires_strict_improvement() {
        let tie = confirm(
            grade(0.5),
            challenger(0.5, 0.8),
            eval(0.5, &[3]),
            Margin::ZERO,
        );
        assert!(matches!(tie, Ok(Decision::Rejected(_))));
        let win = confirm(
            grade(0.5),
            challenger(0.5, 0.8),
            eval(0.5001, &[3]),
            Margin::ZERO,
        );
        assert!(matches!(win, Ok(Decision::Accepted { .. })));
    }

    #[test]
    fn fresh_grade_below_incumbent_is_rejected() {
        let decision = confirm(
            grade(0.5),
            challenger(0.5, 0.8),
            eval(0.3, &[3]),
            Margin::ZERO,
        );
        assert!(matches!(
            decision,
            Ok(Decision::Rejected(Rejection::WithinNoise { delta, .. })) if delta < 0.0
        ));
    }

    #[test]
    fn reused_seed_is_an_error() {
        let decision = confirm(
            grade(0.5),
            challenger(0.5, 0.8),
            eval(0.9, &[7, 2]),
            Margin::ZERO,
        );
        assert_eq!(decision, Err(CoreError::SeedReused(2)));
    }

    #[test]
    fn incumbency() {
        assert!(Decision::Baseline.is_incumbent());
        assert!(Decision::Accepted {
            fresh: grade(0.6),
            delta: 0.1,
            margin: Margin::ZERO
        }
        .is_incumbent());
        assert!(!Decision::Rejected(Rejection::Buggy {
            reason: "rustc failed".into()
        })
        .is_incumbent());
    }
}
