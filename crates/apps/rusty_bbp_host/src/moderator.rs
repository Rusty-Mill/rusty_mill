//! The moderator loop: `bbp mod`. The core decides who holds the turn; this
//! loop makes it happen. Each iteration it re-reads the store, fires
//! deadlines, starts the runner for a pending selected run, launches one
//! agent harness for a newly granted turn with a `bbp mcp` binding in its
//! environment, and reaps a harness whose turn has ended. Human gates are
//! states with no turn, so the loop simply waits through them.
//!
//! The harness never sees a token: it gets `BBP_DIR`, `BBP_TASK`,
//! `BBP_PRINCIPAL` and `BBP_TURN`, and talks to the per-turn `bbp mcp`
//! server, which derives the token under the directory's master secret.
//!
//! One moderator per task: an exclusive lock on `<dir>/mod/<task>.lock` is
//! held for the loop's lifetime. A turn that was live when the loop started
//! belongs to an invocation this moderator cannot see, so it is aborted
//! before anything is launched; the replacement turn gets a fresh token and
//! the old `bbp mcp` answers "turn has ended". Every process the loop starts
//! is in its own process group and is killed, group and all, on every exit
//! path, the error ones included.

use crate::runner::{self, Confinement, Unconfined};
use crate::{now, open_driver};
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Proc, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How to start one role's agent harness. Every argument may use the
/// placeholders `{dir}`, `{task}`, `{principal}`, `{turn}`, `{role}`,
/// `{bbp}` (this binary) and `{mcp_config}` (a Claude Code MCP config file
/// for the turn's `bbp mcp` server).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launcher {
    pub program: String,
    pub args: Vec<String>,
}

/// One launcher per role; a role without one is left to its deadline.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launchers {
    #[rusty_serde(default)]
    pub planner: Option<Launcher>,
    #[rusty_serde(default)]
    pub coder: Option<Launcher>,
    #[rusty_serde(default)]
    pub tester: Option<Launcher>,
    #[rusty_serde(default)]
    pub reviewer: Option<Launcher>,
}

impl Launchers {
    pub fn read_file(file: &Path) -> Result<Launchers, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))
    }

    pub fn for_role(&self, role: Role) -> Option<&Launcher> {
        match role {
            Role::Planner => self.planner.as_ref(),
            Role::Coder => self.coder.as_ref(),
            Role::Tester => self.tester.as_ref(),
            Role::Reviewer => self.reviewer.as_ref(),
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub dir: PathBuf,
    pub task: TaskId,
    /// The `bbp` binary the harness and its MCP config call; `bbp mod` passes
    /// its own path.
    pub bbp: PathBuf,
    /// Local path of the task's repository, for the runner.
    pub repo: PathBuf,
    /// One checkout per run goes under here.
    pub work: PathBuf,
    pub launchers: Launchers,
    pub confinement: Confinement,
    /// How long to sleep when nothing is ready.
    pub poll: Duration,
    /// Give up after this long; the task is left where it stands.
    pub max_wall: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub state: State,
    pub turns_launched: u64,
    pub runs: u64,
}

/// `<dir>/<sub>/<sha256(task id)>`: one directory per task, injective, so
/// two tasks at the same turn id never share a file.
fn task_dir(dir: &Path, sub: &str, task: &TaskId) -> PathBuf {
    dir.join(sub).join(Sha256::of(task.0.as_bytes()).hex())
}

/// The Claude Code MCP config for one turn of `task`.
pub fn mcp_config_path(dir: &Path, task: &TaskId, turn: TurnId) -> PathBuf {
    task_dir(dir, "mcp", task).join(format!("turn-{}.json", turn.0))
}

/// Where one turn's harness writes its output.
pub fn log_path(dir: &Path, task: &TaskId, turn: TurnId) -> PathBuf {
    task_dir(dir, "agents", task).join(format!("turn-{}.log", turn.0))
}

/// The file whose exclusive lock is the right to moderate `task`.
pub fn lock_path(dir: &Path, task: &TaskId) -> PathBuf {
    task_dir(dir, "mod", task).with_extension("lock")
}

/// Take the moderator lock, or say who else would have to let go.
fn lock(dir: &Path, task: &TaskId) -> Result<File, String> {
    let path = lock_path(dir, task);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(format!(
            "another moderator holds task {} ({})",
            task.0,
            path.display()
        )),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
    }
}

fn json_str(s: &str) -> String {
    rusty_serde::json::to_string(&s).unwrap_or_else(|_| "\"\"".into())
}

/// Write the Claude Code MCP config for one turn, whole or not at all, and
/// return its path. A reader sees either no file or this turn's complete
/// configuration, never another task's.
pub fn write_mcp_config(
    dir: &Path,
    task: &TaskId,
    bbp: &Path,
    principal: &PrincipalId,
    turn: TurnId,
) -> Result<PathBuf, String> {
    let path = mcp_config_path(dir, task, turn);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = format!(
        "{{\"mcpServers\":{{\"bbp\":{{\"command\":{},\"args\":[\"mcp\"],\"env\":{{\"BBP_DIR\":{},\"BBP_TASK\":{},\"BBP_PRINCIPAL\":{},\"BBP_TURN\":\"{}\"}}}}}}}}\n",
        json_str(&bbp.to_string_lossy()),
        json_str(&dir.to_string_lossy()),
        json_str(&task.0),
        json_str(&principal.0),
        turn.0
    );
    rusty_atomic_file::write(&path, text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

struct Harness {
    turn: TurnId,
    child: Child,
}

/// Every process the loop started. Dropping it kills the harness and its
/// whole process group, so an error path cleans up like a normal one.
struct Supervision {
    harness: Option<Harness>,
    runner: Option<JoinHandle<Result<Response, String>>>,
}

impl Supervision {
    /// Kill the harness's process group and reap it.
    fn kill_harness(&mut self) -> Result<(), String> {
        match self.harness.take() {
            Some(mut h) => kill_tree(&mut h.child),
            None => Ok(()),
        }
    }
}

impl Drop for Supervision {
    fn drop(&mut self) {
        if let Err(e) = self.kill_harness() {
            eprintln!("bbp mod: harness not cleaned up: {e}");
        }
        if self.runner.is_some() {
            eprintln!("bbp mod: a run is still executing; its wall limit bounds it");
        }
    }
}

/// `SIGKILL` the child's process group (the harness and everything it
/// forked), then reap the child. A group that is already gone is fine.
fn kill_tree(child: &mut Child) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let group = i32::try_from(child.id()).map_err(|_| "pid out of range".to_owned())?;
        match rusty_libc::process::killpg(group, rusty_libc::signal::SIGKILL) {
            Ok(()) | Err(rusty_libc::Errno::ESRCH) => {}
            Err(e) => return Err(format!("killpg({group}): {e:?}")),
        }
    }
    match child.kill() {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {}
        Err(e) => return Err(format!("kill: {e}")),
    }
    child.wait().map(|_| ()).map_err(|e| format!("wait: {e}"))
}

/// Run the loop until the task is closed or cancelled, or `max_wall` passes.
pub fn run(cfg: &Config) -> Result<Outcome, String> {
    let _lock = lock(&cfg.dir, &cfg.task)?;
    let mut d = open_driver(&cfg.dir, &cfg.task)?;
    let started = Instant::now();
    let mut sup = Supervision {
        harness: None,
        runner: None,
    };
    let mut launched: HashSet<TurnId> = HashSet::new();
    let mut ran: HashSet<RunId> = HashSet::new();

    // A turn granted before this loop started belongs to an invocation it
    // cannot see or stop. Abort it so its token dies and the replacement
    // turn is launched here. Gates have no turn and keep waiting.
    d.dispatch(&Command::Tick, now())
        .map_err(|e| format!("{e:?}"))?;
    if let Some(turn) = d.state.turn.as_ref().map(|t| t.id) {
        if !d.state.state.terminal() {
            eprintln!(
                "bbp mod: turn {} predates this moderator; aborting it",
                turn.0
            );
            d.dispatch(&Command::AbortTurn, now())
                .map_err(|e| format!("{e:?}"))?;
        }
    }

    loop {
        d.sync().map_err(|e| format!("{e:?}"))?;
        d.dispatch(&Command::Tick, now())
            .map_err(|e| format!("{e:?}"))?;
        let state = d.state.state;
        if matches!(state, State::Closed | State::Cancelled) {
            sup.kill_harness()?;
            return Ok(Outcome {
                state,
                turns_launched: launched.len() as u64,
                runs: ran.len() as u64,
            });
        }
        if started.elapsed() > cfg.max_wall {
            sup.kill_harness()?;
            return Err(format!("moderator: wall limit reached in state {state:?}"));
        }

        // The runner works on its own thread so the loop keeps ticking,
        // honouring cancellation and the wall limit while a profile runs.
        if sup.runner.as_ref().is_some_and(JoinHandle::is_finished) {
            if let Some(j) = sup.runner.take() {
                match j.join() {
                    Ok(Ok(r)) => eprintln!("bbp mod: run served: {r:?}"),
                    Ok(Err(e)) => eprintln!("bbp mod: run not served: {e}"),
                    Err(_) => eprintln!("bbp mod: runner thread panicked"),
                }
            }
        }
        // A selected run with no report yet: the runner's job, once per run.
        let pending = (state == State::Test && sup.runner.is_none())
            .then(|| d.state.selected_run())
            .flatten()
            .filter(|r| r.status.is_none() && !ran.contains(&r.id))
            .map(|r| r.id);
        if let Some(run) = pending {
            ran.insert(run);
            let cfg = cfg.clone();
            sup.runner = Some(std::thread::spawn(move || run_once(&cfg)));
        }

        let live = d.state.turn.as_ref().map(|t| (t.id, t.role));

        // Reap: a harness that exited without ending its turn forfeits it;
        // one whose turn ended under it is killed.
        if let Some(h) = sup.harness.as_mut() {
            match h.child.try_wait() {
                Ok(Some(status)) => {
                    eprintln!("bbp mod: harness for turn {} exited: {status}", h.turn.0);
                    let turn = h.turn;
                    sup.kill_harness()?;
                    if live.map(|(id, _)| id) == Some(turn) {
                        d.dispatch(&Command::AbortTurn, now())
                            .map_err(|e| format!("{e:?}"))?;
                        continue;
                    }
                }
                Ok(None) if live.map(|(id, _)| id) != Some(h.turn) => {
                    eprintln!("bbp mod: turn {} ended; stopping its harness", h.turn.0);
                    sup.kill_harness()?;
                }
                Ok(None) => {}
                Err(e) => return Err(format!("harness wait: {e}")),
            }
        }

        // Launch: one harness per granted turn.
        if sup.harness.is_none() {
            if let Some((turn, role)) = live {
                if !launched.contains(&turn) {
                    launched.insert(turn);
                    match cfg.launchers.for_role(role) {
                        Some(l) => {
                            let principal = d
                                .state
                                .assigned
                                .get(&role)
                                .cloned()
                                .ok_or_else(|| format!("{role:?} has no principal"))?;
                            let child = spawn(cfg, l, role, &principal, turn)?;
                            sup.harness = Some(Harness { turn, child });
                            continue;
                        }
                        None => eprintln!(
                            "bbp mod: no launcher for {role:?}; turn {} waits for its deadline",
                            turn.0
                        ),
                    }
                }
            }
        }
        std::thread::sleep(cfg.poll);
    }
}

fn run_once(cfg: &Config) -> Result<Response, String> {
    match cfg.confinement {
        Confinement::Unconfined => runner::run_once(
            &cfg.dir,
            &cfg.task,
            &cfg.repo,
            &cfg.work,
            &Unconfined,
            Confinement::Unconfined,
        ),
        Confinement::Sandboxed => {
            let exec = runner::sandboxed(&cfg.dir.join("sandbox-state"))?;
            runner::run_once(
                &cfg.dir,
                &cfg.task,
                &cfg.repo,
                &cfg.work,
                &exec,
                Confinement::Sandboxed,
            )
        }
    }
}

fn spawn(
    cfg: &Config,
    l: &Launcher,
    role: Role,
    principal: &PrincipalId,
    turn: TurnId,
) -> Result<Child, String> {
    let bbp = &cfg.bbp;
    let mcp = write_mcp_config(&cfg.dir, &cfg.task, bbp, principal, turn)?;
    let role_name = crate::admin::role_name(role);
    let fill = |a: &str| {
        a.replace("{dir}", &cfg.dir.to_string_lossy())
            .replace("{task}", &cfg.task.0)
            .replace("{principal}", &principal.0)
            .replace("{turn}", &turn.0.to_string())
            .replace("{role}", role_name)
            .replace("{bbp}", &bbp.to_string_lossy())
            .replace("{mcp_config}", &mcp.to_string_lossy())
    };
    let log_path = log_path(&cfg.dir, &cfg.task, turn);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let log = File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    eprintln!("bbp mod: launching {role_name} for turn {}", turn.0);
    let mut cmd = Proc::new(fill(&l.program));
    cmd.args(l.args.iter().map(|a| fill(a)))
        .env("BBP_DIR", &cfg.dir)
        .env("BBP_TASK", &cfg.task.0)
        .env("BBP_PRINCIPAL", &principal.0)
        .env("BBP_TURN", turn.0.to_string())
        .env("BBP_ROLE", role_name)
        .env("BBP_BIN", bbp)
        .env("BBP_MCP_CONFIG", &mcp)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err));
    // Its own process group, so everything the harness forks dies with it.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.spawn().map_err(|e| format!("spawn {}: {e}", l.program))
}
