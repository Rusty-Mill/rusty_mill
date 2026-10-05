//! The outer loop's ports: the proposer that rewrites the harness and the
//! store that records every candidate (ADR-0005 §6, §7).
//!
//! The proposer sees the run's history only as [`Precedent`]s: verdicts,
//! grades and public scores. A [`LineageEntry`] also holds every task's
//! private score, so the proposer never receives one; [`precedents`] is the
//! only way to build its input, and it drops them by construction.

use std::path::Path;

use crate::accept::{Decision, Rejection};
use crate::budget::CostUsage;
use crate::lineage::{CandidateId, Digest, LineageEntry, ModelId, TaskId};
use crate::rng::Seed;
use crate::score::{Grade, Score};

/// How a candidate fared, without the evidence behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The starting harness.
    Baseline,
    /// Became the incumbent.
    Accepted,
    /// Its first grade did not beat the incumbent.
    NotBetter,
    /// Its fresh grade did not beat the incumbent by the margin.
    WithinNoise,
    /// It did not build.
    Buggy,
    /// It changed files outside the harness source.
    PathViolation,
}

impl Verdict {
    /// The verdict of a recorded decision.
    #[must_use]
    pub const fn of(decision: &Decision) -> Self {
        match decision {
            Decision::Baseline => Self::Baseline,
            Decision::Accepted { .. } => Self::Accepted,
            Decision::Rejected(Rejection::NotBetter { .. }) => Self::NotBetter,
            Decision::Rejected(Rejection::WithinNoise { .. }) => Self::WithinNoise,
            Decision::Rejected(Rejection::Buggy { .. }) => Self::Buggy,
            Decision::Rejected(Rejection::PathViolation { .. }) => Self::PathViolation,
        }
    }

    /// A lowercase name for reports and prompts.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Accepted => "accepted",
            Self::NotBetter => "not better",
            Self::WithinNoise => "within noise",
            Self::Buggy => "buggy",
            Self::PathViolation => "path violation",
        }
    }
}

/// One public score the proposer may see: a task, a seed, and how the
/// harness's submission scored on the public split.
#[derive(Debug, Clone, PartialEq)]
pub struct PublicResult {
    /// The task.
    pub task: TaskId,
    /// The seed of the inner run.
    pub seed: Seed,
    /// The submission's public score; `None` when there was none.
    pub public: Option<Score>,
}

/// A past candidate as the proposer sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Precedent {
    /// The candidate.
    pub candidate: CandidateId,
    /// The candidate it was derived from.
    pub parent: Option<CandidateId>,
    /// How it fared.
    pub verdict: Verdict,
    /// Its first-round grade (an aggregate over the suite), if graded.
    pub grade: Option<Grade>,
    /// The build failure or the offending paths, if any.
    pub note: Option<String>,
    /// Public scores from its first round.
    pub public: Vec<PublicResult>,
}

/// The run's history in the only form a proposer receives: grades and
/// public scores, never a per-task private score.
///
/// # Errors
/// Only if a recorded evaluation cannot be graded, which a validated
/// [`LineageEntry`] rules out.
pub fn precedents(entries: &[LineageEntry]) -> Result<Vec<Precedent>, crate::CoreError> {
    entries
        .iter()
        .map(|entry| {
            let fields = entry.fields();
            let first = fields.evaluations.first();
            let grade = first
                .map(|round| round.evaluation().map(|e| e.grade()))
                .transpose()?;
            let note = match &fields.decision {
                Decision::Rejected(Rejection::Buggy { reason }) => Some(reason.clone()),
                Decision::Rejected(Rejection::PathViolation { paths }) => Some(paths.join(", ")),
                _ => None,
            };
            let public = first
                .map(|round| {
                    round
                        .results
                        .iter()
                        .map(|r| PublicResult {
                            task: r.task.clone(),
                            seed: r.seed,
                            public: r.public,
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(Precedent {
                candidate: fields.candidate,
                parent: fields.parent,
                verdict: Verdict::of(&fields.decision),
                grade,
                note,
                public,
            })
        })
        .collect()
}

/// What a proposer did to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// A one-line description, used as the commit message.
    pub summary: String,
    /// Tokens the outer model consumed. The outer loop measures the wall
    /// time itself and records both in lineage.
    pub usage: CostUsage,
}

/// Rewrites the harness in a workspace.
pub trait Proposer {
    /// The adapter's error type.
    type Error;

    /// The outer model, recorded in lineage; `None` for a scripted one.
    fn model(&self) -> Option<&ModelId>;

    /// Edits the harness checked out at `workspace` (a repository work
    /// tree), given the run's `history`. The outer loop then checks which
    /// paths changed; only the harness source may.
    ///
    /// # Errors
    /// When the outer model could not be reached or the workspace could not
    /// be written. A bad edit is not an error: it is graded or rejected.
    fn propose(&self, history: &[Precedent], workspace: &Path) -> Result<Proposal, Self::Error>;
}

/// The append-only record of a run.
pub trait LineageStore {
    /// The adapter's error type.
    type Error;

    /// Appends `entry`; returns its chain hash. There is no update or
    /// delete.
    ///
    /// # Errors
    /// When the entry cannot be written.
    fn append(&mut self, entry: &LineageEntry) -> Result<Digest, Self::Error>;

    /// Every entry in order, after verifying the hash chain.
    ///
    /// # Errors
    /// When the chain is broken or an entry is malformed or inconsistent.
    fn entries(&self) -> Result<Vec<LineageEntry>, Self::Error>;
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::*;
    use crate::budget::Budget;
    use crate::lineage::{CommitSha, EntryFields, EvaluationRecord, TaskResult};

    fn entry(decision: Decision, results: Vec<(&str, f64, f64)>) -> LineageEntry {
        let evaluations = if results.is_empty() {
            vec![]
        } else {
            vec![EvaluationRecord {
                round: 0,
                results: results
                    .into_iter()
                    .map(|(task, public, private)| TaskResult {
                        task: TaskId::parse(task).expect("task"),
                        seed: Seed::new(3),
                        public: Some(Score::new(public).expect("score")),
                        private: Score::new(private).expect("score"),
                        solution: None,
                        transcript: None,
                        cost: CostUsage::default(),
                    })
                    .collect(),
            }]
        };
        LineageEntry::new(EntryFields {
            candidate: CandidateId::BASELINE,
            parent: None,
            harness_commit: CommitSha::parse(&"a".repeat(40)).expect("sha"),
            diff: None,
            inner_model: ModelId::parse("m").expect("model"),
            outer_model: None,
            outer_cost: None,
            budget: Budget::new(1, Duration::from_secs(1), None).expect("budget"),
            evaluations,
            decision,
            host: "test".into(),
        })
        .expect("valid entry")
    }

    #[test]
    fn precedents_carry_grades_and_public_scores_but_no_private_score() {
        let graded = entry(
            Decision::Baseline,
            vec![("tsp", 0.25, 0.375), ("ml", 0.5, 0.625)],
        );
        let buggy = entry(
            Decision::Rejected(Rejection::Buggy {
                reason: "error[E0308]".into(),
            }),
            vec![],
        );
        let history = precedents(&[graded, buggy]).expect("precedents");
        assert_eq!(history[0].verdict, Verdict::Baseline);
        assert_eq!(
            history[0].grade.map(Grade::get),
            Some(0.5),
            "mean of 0.375, 0.625"
        );
        let public: Vec<f64> = history[0]
            .public
            .iter()
            .filter_map(|p| p.public.map(Score::get))
            .collect();
        assert_eq!(public, vec![0.25, 0.5]);
        // The type has no field that could hold a per-task private score;
        // its debug form shows only the aggregate grade.
        let shown = format!("{:?}", history[0]);
        assert!(
            !shown.contains("0.375") && !shown.contains("0.625"),
            "{shown}"
        );
        assert_eq!(history[1].verdict, Verdict::Buggy);
        assert_eq!(history[1].note.as_deref(), Some("error[E0308]"));
        assert_eq!(history[1].grade, None);
    }

    #[test]
    fn verdict_names() {
        assert_eq!(Verdict::PathViolation.name(), "path violation");
        assert_eq!(Verdict::of(&Decision::Baseline), Verdict::Baseline);
    }
}
