//! `rsi calibrate`, `rsi run` and `rsi report`: the outer loop's commands.
//!
//! ```text
//! rsi calibrate --repo DIR --tasks DIR --tokens N --wall-secs N --out FILE
//!               [--base REV] [--rounds 5] [--z 1.645] [--seeds 1] [--seed N] [--work DIR]
//! rsi run       --repo DIR --tasks DIR --run-dir DIR --steps N --tokens N --wall-secs N
//!               (--calibration FILE | --margin X)
//!               [--base REV] [--seeds 1] [--seed N] [--work DIR]
//! rsi report    --run-dir DIR [--replay --repo DIR --tasks DIR [--work DIR]]
//! ```
//!
//! Models come from the environment, one set per role: `RSI_INNER_*` for
//! the agent under test and `RSI_OUTER_*` for the proposer (`_MODEL`,
//! `_BASE_URL`, `_API_KEY`), or a coding-agent CLI: `RSI_INNER_PROVIDER=codex`
//! for the inner model, `RSI_OUTER_PROPOSER=codex` or `claude` for the
//! proposer (see [`crate::config`]). Nothing secret is accepted as a flag or
//! written to the run.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rsi_core::{Budget, LineageStore, Margin, ModelId, Seed};
use rsi_runtime::agent_cli::CliProposer;
use rsi_runtime::git::Repo;
use rsi_runtime::grading::{GraderCommand, SYSTEM_READ_ROOTS};
use rsi_runtime::lineage_store::{Blobs, JsonlLineage};
use rsi_runtime::outer::{
    calibrate, calibrated_margin, encode_calibration, read_run, run, Grading, Lab, RunConfig,
};
use rsi_runtime::proposer::ModelProposer;
use rsi_runtime::report::{check, replay, summary, RunStatus};
use rsi_runtime::{ProcessExecutor, ScriptedModel, SolutionRunner, TaskDir, Toolchain};

use crate::config::{inner_from_env, outer_from_env, Inner, Outer};
use crate::flags::Flags;

/// Completion tokens per inner model call.
const INNER_COMPLETION_TOKENS: u64 = 4096;
/// Completion tokens per proposal.
const OUTER_COMPLETION_TOKENS: u64 = 16_384;
/// How long one proposal may take.
const OUTER_TIMEOUT: Duration = Duration::from_secs(900);
/// How long past its budget an agent may run.
const GRACE: Duration = Duration::from_secs(5);

/// Everything the three commands share: the repository, the tasks and the
/// sandbox around them.
struct Setup {
    repo: Repo,
    tasks: Vec<TaskDir>,
    executor: ProcessExecutor,
    toolchain: Toolchain,
    runner: SolutionRunner<ProcessExecutor>,
    rsi: PathBuf,
    work: PathBuf,
    state: PathBuf,
}

impl Setup {
    fn new(flags: &Flags) -> Result<Self, String> {
        let repo = Repo::open(&flags.path("--repo").unwrap_or_else(|| PathBuf::from(".")))
            .map_err(|e| e.to_string())?;
        let tasks =
            TaskDir::load_suite(&flags.required_path("--tasks")?).map_err(|e| e.to_string())?;
        let work = flags.path("--work").unwrap_or_else(|| {
            std::env::temp_dir().join(format!("rsi-work-{}", std::process::id()))
        });
        std::fs::create_dir_all(&work).map_err(|e| format!("creating {}: {e}", work.display()))?;
        let work = work
            .canonicalize()
            .map_err(|e| format!("resolving {}: {e}", work.display()))?;
        let state = work.join("state");
        std::fs::create_dir_all(&state)
            .map_err(|e| format!("creating {}: {e}", state.display()))?;
        let rsi = std::env::current_exe().map_err(|e| format!("locating rsi: {e}"))?;
        let executor = ProcessExecutor::new(rsi.clone(), vec!["__sandbox".into()], state.clone());
        let rustc = std::env::var_os("RSI_RUSTC").unwrap_or_else(|| "rustc".into());
        let roots: Vec<PathBuf> = SYSTEM_READ_ROOTS.iter().map(PathBuf::from).collect();
        let runner = SolutionRunner::new(executor.clone(), &roots, &work.join("solutions"))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            toolchain: Toolchain::detect(&rustc).map_err(|e| e.to_string())?,
            repo,
            tasks,
            executor,
            runner,
            rsi,
            work,
            state,
        })
    }

    /// What no sandbox may reach: the tasks, the repository, the
    /// executor's state and `extra` (the run directory).
    fn protected(&self, extra: &[PathBuf]) -> Vec<PathBuf> {
        let mut protected: Vec<PathBuf> = self
            .tasks
            .iter()
            .map(|t| t.root().to_path_buf())
            .chain([self.repo.root().to_path_buf(), self.state.clone()])
            .collect();
        protected.extend_from_slice(extra);
        protected
    }

    /// The inner model from the environment (see [`inner_from_env`]).
    fn inner(&self, extra: &[PathBuf]) -> Result<Inner, String> {
        inner_from_env(
            &self.executor,
            self.work.join("inner-model"),
            self.protected(extra),
        )
    }

    /// A lab around `model`; no agent may reach [`Setup::protected`].
    fn lab<'a, M>(&'a self, model: &'a M, extra: &[PathBuf]) -> Lab<'a, M> {
        let protected = self.protected(extra);
        Lab {
            repo: &self.repo,
            tasks: &self.tasks,
            executor: &self.executor,
            toolchain: &self.toolchain,
            runner: &self.runner,
            grader: GraderCommand {
                program: self.rsi.clone(),
                args: vec!["__grade".into()],
            },
            model,
            scratch: self.work.join("lab"),
            protected,
        }
    }
}

fn grading(flags: &Flags) -> Result<Grading, String> {
    let budget = Budget::new(
        flags.required_number("--tokens")?,
        Duration::from_secs(flags.required_number("--wall-secs")?),
        None,
    )
    .map_err(|e| e.to_string())?;
    Ok(Grading {
        run_seed: Seed::new(flags.number("--seed", 0)?),
        seeds_per_round: flags.number("--seeds", 1)?,
        budget,
        max_completion_tokens: INNER_COMPLETION_TOKENS,
        grace: GRACE,
    })
}

/// `rsi calibrate`: grades the base harness on disjoint seed sets and
/// writes the noise band and margin to `--out`.
///
/// # Errors
/// A usage, configuration or run error, as a message.
pub fn calibrate_main(args: &[OsString]) -> Result<String, String> {
    let flags = Flags::parse(
        args,
        &[
            "--repo",
            "--tasks",
            "--tokens",
            "--wall-secs",
            "--out",
            "--base",
            "--rounds",
            "--z",
            "--seeds",
            "--seed",
            "--work",
        ],
        &[],
    )?;
    let out = flags.required_path("--out")?;
    let setup = Setup::new(&flags)?;
    let model = setup.inner(&[])?;
    let lab = setup.lab(&model, &[]);
    let base = setup
        .repo
        .resolve(&flags.text("--base").unwrap_or_else(|| "HEAD".into()))
        .map_err(|e| e.to_string())?;
    let blobs = Blobs::open(&setup.work.join("calibration")).map_err(|e| e.to_string())?;
    let result = calibrate(
        &lab,
        &base,
        &grading(&flags)?,
        flags.number("--rounds", 5)?,
        flags.number("--z", 1.645)?,
        &blobs,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(&out, encode_calibration(&result).to_json_string_pretty())
        .map_err(|e| format!("writing {}: {e}", out.display()))?;
    let band = &result.band;
    Ok(format!(
        "grades {:?}\nmean {:.4}, sd {:.4} (n = {}; with so few rounds the sd is itself rough)\n\
         range {:.4} to {:.4}\nmargin {:.4} (z = {})\nwritten to {}\n",
        result.grades.iter().map(|g| g.get()).collect::<Vec<_>>(),
        band.mean(),
        band.std_dev(),
        band.samples(),
        band.min(),
        band.max(),
        result.margin.get(),
        result.z,
        out.display()
    ))
}

/// The run's name: the run directory's final component.
fn run_name(run_dir: &Path) -> Result<String, String> {
    run_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("{} has no usable name", run_dir.display()))
}

/// `rsi run`: the outer loop.
///
/// # Errors
/// A usage, configuration or run error, as a message.
pub fn run_main(args: &[OsString]) -> Result<String, String> {
    let flags = Flags::parse(
        args,
        &[
            "--repo",
            "--tasks",
            "--run-dir",
            "--steps",
            "--tokens",
            "--wall-secs",
            "--calibration",
            "--margin",
            "--base",
            "--seeds",
            "--seed",
            "--work",
        ],
        &[],
    )?;
    let margin = match (flags.path("--calibration"), flags.text("--margin")) {
        (Some(file), None) => {
            let text = std::fs::read_to_string(&file)
                .map_err(|e| format!("reading {}: {e}", file.display()))?;
            calibrated_margin(&text).map_err(|e| e.to_string())?
        }
        (None, Some(value)) => Margin::new(flags.required_number("--margin")?)
            .map_err(|e| format!("--margin {value}: {e}"))?,
        _ => return Err("give exactly one of --calibration FILE and --margin X".into()),
    };
    let run_dir = flags.required_path("--run-dir")?;
    std::fs::create_dir_all(&run_dir)
        .map_err(|e| format!("creating {}: {e}", run_dir.display()))?;
    let run_dir = run_dir
        .canonicalize()
        .map_err(|e| format!("resolving {}: {e}", run_dir.display()))?;
    let setup = Setup::new(&flags)?;
    let inner = setup.inner(std::slice::from_ref(&run_dir))?;
    let outer = outer_from_env(OUTER_TIMEOUT)?;
    let lab = setup.lab(&inner, std::slice::from_ref(&run_dir));
    let config = RunConfig {
        name: run_name(&run_dir)?,
        base: setup
            .repo
            .resolve(&flags.text("--base").unwrap_or_else(|| "HEAD".into()))
            .map_err(|e| e.to_string())?,
        steps: flags.required_number("--steps")?,
        margin,
        grading: grading(&flags)?,
    };
    let entries = match outer {
        Outer::Model(model) => {
            let proposer = ModelProposer::new(&model, OUTER_COMPLETION_TOKENS, OUTER_TIMEOUT);
            run(&lab, &proposer, &config, &run_dir)
        }
        Outer::Cli(cli) => {
            let proposer = CliProposer::new(
                &setup.executor,
                cli,
                setup.work.join("proposer"),
                lab.protected.clone(),
            )
            .map_err(|e| e.to_string())?;
            run(&lab, &proposer, &config, &run_dir)
        }
    }
    .map_err(|e| e.to_string())?;
    let info = read_run(&run_dir).map_err(|e| e.to_string())?;
    summary(&info, &entries).map_err(|e| e.to_string())
}

/// `rsi report`: the run's summary, and with `--replay` its grade and
/// trajectory replay.
///
/// # Errors
/// A usage error, a broken lineage, or a replay that does not reproduce.
pub fn report_main(args: &[OsString]) -> Result<String, String> {
    let flags = Flags::parse(
        args,
        &["--run-dir", "--repo", "--tasks", "--work"],
        &["--replay"],
    )?;
    let run_dir = flags.required_path("--run-dir")?;
    let info = read_run(&run_dir).map_err(|e| e.to_string())?;
    let entries = JsonlLineage::open(&run_dir)
        .and_then(|store| store.entries())
        .map_err(|e| e.to_string())?;
    let mut report = summary(&info, &entries).map_err(|e| e.to_string())?;
    if !flags.switch("--replay") {
        return Ok(report);
    }
    let setup = Setup::new(&flags)?;
    // Replay serves every model reply from the transcript; this model is
    // never called.
    let no_model = ScriptedModel::new(
        ModelId::parse("replay").map_err(|e| e.to_string())?,
        vec![],
        0,
        0,
    );
    let run_dir = run_dir
        .canonicalize()
        .map_err(|e| format!("resolving {}: {e}", run_dir.display()))?;
    let lab = setup.lab(&no_model, std::slice::from_ref(&run_dir));
    let blobs = Blobs::open(&run_dir).map_err(|e| e.to_string())?;
    let replayed = replay(&lab, &info, &entries, &blobs).map_err(|e| e.to_string())?;
    report.push_str(&format!(
        "\n## Replay\n\n- {} private scores re-graded\n- {} inner runs replayed from transcripts\n",
        replayed.grades, replayed.trajectories
    ));
    if !replayed.mismatches.is_empty() {
        return Err(format!(
            "{report}\nthe run did not reproduce:\n{}",
            replayed.mismatches.join("\n")
        ));
    }
    match check(&info, &entries).map_err(|e| e.to_string())? {
        RunStatus::Complete => {
            report.push_str("- everything reproduced\n");
            Ok(report)
        }
        RunStatus::Incomplete { recorded, expected } => Err(format!(
            "{report}- the {recorded} recorded candidates reproduced, but the run is \
             incomplete ({recorded} of {expected}); this is not a reproduction of the run\n"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_names_come_from_the_directory() {
        assert_eq!(
            run_name(Path::new("/runs/first-run")),
            Ok("first-run".into())
        );
        assert!(run_name(Path::new("/")).is_err());
    }

    #[test]
    fn run_needs_exactly_one_margin_source() {
        let os = |items: &[&str]| items.iter().map(OsString::from).collect::<Vec<_>>();
        let base = [
            "--run-dir",
            "/tmp/x",
            "--steps",
            "1",
            "--tokens",
            "1",
            "--wall-secs",
            "1",
        ];
        for extra in [&[][..], &["--margin", "0.1", "--calibration", "c.json"]] {
            let args = os(&[&base[..], extra].concat());
            let error = run_main(&args).expect_err("refused");
            assert!(error.contains("exactly one"), "{error}");
        }
    }
}
