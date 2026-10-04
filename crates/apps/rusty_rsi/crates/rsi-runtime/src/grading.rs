//! Running and grading solutions (ADR-0005 §4, invariant 1).
//!
//! - [`SolutionRunner`] stages a fresh working directory and runs a
//!   solution through the sandboxed [`Executor`]. Before every run it proves
//!   the task directory, and so its private data, is unreachable.
//! - [`LocalTask`] is the [`PublicTask`] the inner loop holds: public split
//!   only, scored in-process.
//! - [`SandboxedGrader`] is the [`PrivateGrader`]: it runs the chosen
//!   solution on private inputs in a fresh sandbox, then hands the output to
//!   a separate grader process (`rsi __grade`), the only process that reads
//!   private labels.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rsi_core::{
    Attempt, ExecOutcome, Executor, PrivateGrader, PublicTask, SandboxSpec, Score, Seed, Solution,
    TaskId, Termination,
};

use crate::error::RuntimeError;
use crate::output::{read_capped, read_untrusted, Rejected};
use crate::task_dir::{Split, TaskDir, OUTPUT_FILE};

/// Bytes of solution output shown to the inner agent.
pub const FEEDBACK_BYTES: usize = 4096;

/// System directories a Python solution needs to read and execute.
pub const SYSTEM_READ_ROOTS: [&str; 4] = ["/usr", "/lib", "/lib64", "/bin"];

static NEXT_WORK_DIR: AtomicU64 = AtomicU64::new(0);

/// What one sandboxed solution run produced.
#[derive(Debug)]
struct Run {
    outcome: ExecOutcome,
    /// The snapshot of `output.txt`, read once by [`read_untrusted`]; every
    /// later step uses these bytes and never reopens the path.
    output: Result<Vec<u8>, Rejected>,
    work: PathBuf,
}

/// Stages and runs solutions in fresh sandboxed working directories.
#[derive(Debug)]
pub struct SolutionRunner<E> {
    executor: E,
    read_roots: Vec<PathBuf>,
    scratch: PathBuf,
}

impl<E: Executor<Error = RuntimeError>> SolutionRunner<E> {
    /// A runner whose sandboxes may read `read_roots` (those that exist) and
    /// write one fresh directory under `scratch`.
    ///
    /// # Errors
    /// [`RuntimeError::Io`] when `scratch` cannot be created or a root
    /// cannot be resolved.
    pub fn new(executor: E, read_roots: &[PathBuf], scratch: &Path) -> Result<Self, RuntimeError> {
        std::fs::create_dir_all(scratch).map_err(|e| RuntimeError::io("creating scratch", e))?;
        let scratch = scratch
            .canonicalize()
            .map_err(|e| RuntimeError::io("resolving scratch", e))?;
        let mut roots = Vec::new();
        for root in read_roots.iter().filter(|root| root.exists()) {
            let root = root
                .canonicalize()
                .map_err(|e| RuntimeError::io(format!("resolving {}", root.display()), e))?;
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        Ok(Self {
            executor,
            read_roots: roots,
            scratch,
        })
    }

    /// The canonical read roots in use.
    #[must_use]
    pub fn read_roots(&self) -> &[PathBuf] {
        &self.read_roots
    }

    fn run(
        &self,
        task: &TaskDir,
        split: Split,
        solution: &Solution,
        seed: Seed,
    ) -> Result<Run, RuntimeError> {
        let work = self.scratch.join(format!(
            "run-{}-{}",
            std::process::id(),
            NEXT_WORK_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&work).map_err(|e| RuntimeError::io("creating a work dir", e))?;
        let spec = SandboxSpec::new(
            self.read_roots.clone(),
            vec![work.clone()],
            work.clone(),
            solution_env(&work, seed),
            task.manifest().limits,
        )?;
        if spec.can_reach(task.root()) {
            remove_work(&work)?;
            return Err(RuntimeError::Sandbox(format!(
                "task {} is reachable from the sandbox; refusing to run",
                task.root().display()
            )));
        }
        task.stage(split, solution, &work)?;
        let (program, args) = task.command();
        let outcome = match self.executor.exec(&spec, &program, &args) {
            Ok(outcome) => outcome,
            Err(error) => {
                remove_work(&work)?;
                return Err(error);
            }
        };
        // The executor has killed and verified the whole process group by
        // now, so nothing in the sandbox can still change the file.
        let output = match outcome.termination {
            Termination::Exited(0) => {
                read_untrusted(&work.join(OUTPUT_FILE), task.manifest().limits.file_bytes())?
            }
            _ => Err(Rejected::Missing),
        };
        Ok(Run {
            outcome,
            output,
            work,
        })
    }
}

fn solution_env(work: &Path, seed: Seed) -> Vec<(String, String)> {
    [
        ("PATH", "/usr/local/bin:/usr/bin:/bin".to_owned()),
        ("HOME", work.display().to_string()),
        ("LANG", "C.UTF-8".to_owned()),
        ("PYTHONHASHSEED", "0".to_owned()),
        ("PYTHONDONTWRITEBYTECODE", "1".to_owned()),
        ("RSI_SEED", seed.get().to_string()),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect()
}

fn remove_work(work: &Path) -> Result<(), RuntimeError> {
    std::fs::remove_dir_all(work)
        .map_err(|e| RuntimeError::io(format!("removing {}", work.display()), e))
}

/// The run's output, as the inner agent sees it: termination, then stdout
/// and stderr, cut to [`FEEDBACK_BYTES`] from the end.
fn feedback(outcome: &ExecOutcome, output: &Result<Vec<u8>, Rejected>, scored: bool) -> String {
    let status = match (outcome.termination, output) {
        (Termination::Exited(0), _) if scored => "exited 0".to_owned(),
        (Termination::Exited(0), Ok(_)) => format!("exited 0 but {OUTPUT_FILE} was invalid"),
        (Termination::Exited(0), Err(Rejected::Missing)) => {
            format!("exited 0 but {OUTPUT_FILE} was missing")
        }
        (Termination::Exited(0), Err(rejected)) => {
            format!(
                "exited 0 but {OUTPUT_FILE} was rejected: {}",
                rejected.reason()
            )
        }
        (termination, _) => describe(termination),
    };
    let mut text = format!("[{status}]\n");
    text.push_str(&String::from_utf8_lossy(&outcome.stdout));
    if !outcome.stderr.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&String::from_utf8_lossy(&outcome.stderr));
    if text.len() <= FEEDBACK_BYTES {
        return text;
    }
    let cut = text.len() - FEEDBACK_BYTES;
    let start = (cut..text.len())
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(text.len());
    format!("[{status}; output truncated]\n{}", &text[start..])
}

fn describe(termination: Termination) -> String {
    match termination {
        Termination::Exited(code) => format!("exited with status {code}"),
        Termination::Signaled(24) => "killed at the CPU limit (SIGXCPU)".to_owned(),
        Termination::Signaled(9) => "killed (SIGKILL; memory or CPU hard limit)".to_owned(),
        Termination::Signaled(signal) => format!("killed by signal {signal}"),
        Termination::TimedOut => "killed at the wall-clock limit".to_owned(),
    }
}

/// A task as the inner loop sees it: public split, in-process scoring.
#[derive(Debug)]
pub struct LocalTask<'a, E> {
    task: &'a TaskDir,
    runner: &'a SolutionRunner<E>,
}

impl<'a, E> LocalTask<'a, E> {
    /// Wraps a task directory and the runner that executes its solutions.
    #[must_use]
    pub const fn new(task: &'a TaskDir, runner: &'a SolutionRunner<E>) -> Self {
        Self { task, runner }
    }
}

impl<E: Executor<Error = RuntimeError>> PublicTask for LocalTask<'_, E> {
    type Error = RuntimeError;

    fn id(&self) -> &TaskId {
        &self.task.manifest().id
    }

    fn baseline(&self) -> &Solution {
        self.task.baseline()
    }

    fn public_score(&self, solution: &Solution, seed: Seed) -> Result<Attempt, RuntimeError> {
        let run = self.runner.run(self.task, Split::Public, solution, seed)?;
        // Inputs and labels come from the task directory, never from the
        // work directory the solution controlled.
        let score = match &run.output {
            Ok(bytes) => self
                .task
                .score(Split::Public, &String::from_utf8_lossy(bytes))?,
            Err(_) => None,
        };
        remove_work(&run.work)?;
        Ok(Attempt {
            score,
            feedback: feedback(&run.outcome, &run.output, score.is_some()),
            wall: run.outcome.wall,
        })
    }
}

/// How to start the out-of-process grader: the `rsi` binary and `__grade`.
#[derive(Debug, Clone)]
pub struct GraderCommand {
    /// The program to start.
    pub program: PathBuf,
    /// Arguments before the grader's own flags.
    pub args: Vec<OsString>,
}

/// Grades solutions on private data through a separate grader process.
#[derive(Debug)]
pub struct SandboxedGrader<'a, E> {
    tasks: &'a [TaskDir],
    runner: &'a SolutionRunner<E>,
    grader: GraderCommand,
}

impl<'a, E> SandboxedGrader<'a, E> {
    /// A grader over `tasks`, running solutions with `runner` and scoring
    /// their output with `grader`.
    #[must_use]
    pub const fn new(
        tasks: &'a [TaskDir],
        runner: &'a SolutionRunner<E>,
        grader: GraderCommand,
    ) -> Self {
        Self {
            tasks,
            runner,
            grader,
        }
    }

    /// Hands the output snapshot to `rsi __grade` on stdin: the grader never
    /// sees, let alone opens, a path the solution controlled.
    fn grade_out_of_process(&self, task: &TaskDir, output: &[u8]) -> Result<Score, RuntimeError> {
        use std::io::Write;
        use std::process::Stdio;

        let mut child = std::process::Command::new(&self.grader.program)
            .args(&self.grader.args)
            .arg("--task")
            .arg(task.root())
            .args(["--split", "private", "--output", "-"])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| RuntimeError::io("starting the grader process", e))?;
        let written = match child.stdin.take() {
            Some(mut stdin) => stdin.write_all(output),
            None => Err(std::io::Error::other("grader stdin was not piped")),
        };
        let result = child
            .wait_with_output()
            .map_err(|e| RuntimeError::io("waiting for the grader process", e))?;
        written.map_err(|e| RuntimeError::io("sending output to the grader", e))?;
        if !result.status.success() {
            return Err(RuntimeError::Grader(format!(
                "{}: {}",
                result.status,
                String::from_utf8_lossy(&result.stderr).trim()
            )));
        }
        parse_score_line(&String::from_utf8_lossy(&result.stdout))
    }
}

impl<E: Executor<Error = RuntimeError>> PrivateGrader for SandboxedGrader<'_, E> {
    type Error = RuntimeError;

    fn private_score(
        &self,
        task: &TaskId,
        solution: Option<&Solution>,
        seed: Seed,
    ) -> Result<Score, RuntimeError> {
        let task = self
            .tasks
            .iter()
            .find(|t| &t.manifest().id == task)
            .ok_or_else(|| RuntimeError::Task(format!("unknown task `{}`", task.as_str())))?;
        let Some(solution) = solution else {
            return Ok(task.manifest().floor);
        };
        let run = self.runner.run(task, Split::Private, solution, seed)?;
        let score = match &run.output {
            Ok(bytes) => self.grade_out_of_process(task, bytes),
            Err(_) => Ok(task.manifest().floor),
        };
        remove_work(&run.work)?;
        score
    }
}

/// Formats a score so [`parse_score_line`] recovers it bit for bit.
#[must_use]
pub fn score_line(score: Score) -> String {
    format!("score {:?}", score.get())
}

/// Parses the grader's `score <value>` line.
///
/// # Errors
/// [`RuntimeError::Grader`] for anything else, including a value outside `[0, 1]`.
pub fn parse_score_line(text: &str) -> Result<Score, RuntimeError> {
    let value = text
        .trim()
        .strip_prefix("score ")
        .and_then(|v| v.parse::<f64>().ok())
        .ok_or_else(|| RuntimeError::Grader(format!("unexpected output {:?}", text.trim())))?;
    Score::new(value).map_err(|e| RuntimeError::Grader(e.to_string()))
}

/// The `rsi __grade` entry point: `--task DIR --split SPLIT --output FILE`,
/// where `FILE` may be `-` for standard input.
///
/// A file is read with [`read_untrusted`]; a missing, rejected or oversized
/// output scores the task's floor.
///
/// # Errors
/// [`RuntimeError::Grader`] for bad arguments; task loading and reading errors.
pub fn grade_main(args: &[OsString]) -> Result<Score, RuntimeError> {
    let bad = |what: &str| RuntimeError::Grader(format!("arguments: {what}"));
    let (mut task, mut split, mut output) = (None, None, None);
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let value = iter.next().ok_or_else(|| bad("flag without a value"))?;
        match flag.to_str() {
            Some("--task") => task = Some(PathBuf::from(value)),
            Some("--split") => {
                split = Some(
                    value
                        .to_str()
                        .and_then(Split::parse)
                        .ok_or_else(|| bad("bad --split"))?,
                );
            }
            Some("--output") => output = Some(PathBuf::from(value)),
            _ => return Err(bad("unknown flag")),
        }
    }
    let task = TaskDir::load(&task.ok_or_else(|| bad("--task is required"))?)?;
    let split = split.ok_or_else(|| bad("--split is required"))?;
    let output = output.ok_or_else(|| bad("--output is required"))?;
    let limit = task.manifest().limits.file_bytes();
    let bytes = if output.as_os_str() == "-" {
        read_capped(std::io::stdin().lock(), limit)?
    } else {
        read_untrusted(&output, limit)?
    };
    let text = bytes.ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    task.score_output(split, text.as_deref())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn score_lines_round_trip_exactly() {
        for value in [0.0, 1.0, 0.1, 1.0 / 3.0, 5e-324, 0.999_999_999_999_999_9] {
            let score = Score::new(value).expect("valid");
            assert_eq!(
                parse_score_line(&score_line(score)).ok().map(Score::get),
                Some(value)
            );
        }
        for bad in [
            "",
            "score",
            "score x",
            "score 1.5",
            "score -0.1",
            "points 0.5",
        ] {
            assert!(parse_score_line(bad).is_err(), "{bad:?}");
        }
    }

    fn outcome(termination: Termination, stdout: &str) -> ExecOutcome {
        ExecOutcome {
            termination,
            stdout: stdout.as_bytes().to_vec(),
            stderr: b"err\n".to_vec(),
            wall: Duration::from_millis(5),
        }
    }

    #[test]
    fn feedback_names_the_termination() {
        let ok: Result<Vec<u8>, Rejected> = Ok(b"1\n".to_vec());
        let missing = Err(Rejected::Missing);
        let fb =
            |t, out: &Result<Vec<u8>, Rejected>, scored| feedback(&outcome(t, ""), out, scored);
        assert!(
            feedback(&outcome(Termination::Exited(0), "hi\n"), &ok, true)
                .starts_with("[exited 0]\nhi\nerr")
        );
        assert!(fb(Termination::Exited(0), &ok, false).contains("was invalid"));
        assert!(fb(Termination::Exited(0), &missing, false).contains("was missing"));
        let link = fb(Termination::Exited(0), &Err(Rejected::Symlink), false);
        assert!(link.contains("rejected: a symbolic link"), "{link}");
        assert!(fb(Termination::Signaled(24), &missing, false).contains("CPU limit"));
        assert!(fb(Termination::TimedOut, &missing, false).contains("wall-clock"));
        assert!(fb(Termination::Exited(3), &missing, false).contains("status 3"));
    }

    #[test]
    fn feedback_keeps_the_tail_on_a_char_boundary() {
        let long = format!("{}é-end", "x".repeat(FEEDBACK_BYTES * 2));
        let text = feedback(
            &outcome(Termination::Exited(1), &long),
            &Err(Rejected::Missing),
            false,
        );
        assert!(text.starts_with("[exited with status 1; output truncated]"));
        assert!(text.ends_with("-end\nerr\n"));
        assert!(text.len() <= FEEDBACK_BYTES + 64);
    }
}
