//! `rsi report`: a run's summary, and its replay (ADR-0005 §6).
//!
//! - [`summary`] renders the lineage as Markdown: one row per candidate,
//!   with its verdict, grades, the delta over the incumbent and its cost.
//! - [`replay`] re-derives the run's evidence at two levels. *Grade
//!   replay* re-runs every stored submission through the private grader
//!   and requires each private score bit for bit. *Trajectory replay*
//!   rebuilds each graded candidate from its commit and re-runs it against
//!   its recorded broker transcript, without the model, requiring the same
//!   submission.

use std::fmt::Write;

use rsi_core::{
    ChatModel, CostUsage, Decision, Grade, LineageEntry, PrivateGrader, PublicTask, Solution,
    Verdict,
};

use crate::error::RuntimeError;
use crate::grading::SandboxedGrader;
use crate::harness::HarnessProcess;
use crate::lineage_store::Blobs;
use crate::outer::{Lab, RunInfo};

/// How much of its run a lineage holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// The baseline and every configured proposal.
    Complete,
    /// A valid history from the baseline that stops early: the run failed
    /// part-way, or its last entries are missing. (The hash chain detects
    /// an edit or a gap, not a cut-off tail; `run.json`'s step count does.)
    Incomplete {
        /// Candidates recorded.
        recorded: usize,
        /// Candidates the run was configured for.
        expected: usize,
    },
}

/// Checks that `entries` are a run's history from its baseline: candidate
/// 0 is the baseline, candidates follow in order, and there are no more
/// than configured.
///
/// # Errors
/// [`RuntimeError::Lineage`] when there is no baseline (so nothing to
/// report or reproduce), the numbering is broken, or there are too many.
pub fn check(info: &RunInfo, entries: &[LineageEntry]) -> Result<RunStatus, RuntimeError> {
    let bad = |what: String| RuntimeError::Lineage(format!("run `{}`: {what}", info.name));
    match entries.first().map(|e| &e.fields().decision) {
        Some(Decision::Baseline) => {}
        Some(_) => return Err(bad("the first entry is not the baseline".into())),
        None => {
            return Err(bad(
                "no baseline was recorded, so there is nothing to report or reproduce".into(),
            ));
        }
    }
    if let Some((index, entry)) = entries
        .iter()
        .enumerate()
        .find(|(index, e)| e.fields().candidate.get() != *index as u64)
    {
        return Err(bad(format!(
            "entry {index} is candidate {}, out of order",
            entry.fields().candidate.get()
        )));
    }
    let expected = info.steps as usize + 1;
    match entries.len() {
        n if n == expected => Ok(RunStatus::Complete),
        n if n < expected => Ok(RunStatus::Incomplete {
            recorded: n,
            expected,
        }),
        n => Err(bad(format!(
            "{n} candidates, but the run was configured for {expected}"
        ))),
    }
}

fn grade_text(grade: Option<Grade>) -> String {
    grade.map_or_else(|| "–".to_owned(), |g| format!("{:.4}", g.get()))
}

/// The run as a Markdown report, saying plainly whether it is complete.
///
/// # Errors
/// What [`check`] rejects.
pub fn summary(info: &RunInfo, entries: &[LineageEntry]) -> Result<String, RuntimeError> {
    let status = check(info, entries)?;
    let mut out = String::new();
    let accepted = entries
        .iter()
        .filter(|e| matches!(e.fields().decision, Decision::Accepted { .. }))
        .count();
    let mut cost = CostUsage::default();
    let mut incumbent = (0, None);
    let mut rows = String::new();
    for entry in entries {
        let f = entry.fields();
        let grades = f
            .evaluations
            .iter()
            .map(|round| round.evaluation().map(|e| e.grade()))
            .collect::<Result<Vec<_>, _>>()?;
        let round_cost = f
            .evaluations
            .iter()
            .fold(CostUsage::default(), |total, round| {
                total.plus(round.cost())
            });
        cost = cost.plus(round_cost);
        let delta = f
            .decision
            .delta()
            .map_or_else(|| "–".to_owned(), |d| format!("{d:+.4}"));
        match &f.decision {
            Decision::Baseline => incumbent = (f.candidate.get(), grades.first().copied()),
            Decision::Accepted { fresh, .. } => incumbent = (f.candidate.get(), Some(*fresh)),
            Decision::Rejected(_) => {}
        }
        let parent = f
            .parent
            .map_or_else(|| "–".to_owned(), |p| p.get().to_string());
        let _ = writeln!(
            rows,
            "| {} | {parent} | {} | {} | {} | {delta} | {} | {:.1} s | `{}` |",
            f.candidate.get(),
            Verdict::of(&f.decision).name(),
            grade_text(grades.first().copied()),
            grade_text(grades.get(1).copied()),
            round_cost.tokens(),
            round_cost.wall.as_secs_f64(),
            &f.harness_commit.as_str()[..12.min(f.harness_commit.as_str().len())],
        );
    }
    let _ = writeln!(out, "# rsi run `{}`\n", info.name);
    let _ = writeln!(
        out,
        "- **Candidates:** {} (baseline + {} proposals), {accepted} accepted",
        entries.len(),
        entries.len().saturating_sub(1)
    );
    let _ = writeln!(
        out,
        "- **Incumbent:** candidate {}, grade {}",
        incumbent.0,
        grade_text(incumbent.1)
    );
    let _ = writeln!(out, "- **Accept margin:** {:.4}", info.margin.get());
    let _ = writeln!(out, "- **Tasks:** {}", info.tasks.join(", "));
    let _ = writeln!(
        out,
        "- **Inner cost:** {} tokens, {:.1} s",
        cost.tokens(),
        cost.wall.as_secs_f64()
    );
    let _ = writeln!(out, "- **Lineage:** hash chain verified");
    match status {
        RunStatus::Complete => {
            let _ = writeln!(out, "- **Status:** complete\n");
        }
        RunStatus::Incomplete { recorded, expected } => {
            let _ = writeln!(
                out,
                "- **Status:** INCOMPLETE, {recorded} of {expected} candidates recorded \
                 (the run stopped early, or its last entries are missing)\n"
            );
        }
    }
    out.push_str(
        "| # | parent | verdict | grade | fresh grade | Δ incumbent | tokens | wall | commit |\n\
         |---|---|---|---|---|---|---|---|---|\n",
    );
    out.push_str(&rows);
    for entry in entries {
        let f = entry.fields();
        if let Decision::Rejected(rsi_core::Rejection::PathViolation { paths }) = &f.decision {
            let _ = writeln!(
                out,
                "\nCandidate {} changed {}.",
                f.candidate.get(),
                paths.join(", ")
            );
        }
    }
    Ok(out)
}

/// What [`replay`] checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replayed {
    /// Private scores re-derived.
    pub grades: usize,
    /// Inner runs replayed from their transcripts.
    pub trajectories: usize,
    /// Every disagreement found; empty when the run reproduces.
    pub mismatches: Vec<String>,
}

/// Replays the run's grades and trajectories.
///
/// # Errors
/// What [`check`] rejects (a history without a baseline proves nothing),
/// infrastructure failures, and [`RuntimeError::Lineage`] for a missing or
/// altered blob. Disagreements are reported in [`Replayed::mismatches`].
pub fn replay<M>(
    lab: &Lab<'_, M>,
    info: &RunInfo,
    entries: &[LineageEntry],
    blobs: &Blobs,
) -> Result<Replayed, RuntimeError>
where
    M: ChatModel,
    M::Error: std::fmt::Display,
{
    check(info, entries)?;
    let mut replayed = Replayed::default();
    let grader = SandboxedGrader::new(lab.tasks, lab.runner, lab.grader.clone());
    for entry in entries {
        let f = entry.fields();
        if f.evaluations.is_empty() {
            continue;
        }
        let tag = format!("replay-{}", f.candidate.get());
        let worktree = lab.checkout(&f.harness_commit, &tag)?;
        let built = lab.build(&worktree, &tag);
        worktree.remove()?;
        let binary = built?.map_err(|failure| {
            RuntimeError::Lineage(format!(
                "candidate {} no longer builds: {}",
                f.candidate.get(),
                failure.0
            ))
        })?;
        let process = HarnessProcess::new(
            lab.executor,
            binary,
            &lab.scratch.join("agent"),
            &lab.protected,
            info.grace,
        )?;
        for result in f.evaluations.iter().flat_map(|round| &round.results) {
            let task = lab
                .tasks
                .iter()
                .find(|t| t.manifest().id == result.task)
                .ok_or_else(|| {
                    RuntimeError::Lineage(format!("unknown task `{}`", result.task.as_str()))
                })?;
            let solution = result
                .solution
                .as_ref()
                .map(|id| {
                    let bytes = blobs.get(id)?;
                    let text = String::from_utf8(bytes)
                        .map_err(|_| RuntimeError::Lineage(format!("blob {id} is not UTF-8")))?;
                    Ok::<_, RuntimeError>(Solution::new(text)?)
                })
                .transpose()?;
            let where_ = format!(
                "candidate {} task `{}` seed {}",
                f.candidate.get(),
                result.task.as_str(),
                result.seed.get()
            );
            let private = grader.private_score(&result.task, solution.as_ref(), result.seed)?;
            replayed.grades += 1;
            if private.get().to_bits() != result.private.get().to_bits() {
                replayed.mismatches.push(format!(
                    "{where_}: private score {} replays as {}",
                    result.private.get(),
                    private.get()
                ));
            }
            let Some(transcript) = &result.transcript else {
                continue;
            };
            let local = crate::grading::LocalTask::new(task, lab.runner);
            let again = process.replay(
                local.description(),
                result.seed,
                f.budget.wall(),
                &blobs.get(transcript)?,
            );
            replayed.trajectories += 1;
            match again {
                Ok(submission) if submission == solution => {}
                Ok(_) => replayed
                    .mismatches
                    .push(format!("{where_}: the replayed submission differs")),
                Err(error) => replayed
                    .mismatches
                    .push(format!("{where_}: trajectory replay failed: {error}")),
            }
        }
    }
    Ok(replayed)
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use rsi_core::{
        Budget, CandidateId, CommitSha, CostUsage, EntryFields, EvaluationRecord, Margin, ModelId,
        Rejection, Score, Seed, TaskId, TaskResult,
    };

    use super::*;

    fn info(steps: u32) -> RunInfo {
        RunInfo {
            name: "r".into(),
            steps,
            margin: Margin::new(0.1).expect("margin"),
            grace: Duration::from_secs(1),
            tasks: vec!["t".into()],
        }
    }

    fn entry(candidate: u64, decision: Decision) -> LineageEntry {
        let graded = matches!(decision, Decision::Baseline);
        LineageEntry::new(EntryFields {
            candidate: CandidateId::new(candidate),
            parent: candidate.checked_sub(1).map(|_| CandidateId::BASELINE),
            harness_commit: CommitSha::parse(&"c".repeat(40)).expect("sha"),
            diff: None,
            inner_model: ModelId::parse("m").expect("model"),
            outer_model: None,
            budget: Budget::new(1, Duration::from_secs(1), None).expect("budget"),
            evaluations: if graded {
                vec![EvaluationRecord {
                    round: 0,
                    results: vec![TaskResult {
                        task: TaskId::parse("t").expect("task"),
                        seed: Seed::new(1),
                        public: None,
                        private: Score::new(0.5).expect("score"),
                        solution: None,
                        transcript: None,
                        cost: CostUsage::default(),
                    }],
                }]
            } else {
                vec![]
            },
            decision,
            host: "h".into(),
        })
        .expect("valid")
    }

    fn buggy(candidate: u64) -> LineageEntry {
        entry(
            candidate,
            Decision::Rejected(Rejection::Buggy {
                reason: "no".into(),
            }),
        )
    }

    #[test]
    fn a_run_without_a_baseline_reports_and_reproduces_nothing() {
        let error = check(&info(2), &[]).expect_err("empty");
        assert!(error.to_string().contains("no baseline"), "{error}");
        assert!(summary(&info(2), &[]).is_err());
        assert!(
            check(&info(2), &[buggy(0)]).is_err(),
            "first entry not a baseline"
        );
    }

    #[test]
    fn a_partial_history_is_flagged_incomplete() {
        let partial = [entry(0, Decision::Baseline), buggy(1)];
        assert_eq!(
            check(&info(3), &partial).expect("valid"),
            RunStatus::Incomplete {
                recorded: 2,
                expected: 4
            }
        );
        let report = summary(&info(3), &partial).expect("inspectable");
        assert!(report.contains("INCOMPLETE, 2 of 4"), "{report}");
        assert_eq!(
            check(&info(1), &partial).expect("valid"),
            RunStatus::Complete
        );
        assert!(summary(&info(1), &partial)
            .expect("ok")
            .contains("**Status:** complete"));
        assert!(check(&info(0), &partial).is_err(), "more than configured");
        let gap = [entry(0, Decision::Baseline), buggy(2)];
        assert!(check(&info(3), &gap).is_err(), "out of order");
    }
}
