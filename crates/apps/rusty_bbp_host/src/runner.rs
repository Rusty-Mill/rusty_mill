//! The runner supervisor. One invocation serves the task's selected run:
//! check out the candidate's base in a fresh clone, apply its diffs in order,
//! execute every profile under the sandbox, store the log, then the report.
//!
//! The supervisor is trusted and holds the run secret; the workload is not
//! and runs confined. A sandbox that cannot be set up means nothing runs and
//! the run is reported as `error`, never as a silent unconfined pass.

use crate::profiles::{self, ProfileSet};
use crate::{fresh_op, now, open_driver};
use rusty_bbp::*;
use rusty_sandbox::{
    ExecOutcome, Executor, Limits, ProcessExecutor, SandboxSpec, Sockets, Termination,
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as Proc, Stdio};
use std::time::Duration;

/// How the workload is confined, as recorded in the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confinement {
    /// `rusty_sandbox`: Landlock, seccomp refusing `socket(2)`, rlimits.
    Sandboxed,
    /// Plain `std::process`, for trusted local runs and tests. Recorded as such.
    Unconfined,
}

impl Confinement {
    /// The policy name written into `Report.sandbox`.
    pub fn label(&self) -> &'static str {
        match self {
            Confinement::Sandboxed => Sandboxed::LABEL,
            Confinement::Unconfined => "unconfined",
        }
    }
}

/// The sandboxed executor. `rusty_sandbox`'s default socket rule only bars
/// the internet; a test workload gets [`Sockets::NoEndpoints`], so it can
/// reach no network, Unix or abstract socket at all (anonymous socketpairs,
/// which rustc needs to start its linker, still work).
pub struct Sandboxed(ProcessExecutor);

impl Sandboxed {
    pub const LABEL: &'static str = "landlock+seccomp:no-endpoints";

    /// `helper` is the `bbp` binary, re-invoked as `bbp __sandbox`.
    /// `state_dir` must lie outside every workload's reach.
    pub fn new(helper: PathBuf, state_dir: &Path) -> Sandboxed {
        Sandboxed(ProcessExecutor::new(
            helper,
            vec!["__sandbox".into()],
            state_dir.to_path_buf(),
        ))
    }
}

impl Executor for Sandboxed {
    type Error = rusty_sandbox::Error;
    fn exec(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<ExecOutcome, rusty_sandbox::Error> {
        self.0
            .exec_with(spec, program, args, Stdio::null(), Sockets::NoEndpoints)
    }
}

/// The sandboxed executor with this binary as the helper.
pub fn sandboxed(state_dir: &Path) -> Result<Sandboxed, String> {
    let me = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    Ok(Sandboxed::new(me, state_dir))
}

/// An executor that confines nothing. Only for trusted local use; the report
/// says `unconfined` so a reviewer can see it.
pub struct Unconfined;

impl Executor for Unconfined {
    type Error = String;
    fn exec(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<ExecOutcome, String> {
        let start = std::time::Instant::now();
        let out = Proc::new(program)
            .args(args)
            .current_dir(spec.cwd())
            .env_clear()
            .envs(spec.env().iter().cloned())
            .output()
            .map_err(|e| format!("spawn {program}: {e}"))?;
        let termination = match out.status.code() {
            Some(c) => Termination::Exited(c),
            None => Termination::Signaled(0),
        };
        Ok(ExecOutcome {
            termination,
            stdout: out.stdout,
            stderr: out.stderr,
            wall: start.elapsed(),
        })
    }
}

fn git(work: &Path, args: &[&str]) -> Result<String, String> {
    git_with_stdin(work, args, &[])
}

/// Run git in `work` with `stdin` as its standard input. A patch travels this
/// way so the supervisor never writes a file into the candidate's checkout,
/// where a tracked or candidate-created symlink could redirect the write.
fn git_with_stdin(work: &Path, args: &[&str], stdin: &[u8]) -> Result<String, String> {
    let mut child = Proc::new("git")
        .arg("-C")
        .arg(work)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git: {e}"))?;
    if let Some(mut pipe) = child.stdin.take() {
        // A git that stops reading early reports its own reason below.
        let _ = pipe.write_all(stdin);
    }
    let out = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Clone `repo` into `work`, check out `base`, apply `diffs` in order.
/// Returns the resulting tree id. Any failure is an `error` run.
pub fn prepare(repo: &Path, work: &Path, base: &str, diffs: &[Vec<u8>]) -> Result<String, String> {
    if work.exists() {
        std::fs::remove_dir_all(work).map_err(|e| e.to_string())?;
    }
    let out = Proc::new("git")
        .args(["clone", "--quiet", "--no-checkout"])
        .arg(repo)
        .arg(work)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "git clone: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    git(work, &["checkout", "--quiet", base])?;
    for diff in diffs {
        git_with_stdin(work, &["apply", "--index", "-"], diff)?;
    }
    git(work, &["write-tree"])
}

struct Outcome {
    status: RunStatus,
    profiles: Vec<ProfileResult>,
    log: String,
}

fn execute<E: Executor>(exec: &E, set: &ProfileSet, work: &Path) -> Outcome
where
    E::Error: std::fmt::Display,
{
    if set.profiles.is_empty() {
        return Outcome {
            status: RunStatus::Error,
            profiles: vec![],
            log: "profile set has no profiles\n".into(),
        };
    }
    let mut log = String::new();
    let mut results = Vec::new();
    let mut status = RunStatus::Passed;
    let limits = Limits::new(
        Duration::from_secs(set.limits.cpu_secs.max(1)),
        Duration::from_secs(set.limits.wall_secs.max(1)),
        set.limits.memory_bytes,
        set.limits.file_bytes,
        set.limits.open_files,
        set.limits.processes,
    );
    let limits = match limits {
        Ok(l) => l,
        Err(e) => {
            return Outcome {
                status: RunStatus::Error,
                profiles: vec![],
                log: format!("invalid limits: {e}"),
            }
        }
    };
    let mut read_roots: Vec<PathBuf> = set.read_roots.iter().map(PathBuf::from).collect();
    read_roots.push(work.to_path_buf());
    let spec = match SandboxSpec::new(
        read_roots,
        vec![work.to_path_buf()],
        work.to_path_buf(),
        set.env.clone(),
        limits,
    ) {
        Ok(s) => s,
        Err(e) => {
            return Outcome {
                status: RunStatus::Error,
                profiles: vec![],
                log: format!("invalid sandbox spec: {e}"),
            }
        }
    };
    for p in &set.profiles {
        log.push_str(&format!(
            "=== {} : {} {}\n",
            p.name,
            p.program,
            p.args.join(" ")
        ));
        match exec.exec(&spec, &p.program, &p.args) {
            Ok(out) => {
                log.push_str(&String::from_utf8_lossy(&out.stdout));
                log.push_str(&String::from_utf8_lossy(&out.stderr));
                let (code, failed) = match out.termination {
                    Termination::Exited(0) => (0, 0),
                    Termination::Exited(c) => {
                        status = worst(status, RunStatus::Failed);
                        (c, 1)
                    }
                    Termination::Signaled(s) => {
                        status = worst(status, RunStatus::Failed);
                        (128 + s, 1)
                    }
                    Termination::TimedOut => {
                        status = RunStatus::Error;
                        (124, 1)
                    }
                };
                log.push_str(&format!("--- {}: exit {code} in {:?}\n", p.name, out.wall));
                results.push(ProfileResult {
                    name: p.name.clone(),
                    exit_code: code,
                    failed,
                });
            }
            Err(e) => {
                status = RunStatus::Error;
                log.push_str(&format!("--- {}: not run: {e}\n", p.name));
                results.push(ProfileResult {
                    name: p.name.clone(),
                    exit_code: 125,
                    failed: 1,
                });
            }
        }
    }
    Outcome {
        status,
        profiles: results,
        log,
    }
}

fn worst(a: RunStatus, b: RunStatus) -> RunStatus {
    match (a, b) {
        (RunStatus::Error, _) | (_, RunStatus::Error) => RunStatus::Error,
        (RunStatus::Failed, _) | (_, RunStatus::Failed) => RunStatus::Failed,
        _ => RunStatus::Passed,
    }
}

/// Serve the task's selected run once. `repo` is a local path to the task's
/// repository; `work_root` holds one checkout per run.
pub fn run_once<E: Executor>(
    dir: &Path,
    task: &TaskId,
    repo: &Path,
    work_root: &Path,
    exec: &E,
    confinement: Confinement,
) -> Result<Response, String>
where
    E::Error: std::fmt::Display,
{
    let mut d = open_driver(dir, task)?;
    let st = &d.state;
    let run = st
        .selected_run()
        .filter(|r| r.status.is_none())
        .ok_or("no pending selected run")?
        .clone();
    let cand = st
        .artifacts
        .get(&run.candidate)
        .ok_or("candidate record missing")?;
    let ArtifactPayload::Candidate(manifest) = &cand.payload else {
        return Err("candidate payload is not a manifest".into());
    };
    let set = profiles::load(dir, task, st.profile_digest)?;
    let mut diffs = Vec::new();
    for id in &manifest.diffs {
        let a = st
            .artifacts
            .get(id)
            .ok_or_else(|| format!("{id} missing"))?;
        diffs.push(
            d.store
                .blob_get(&a.blob.sha)
                .map_err(|e| format!("{e:?}"))?,
        );
    }
    let work = work_root.join(format!("run-{}", run.id.0));
    let secret = d.state.secret_for(run.id);
    // A previous invocation stored the log and died before the report (A4:
    // log first, report second). The core accepts one log per run, and the
    // result that log describes died with that invocation, so a fresh
    // execution could only be reported against evidence it did not produce.
    // Report `error` against the log on record and let the human rerun.
    let interrupted = run.log.is_some();
    let (tree, outcome) = if interrupted {
        (
            None,
            Outcome {
                status: RunStatus::Error,
                profiles: vec![],
                log: String::new(),
            },
        )
    } else {
        match prepare(repo, &work, &manifest.base, &diffs) {
            Ok(tree) => (
                Some(Sha256::of(tree.as_bytes())),
                execute(exec, &set, &work),
            ),
            Err(e) => (
                None,
                Outcome {
                    status: RunStatus::Error,
                    profiles: vec![],
                    log: format!("prepare: {e}\n"),
                },
            ),
        }
    };
    let log_id = match run.log {
        Some(id) => id,
        None => {
            let log_blob = d.store.blob_put(outcome.log.as_bytes());
            let logged = d
                .dispatch(
                    &Command::Runner {
                        op: fresh_op(),
                        run: run.id,
                        secret,
                        blob: log_blob,
                        payload: ArtifactPayload::Log,
                    },
                    now(),
                )
                .map_err(|e| format!("{e:?}"))?;
            match logged {
                Response::Stored(id) => id,
                other => return Ok(other),
            }
        }
    };
    let report = Report {
        candidate: run.candidate,
        run: run.id,
        profile_digest: d.state.profile_digest,
        status: outcome.status,
        profiles: outcome.profiles,
        tree,
        sandbox: confinement.label().into(),
        log: log_id,
    };
    let payload = ArtifactPayload::TestReport(report);
    let bytes = encode_artifact(&payload, b"").map_err(|e| e.to_string())?;
    let blob = d.store.blob_put(&bytes);
    d.dispatch(
        &Command::Runner {
            op: fresh_op(),
            run: run.id,
            secret,
            blob,
            payload,
        },
        now(),
    )
    .map_err(|e| format!("{e:?}"))
}
