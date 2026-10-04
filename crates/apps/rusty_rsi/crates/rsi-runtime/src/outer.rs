//! The outer loop and noise calibration (ADR-0005 §5, §7).
//!
//! For each step the loop:
//!
//! 1. takes the incumbent as the parent (AIDE²'s rule);
//! 2. checks it out in a fresh sparse worktree and lets the [`Proposer`]
//!    edit it, showing it only [`precedents`] (grades, never per-task
//!    private scores);
//! 3. stages and commits the edit under `refs/rsi/<run>/<step>`, and
//!    rejects it as a path violation if anything outside the harness source
//!    changed;
//! 4. builds it (a compile error is a buggy candidate), grades it on fresh
//!    seeds, gates it ([`screen`], then [`confirm`] on another fresh seed
//!    set) and appends the entry to the lineage.
//!
//! [`calibrate`] grades one harness on several disjoint seed sets to
//! measure the noise the accept margin has to beat.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rsi_core::{
    confirm, precedents, screen, seed_set, Budget, CandidateId, ChatModel, CommitSha, Decision,
    EntryFields, EvaluationRecord, Grade, Harness, LineageEntry, LineageStore, Margin, NoiseBand,
    PrivateGrader, Proposer, PublicTask, Rejection, Screen, Seed, TaskId, TaskResult,
};
use rusty_json::Value;

use crate::error::RuntimeError;
use crate::executor::ProcessExecutor;
use crate::git::{self, Repo, Worktree};
use crate::grading::{GraderCommand, LocalTask, SandboxedGrader, SolutionRunner};
use crate::harness::{build, HarnessBinary, HarnessProcess, SandboxedHarness, Toolchain};
use crate::lineage_store::{Blobs, JsonlLineage};
use crate::task_dir::TaskDir;

/// The run's configuration file inside its directory.
pub const RUN_FILE: &str = "run.json";

/// How much of a build failure is kept in lineage.
const BUILD_LOG_CHARS: usize = 4000;

/// How every harness is graded: the same for every candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Grading {
    /// The root of every seed in the run.
    pub run_seed: Seed,
    /// Seeds per grading round (each task runs once per seed).
    pub seeds_per_round: usize,
    /// The inner budget per task run.
    pub budget: Budget,
    /// Completion tokens asked for per inner model call.
    pub max_completion_tokens: u64,
    /// How long past its budget an agent may run before it is killed.
    pub grace: Duration,
}

/// Everything fixed for one run of the outer loop.
#[derive(Debug, Clone, PartialEq)]
pub struct RunConfig {
    /// The run's name: lowercase letters, digits, `-` and `_`. Candidate
    /// commits are kept under `refs/rsi/<name>/<step>`.
    pub name: String,
    /// The commit whose harness is candidate 0.
    pub base: CommitSha,
    /// Proposals to make after the baseline.
    pub steps: u32,
    /// The calibrated accept margin.
    pub margin: Margin,
    /// How candidates are graded.
    pub grading: Grading,
}

/// The machinery the loop drives.
#[derive(Debug)]
pub struct Lab<'a, M> {
    /// The repository holding the harness.
    pub repo: &'a Repo,
    /// The task suite.
    pub tasks: &'a [TaskDir],
    /// The sandboxed executor.
    pub executor: &'a ProcessExecutor,
    /// The compiler for candidate harnesses.
    pub toolchain: &'a Toolchain,
    /// Runs solutions in the sandbox.
    pub runner: &'a SolutionRunner<ProcessExecutor>,
    /// Starts the out-of-process private grader.
    pub grader: GraderCommand,
    /// The inner model.
    pub model: &'a M,
    /// Where worktrees, builds and agent work directories go.
    pub scratch: PathBuf,
    /// Paths no agent sandbox may reach (tasks, the run directory, the
    /// repository, the executor's state).
    pub protected: Vec<PathBuf>,
}

fn host() -> String {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let cpus = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    format!(
        "{}-{} {} {cpus} cpus",
        std::env::consts::OS,
        std::env::consts::ARCH,
        release.trim()
    )
}

impl<M> Lab<'_, M>
where
    M: ChatModel,
    M::Error: std::fmt::Display,
{
    fn fresh(&self, name: &str) -> Result<PathBuf, RuntimeError> {
        let path = self.scratch.join(name);
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .map_err(|e| RuntimeError::io(format!("clearing {}", path.display()), e))?;
        }
        std::fs::create_dir_all(&self.scratch)
            .map_err(|e| RuntimeError::io("creating scratch", e))?;
        Ok(path)
    }

    /// A sparse worktree of `commit`, named `tag` under the scratch dir.
    ///
    /// # Errors
    /// [`RuntimeError::Git`] if git refuses.
    pub fn checkout(&self, commit: &CommitSha, tag: &str) -> Result<Worktree, RuntimeError> {
        self.repo
            .worktree(&self.fresh(&format!("wt-{tag}"))?, commit)
    }

    /// Builds the harness in `worktree`.
    ///
    /// # Errors
    /// Infrastructure failures; a compile error is `Ok(Err(..))`.
    pub fn build(
        &self,
        worktree: &Worktree,
        tag: &str,
    ) -> Result<Result<HarnessBinary, crate::harness::BuildFailure>, RuntimeError> {
        let out = self.fresh(&format!("build-{tag}"))?;
        build(self.executor, self.toolchain, &worktree.harness(), &out)
    }

    /// Grades `binary` once per task and seed of round `round` of
    /// `candidate`, storing each submission and transcript in `blobs`.
    ///
    /// # Errors
    /// Infrastructure failures (sandbox, model, grader).
    pub fn evaluate(
        &self,
        binary: &HarnessBinary,
        candidate: u64,
        round: u32,
        grading: &Grading,
        blobs: &Blobs,
    ) -> Result<EvaluationRecord, RuntimeError> {
        let process = HarnessProcess::new(
            self.executor,
            binary.clone(),
            &self.scratch.join("agent"),
            &self.protected,
            grading.grace,
        )?;
        let harness = SandboxedHarness::new(process, self.model, grading.max_completion_tokens);
        let grader = SandboxedGrader::new(self.tasks, self.runner, self.grader.clone());
        let mut results = Vec::new();
        for seed in seed_set(grading.run_seed, candidate, round, grading.seeds_per_round) {
            for task in self.tasks {
                let public_task = LocalTask::new(task, self.runner);
                let outcome = harness.run(&public_task, &grading.budget, seed)?;
                let submission = outcome.submission.as_ref();
                let public = match submission {
                    Some(solution) => public_task.public_score(solution, seed, None)?.score,
                    None => None,
                };
                let id: &TaskId = &task.manifest().id;
                results.push(TaskResult {
                    task: id.clone(),
                    seed,
                    public,
                    private: grader.private_score(id, submission, seed)?,
                    solution: submission
                        .map(|s| blobs.put(s.source().as_bytes()))
                        .transpose()?,
                    transcript: Some(blobs.put(&outcome.transcript)?),
                    cost: outcome.usage,
                });
            }
        }
        Ok(EvaluationRecord { round, results })
    }
}

/// The result of [`calibrate`].
#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    /// The grades, one per round.
    pub grades: Vec<Grade>,
    /// Their spread.
    pub band: NoiseBand,
    /// The confidence factor used.
    pub z: f64,
    /// `z · √2 · σ̂`.
    pub margin: Margin,
}

/// Grades the harness at `commit` on `rounds` disjoint seed sets and
/// derives the accept margin from the spread.
///
/// # Errors
/// Infrastructure failures; [`RuntimeError::Core`] for fewer than two
/// rounds or a bad `z`; [`RuntimeError::Harness`] if the harness does not
/// build.
pub fn calibrate<M>(
    lab: &Lab<'_, M>,
    commit: &CommitSha,
    grading: &Grading,
    rounds: u32,
    z: f64,
    blobs: &Blobs,
) -> Result<Calibration, RuntimeError>
where
    M: ChatModel,
    M::Error: std::fmt::Display,
{
    let worktree = lab.checkout(commit, "calibrate")?;
    let built = lab.build(&worktree, "calibrate");
    worktree.remove()?;
    let binary = built?.map_err(|f| RuntimeError::Harness(f.0))?;
    let grades = (0..rounds)
        .map(|round| {
            lab.evaluate(&binary, 0, round, grading, blobs)
                .and_then(|record| Ok(record.evaluation()?.grade()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let band = NoiseBand::from_grades(&grades)?;
    let margin = band.margin(z)?;
    Ok(Calibration {
        grades,
        band,
        z,
        margin,
    })
}

/// The incumbent: the harness every new candidate is derived from.
#[derive(Debug, Clone)]
struct Incumbent {
    candidate: CandidateId,
    commit: CommitSha,
    grade: Grade,
}

/// Runs the outer loop into the fresh directory `run_dir`: the baseline,
/// then `config.steps` proposals. Returns the lineage.
///
/// # Errors
/// [`RuntimeError::Lineage`] if `run_dir` already holds a run; any
/// infrastructure failure, which ends the run (entries written so far
/// stay valid).
pub fn run<M, P>(
    lab: &Lab<'_, M>,
    proposer: &P,
    config: &RunConfig,
    run_dir: &Path,
) -> Result<Vec<LineageEntry>, RuntimeError>
where
    M: ChatModel,
    M::Error: std::fmt::Display,
    P: Proposer<Error = RuntimeError>,
{
    TaskId::parse(&config.name).map_err(|_| {
        RuntimeError::Lineage(format!(
            "run name `{}` must be lowercase letters, digits, `-` or `_`",
            config.name
        ))
    })?;
    if run_dir.join(RUN_FILE).exists() {
        return Err(RuntimeError::Lineage(format!(
            "{} already holds a run",
            run_dir.display()
        )));
    }
    std::fs::create_dir_all(run_dir).map_err(|e| RuntimeError::io("creating the run dir", e))?;
    // Claim the run's ref namespace before writing anything: two run
    // directories with the same name must not share (and overwrite)
    // refs/rsi/<name>/*.
    lab.repo
        .create_ref(&format!("refs/rsi/{}/base", config.name), &config.base)
        .map_err(|e| {
            RuntimeError::Lineage(format!(
                "run name `{}` is already used in this repository; \
                 choose another run directory name ({e})",
                config.name
            ))
        })?;
    std::fs::write(
        run_dir.join(RUN_FILE),
        encode_run(config, lab, proposer).to_json_string_pretty(),
    )
    .map_err(|e| RuntimeError::io("writing run.json", e))?;
    let mut store = JsonlLineage::create(run_dir)?;
    let blobs = Blobs::open(run_dir)?;
    let host = host();
    let fields = |candidate: u64, parent: Option<CandidateId>, commit: CommitSha| EntryFields {
        candidate: CandidateId::new(candidate),
        parent,
        harness_commit: commit,
        diff: None,
        inner_model: lab.model.id().clone(),
        outer_model: proposer.model().cloned(),
        budget: config.grading.budget,
        evaluations: Vec::new(),
        decision: Decision::Baseline,
        host: host.clone(),
    };

    // Candidate 0: the harness at the base commit.
    let worktree = lab.checkout(&config.base, "0")?;
    let built = lab.build(&worktree, "0");
    worktree.remove()?;
    let binary = built?.map_err(|f| RuntimeError::Harness(f.0))?;
    let record = lab.evaluate(&binary, 0, 0, &config.grading, &blobs)?;
    let mut incumbent = Incumbent {
        candidate: CandidateId::BASELINE,
        commit: config.base.clone(),
        grade: record.evaluation()?.grade(),
    };
    let mut baseline = fields(0, None, config.base.clone());
    baseline.evaluations = vec![record];
    store.append(&LineageEntry::new(baseline)?)?;

    for step in 1..=u64::from(config.steps) {
        let history = store.entries()?;
        let mut entry = fields(step, Some(incumbent.candidate), incumbent.commit.clone());
        let tag = step.to_string();
        let worktree = lab.checkout(&incumbent.commit, &tag)?;
        let proposed = propose_and_commit(lab, proposer, &history, &worktree, config, step, &blobs);
        let removal = worktree.remove();
        let Committed {
            commit,
            diff,
            violations,
        } = proposed?;
        removal?;
        entry.harness_commit = commit.clone();
        entry.diff = Some(diff);
        if violations.is_empty() {
            gate(lab, config, &blobs, step, &commit, &incumbent, &mut entry)?;
        } else {
            entry.decision = Decision::Rejected(Rejection::PathViolation { paths: violations });
        }
        if let Decision::Accepted { fresh, .. } = entry.decision {
            incumbent = Incumbent {
                candidate: entry.candidate,
                commit: commit.clone(),
                grade: fresh,
            };
        }
        store.append(&LineageEntry::new(entry)?)?;
    }
    store.entries()
}

/// A committed proposal.
struct Committed {
    commit: CommitSha,
    diff: rsi_core::BlobId,
    /// The changed paths the allowlist forbids; empty when it passes.
    violations: Vec<String>,
}

/// Lets the proposer edit `worktree`, then stages, records and commits the
/// edit.
fn propose_and_commit<M, P>(
    lab: &Lab<'_, M>,
    proposer: &P,
    history: &[LineageEntry],
    worktree: &Worktree,
    config: &RunConfig,
    step: u64,
    blobs: &Blobs,
) -> Result<Committed, RuntimeError>
where
    P: Proposer<Error = RuntimeError>,
{
    let proposal = proposer.propose(&precedents(history)?, worktree.path())?;
    let staged = worktree.stage()?;
    let diff = blobs.put(&staged.diff)?;
    let summary = if proposal.summary.trim().is_empty() {
        format!("rsi {} candidate {step}", config.name)
    } else {
        proposal.summary
    };
    let commit = worktree.commit(&summary)?;
    lab.repo
        .create_ref(&format!("refs/rsi/{}/{step}", config.name), &commit)?;
    Ok(Committed {
        commit,
        diff,
        violations: git::violations(&staged.changes),
    })
}

/// Builds and grades a candidate that passed the allowlist, filling in its
/// evaluations and decision.
fn gate<M>(
    lab: &Lab<'_, M>,
    config: &RunConfig,
    blobs: &Blobs,
    step: u64,
    commit: &CommitSha,
    incumbent: &Incumbent,
    entry: &mut EntryFields,
) -> Result<(), RuntimeError>
where
    M: ChatModel,
    M::Error: std::fmt::Display,
{
    let tag = step.to_string();
    let worktree = lab.checkout(commit, &format!("{tag}-build"))?;
    let built = lab.build(&worktree, &tag);
    worktree.remove()?;
    let binary = match built? {
        Ok(binary) => binary,
        Err(failure) => {
            let reason: String = failure.0.chars().take(BUILD_LOG_CHARS).collect();
            entry.decision = Decision::Rejected(Rejection::Buggy { reason });
            return Ok(());
        }
    };
    let first = lab.evaluate(&binary, step, 0, &config.grading, blobs)?;
    let challenger = match screen(incumbent.grade, first.evaluation()?) {
        Screen::Reject(rejection) => {
            entry.evaluations = vec![first];
            entry.decision = Decision::Rejected(rejection);
            return Ok(());
        }
        Screen::Reevaluate(challenger) => challenger,
    };
    let fresh = lab.evaluate(&binary, step, 1, &config.grading, blobs)?;
    entry.decision = confirm(
        incumbent.grade,
        challenger,
        fresh.evaluation()?,
        config.margin,
    )?;
    entry.evaluations = vec![first, fresh];
    Ok(())
}

fn exact(value: f64) -> Value {
    Value::from(value.to_string())
}

fn nanos(duration: Duration) -> Value {
    Value::from(u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX))
}

fn encode_run<M: ChatModel, P: Proposer>(
    config: &RunConfig,
    lab: &Lab<'_, M>,
    proposer: &P,
) -> Value {
    let mut grading = Value::object();
    let g = &config.grading;
    grading.insert("run_seed", g.run_seed.get().to_string());
    grading.insert("seeds_per_round", g.seeds_per_round as u64);
    grading.insert("tokens", g.budget.tokens());
    grading.insert("wall_ns", nanos(g.budget.wall()));
    grading.insert("max_completion_tokens", g.max_completion_tokens);
    grading.insert("grace_ns", nanos(g.grace));
    let mut out = Value::object();
    out.insert("name", config.name.as_str());
    out.insert("base", config.base.as_str());
    out.insert("steps", u64::from(config.steps));
    out.insert("margin", exact(config.margin.get()));
    out.insert("grading", grading);
    out.insert("inner_model", lab.model.id().as_str());
    out.insert(
        "outer_model",
        proposer
            .model()
            .map_or(Value::Null, |m| Value::from(m.as_str())),
    );
    out.insert(
        "tasks",
        Value::from(
            lab.tasks
                .iter()
                .map(|t| Value::from(t.manifest().id.as_str()))
                .collect::<Vec<_>>(),
        ),
    );
    out
}

/// The parts of `run.json` that report and replay need.
#[derive(Debug, Clone, PartialEq)]
pub struct RunInfo {
    /// The run's name.
    pub name: String,
    /// Proposals the run was configured to make after the baseline.
    pub steps: u32,
    /// The accept margin.
    pub margin: Margin,
    /// The kill grace period.
    pub grace: Duration,
    /// The task ids, in order.
    pub tasks: Vec<String>,
}

/// Reads `run.json` from `run_dir`.
///
/// # Errors
/// [`RuntimeError::Lineage`] when it is missing or malformed.
pub fn read_run(run_dir: &Path) -> Result<RunInfo, RuntimeError> {
    let bad = |what: &str| RuntimeError::Lineage(format!("run.json: {what}"));
    let text = std::fs::read_to_string(run_dir.join(RUN_FILE)).map_err(|e| bad(&e.to_string()))?;
    let json = Value::parse(&text).map_err(|e| bad(&e.to_string()))?;
    let margin = json
        .get("margin")
        .and_then(Value::as_str)
        .and_then(|m| m.parse::<f64>().ok())
        .ok_or_else(|| bad("`margin` is missing"))?;
    let steps = json
        .get("steps")
        .and_then(Value::as_u64)
        .and_then(|s| u32::try_from(s).ok())
        .ok_or_else(|| bad("`steps` is missing"))?;
    let grace = json
        .pointer("/grading/grace_ns")
        .and_then(Value::as_u64)
        .ok_or_else(|| bad("`grading.grace_ns` is missing"))?;
    Ok(RunInfo {
        name: json
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("`name` is missing"))?
            .to_owned(),
        steps,
        margin: Margin::new(margin)?,
        grace: Duration::from_nanos(grace),
        tasks: json
            .get("tasks")
            .and_then(Value::as_array)
            .map(|tasks| {
                tasks
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The JSON form of a calibration, as `rsi calibrate` writes it.
#[must_use]
pub fn encode_calibration(calibration: &Calibration) -> Value {
    let band = &calibration.band;
    let mut out = Value::object();
    out.insert("margin", exact(calibration.margin.get()));
    out.insert("z", exact(calibration.z));
    out.insert("samples", band.samples() as u64);
    out.insert("mean", exact(band.mean()));
    out.insert("std_dev", exact(band.std_dev()));
    out.insert("min", exact(band.min()));
    out.insert("max", exact(band.max()));
    out.insert(
        "grades",
        Value::from(
            calibration
                .grades
                .iter()
                .map(|g| exact(g.get()))
                .collect::<Vec<_>>(),
        ),
    );
    out
}

/// The margin in a calibration file written by [`encode_calibration`].
///
/// # Errors
/// [`RuntimeError::Lineage`] when the file has no valid margin.
pub fn calibrated_margin(text: &str) -> Result<Margin, RuntimeError> {
    let json =
        Value::parse(text).map_err(|e| RuntimeError::Lineage(format!("calibration: {e}")))?;
    let margin = json
        .get("margin")
        .and_then(Value::as_str)
        .and_then(|m| m.parse::<f64>().ok())
        .ok_or_else(|| RuntimeError::Lineage("calibration: no `margin`".into()))?;
    Ok(Margin::new(margin)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_files_round_trip_the_margin() {
        let grades: Vec<Grade> = [0.5, 0.25, 0.375]
            .iter()
            .map(|&g| Grade::new(g).expect("grade"))
            .collect();
        let band = NoiseBand::from_grades(&grades).expect("band");
        let calibration = Calibration {
            margin: band.margin(1.645).expect("margin"),
            grades,
            band,
            z: 1.645,
        };
        let text = encode_calibration(&calibration).to_json_string();
        assert_eq!(
            calibrated_margin(&text).expect("margin"),
            calibration.margin
        );
        assert!(calibrated_margin("{}").is_err());
        assert!(calibrated_margin("{\"margin\":\"-1\"}").is_err());
    }
}
