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

use crate::runner::Confinement;
use crate::{now, open_driver};
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Proc, Stdio};
use std::time::{Duration, Instant};

/// How to start one role's agent harness. Every argument may use the
/// placeholders `{dir}`, `{task}`, `{principal}`, `{turn}`, `{role}`,
/// `{bbp}` (this binary) and `{mcp_config}` (a Claude Code MCP config file
/// for the turn's `bbp mcp` server).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launcher {
    pub program: String,
    pub args: Vec<String>,
    /// Whether this role's harness keeps its built-in tools (shell, file
    /// access). Host policy for the role, declared here with the rest of its
    /// launch configuration; the protocol has no opinion.
    #[rusty_serde(default)]
    pub builtin_tools: bool,
}

/// One launcher per role id, keyed by the ids of the task's roster. A role
/// without one is left to its deadline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launchers {
    by_role: BTreeMap<String, Launcher>,
}

impl Launchers {
    /// Read an agents file: a JSON object from role id to [`Launcher`].
    pub fn read_file(file: &Path) -> Result<Launchers, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let by_role =
            rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(Launchers { by_role })
    }

    pub fn with(mut self, role: &str, launcher: Launcher) -> Launchers {
        self.by_role.insert(role.to_owned(), launcher);
        self
    }

    pub fn for_role(&self, role: &RoleId) -> Option<&Launcher> {
        self.by_role.get(&role.0)
    }

    /// The role ids that have a launcher.
    pub fn roles(&self) -> impl Iterator<Item = &str> {
        self.by_role.keys().map(String::as_str)
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
    /// Wall clock for this invocation, from loop start through successful
    /// final harness cleanup. Visits stop before that cleanup.
    pub elapsed: Duration,
    /// Observed state visits for this moderator invocation, in order.
    /// Transitions between observations are invisible, including departures
    /// and returns to the same state. Loop work and scheduling delays mean
    /// measurement error is not bounded by the configured poll interval.
    /// Restarting starts a fresh series; error and wall-limit exits return no
    /// Outcome and discard visits. Successful visits end at the terminal-state
    /// observation, before final harness cleanup (included in `elapsed`).
    /// The first Planning visit excludes any later replanning; these raw visits
    /// do not define authoring time for the ultimately approved spec.
    pub visits: Vec<(State, Duration)>,
}

/// Ordered state visits, timed by the loop's own clock (the log has none).
struct Visits {
    done: Vec<(State, Duration)>,
    current: (State, Instant),
}

impl Visits {
    fn new(state: State, at: Instant) -> Self {
        Visits {
            done: Vec::new(),
            current: (state, at),
        }
    }

    /// The loop saw `state` at `at`; a change closes the previous visit.
    fn observe(&mut self, state: State, at: Instant) {
        if state == self.current.0 {
            return;
        }
        self.done
            .push((self.current.0, at.duration_since(self.current.1)));
        self.current = (state, at);
    }

    /// Close the last visit.
    fn finish(mut self, at: Instant) -> Vec<(State, Duration)> {
        self.done
            .push((self.current.0, at.duration_since(self.current.1)));
        self.done
    }
}

/// `<dir>/<sub>/<sha256(task id)>`: one directory per task, injective, so
/// two tasks at the same turn id never share a file.
pub(crate) fn task_dir(dir: &Path, sub: &str, task: &TaskId) -> PathBuf {
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
struct ActiveRun {
    run: RunId,
    child: Child,
}

struct Supervision {
    harnesses: Vec<Harness>,
    runner: Option<ActiveRun>,
}

impl Supervision {
    /// Cancel and join before releasing the moderator lock, even on error paths.
    fn stop_runner(&mut self) -> Result<(), String> {
        let Some(mut runner) = self.runner.take() else {
            return Ok(());
        };
        // EOF is cancellation, including when this moderator is killed abruptly.
        drop(runner.child.stdin.take());
        runner
            .child
            .wait()
            .map(|_| ())
            .map_err(|e| format!("runner cleanup: {e}"))
    }

    /// Kill every harness's process group and reap them, reporting the first failure.
    fn kill_harnesses(&mut self) -> Result<(), String> {
        let mut first = Ok(());
        for mut h in self.harnesses.drain(..) {
            let r = kill_tree(&mut h.child);
            first = first.and(r);
        }
        first
    }

    /// Kill the harness of `turn`, if one is running.
    fn kill_harness_of(&mut self, turn: TurnId) -> Result<(), String> {
        match self.harnesses.iter().position(|h| h.turn == turn) {
            Some(i) => kill_tree(&mut self.harnesses.remove(i).child),
            None => Ok(()),
        }
    }
}

impl Drop for Supervision {
    fn drop(&mut self) {
        if let Err(e) = self.kill_harnesses() {
            eprintln!("bbp mod: harness not cleaned up: {e}");
        }
        if let Err(error) = self.stop_runner() {
            eprintln!("bbp mod: runner not cleaned up: {error}");
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
    crate::moderator_health::inspect(&d, &cfg.dir, &cfg.task)?.check()?;
    let started = Instant::now();
    let mut visits = Visits::new(d.state.state, started);
    let mut sup = Supervision {
        harnesses: Vec::new(),
        runner: None,
    };
    let mut launched: HashSet<TurnId> = HashSet::new();
    let mut ran: HashSet<RunId> = HashSet::new();

    // A turn that was live before this loop did anything belongs to an
    // invocation it cannot see or stop. Abort it so its token dies and the
    // replacement turn is launched here. The first tick fires deadlines and
    // grants the first turn of a freshly assigned task; that turn is ours.
    // Gates have no turn and keep waiting.
    let inherited: Vec<TurnId> = d.state.turns.iter().map(|t| t.id).collect();
    d.dispatch(&Command::Tick, now())
        .map_err(|e| format!("{e:?}"))?;
    if !d.state.state.terminal() {
        for turn in inherited {
            if d.state.turns.iter().any(|t| t.id == turn) {
                eprintln!(
                    "bbp mod: turn {} predates this moderator; aborting it",
                    turn.0
                );
                d.dispatch(&Command::AbortTurn { turn: Some(turn) }, now())
                    .map_err(|e| format!("{e:?}"))?;
            }
        }
    }

    loop {
        d.sync().map_err(|e| format!("{e:?}"))?;
        crate::moderator_health::inspect(&d, &cfg.dir, &cfg.task)?.check()?;
        d.dispatch(&Command::Tick, now())
            .map_err(|e| format!("{e:?}"))?;
        let state = d.state.state;
        let seen = Instant::now();
        visits.observe(state, seen);
        if matches!(state, State::Closed | State::Cancelled) {
            sup.kill_harnesses()?;
            sup.stop_runner()?;
            return Ok(Outcome {
                visits: visits.finish(seen),
                state,
                turns_launched: launched.len() as u64,
                runs: ran.len() as u64,
                elapsed: started.elapsed(),
            });
        }
        if started.elapsed() > cfg.max_wall {
            sup.kill_harnesses()?;
            return Err(format!("moderator: wall limit reached in state {state:?}"));
        }

        // A runner process owns its workload and wall timer. Its stdin pipe
        // is this moderator's lease; dropping it cancels and reaps the job.
        if let Some(runner) = sup.runner.as_mut() {
            if let Some(status) = runner
                .child
                .try_wait()
                .map_err(|e| format!("runner wait: {e}"))?
            {
                let run = runner.run;
                sup.runner.take();
                eprintln!("bbp mod: runner for run {} exited: {status}", run.0);
                // A process/transport failure with no report is retryable. The
                // runner's task lock serializes cleanup and recovery attempts.
                if d.state
                    .selected_run()
                    .is_some_and(|r| r.id == run && r.status.is_none())
                {
                    ran.remove(&run);
                }
            }
        }
        if sup.runner.as_ref().is_some_and(|runner| {
            state != State::Test || d.state.selected_run().is_none_or(|r| r.id != runner.run)
        }) {
            sup.stop_runner()?;
        }
        // A selected run with no report yet: the runner's job, once per run.
        let pending = (state == State::Test && sup.runner.is_none())
            .then(|| d.state.selected_run())
            .flatten()
            .filter(|r| r.status.is_none() && !ran.contains(&r.id))
            .map(|r| r.id);
        if let Some(run) = pending {
            ran.insert(run);
            sup.runner = Some(ActiveRun {
                run,
                child: spawn_runner(cfg)?,
            });
        }

        let live: Vec<(TurnId, RoleId)> = d
            .state
            .turns
            .iter()
            .map(|t| (t.id, t.role.clone()))
            .collect();

        // Reap: a harness that exited without ending its turn forfeits it;
        // one whose turn ended under it is killed.
        let mut forfeited = None;
        for h in &mut sup.harnesses {
            let is_live = live.iter().any(|(id, _)| *id == h.turn);
            match h.child.try_wait() {
                Ok(Some(status)) => {
                    eprintln!("bbp mod: harness for turn {} exited: {status}", h.turn.0);
                    forfeited = Some((h.turn, is_live));
                    break;
                }
                Ok(None) if !is_live => {
                    eprintln!("bbp mod: turn {} ended; stopping its harness", h.turn.0);
                    forfeited = Some((h.turn, false));
                    break;
                }
                Ok(None) => {}
                Err(e) => return Err(format!("harness wait: {e}")),
            }
        }
        if let Some((turn, abort)) = forfeited {
            sup.kill_harness_of(turn)?;
            if abort {
                d.dispatch(&Command::AbortTurn { turn: Some(turn) }, now())
                    .map_err(|e| format!("{e:?}"))?;
            }
            continue;
        }

        // Launch: one harness per granted turn.
        for (turn, role) in live {
            if launched.contains(&turn) {
                continue;
            }
            launched.insert(turn);
            match cfg.launchers.for_role(&role) {
                Some(l) => {
                    let principal = d
                        .state
                        .assigned
                        .get(&role)
                        .cloned()
                        .ok_or_else(|| format!("{role} has no principal"))?;
                    let child = spawn(cfg, l, &role, &principal, turn)?;
                    sup.harnesses.push(Harness { turn, child });
                }
                None => eprintln!(
                    "bbp mod: no launcher for {role}; turn {} waits for its deadline",
                    turn.0
                ),
            }
        }
        std::thread::sleep(cfg.poll);
    }
}

/// Explicit owner acknowledgement after repairing a no-progress launcher.
/// Requires the moderator lock and the current displayed BBP revision.
pub fn reset_failures(dir: &Path, task: &TaskId, expected: Rev) -> Result<(), String> {
    let _lock = lock(dir, task)?;
    let d = open_driver(dir, task)?;
    if d.state.rev != expected {
        return Err("stale moderator reset revision; inspect bbp show again".into());
    }
    crate::moderator_health::acknowledge(dir, task, expected)
}

fn spawn_runner(cfg: &Config) -> Result<Child, String> {
    let mut command = Proc::new(&cfg.bbp);
    command
        .arg("runner")
        .arg("--dir")
        .arg(&cfg.dir)
        .arg("--task")
        .arg(&cfg.task.0)
        .arg("--repo-path")
        .arg(&cfg.repo)
        .arg("--work")
        .arg(&cfg.work)
        .arg("--confine")
        .arg(match cfg.confinement {
            Confinement::Sandboxed => "sandbox",
            Confinement::Unconfined => "none",
        })
        .args(["--supervised", "yes"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().map_err(|e| format!("starting runner: {e}"))
}

fn spawn(
    cfg: &Config,
    l: &Launcher,
    role: &RoleId,
    principal: &PrincipalId,
    turn: TurnId,
) -> Result<Child, String> {
    let bbp = &cfg.bbp;
    let mcp = write_mcp_config(&cfg.dir, &cfg.task, bbp, principal, turn)?;
    let role_name = role.0.as_str();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_close_on_change_and_keep_order_and_repeats() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut v = Visits::new(State::Planning, t0);
        v.observe(State::Planning, at(10)); // same state: no new visit
        v.observe(State::Build, at(40));
        v.observe(State::Planning, at(100)); // a revisit is its own entry
        let done = v.finish(at(130));
        assert_eq!(
            done,
            vec![
                (State::Planning, Duration::from_millis(40)),
                (State::Build, Duration::from_millis(60)),
                (State::Planning, Duration::from_millis(30)),
            ]
        );
    }

    #[test]
    fn rejected_plan_and_replanning_remain_separate_observed_visits() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut v = Visits::new(State::Planning, t0);
        v.observe(State::PlanGate, at(10));
        v.observe(State::Planning, at(30)); // rejected draft, then rework
        v.observe(State::PlanGate, at(70));
        v.observe(State::Build, at(80)); // eventual approval
        assert_eq!(
            v.finish(at(90)),
            vec![
                (State::Planning, Duration::from_millis(10)),
                (State::PlanGate, Duration::from_millis(20)),
                (State::Planning, Duration::from_millis(40)),
                (State::PlanGate, Duration::from_millis(10)),
                (State::Build, Duration::from_millis(10)),
            ]
        );
        // Raw arithmetic only: choosing an authoring-time endpoint is a
        // separate policy decision; the first visit omits the 40 ms of rework.
    }

    fn proving(file: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("docs/proving")
            .join(file)
    }

    /// The shipped proving-run configs parse against the schemas they document.
    #[test]
    fn proving_run_configs_match_the_schemas() -> Result<(), String> {
        let agents = Launchers::read_file(&proving("agents.json"))?;
        let roster = crate::admin::read_roster(&proving("roster.json"))?;
        let roles: Vec<&str> = agents.roles().collect();
        assert_eq!(roles.len(), roster.len(), "one launcher per rostered role");
        for r in &roster {
            let l = agents
                .for_role(&r.id)
                .ok_or_else(|| format!("{} has no launcher", r.id))?;
            let joined = l.args.join(" ");
            assert!(
                joined.contains("{mcp_config}"),
                "{} gets the MCP config",
                r.id
            );
            assert!(
                joined.contains("--output-format stream-json --verbose"),
                "{} logs every tool call",
                r.id
            );
            // The launcher's own declaration decides, not the role's name.
            let mcp_only = l.args.windows(2).any(|w| w == ["--tools", ""]);
            assert_eq!(
                mcp_only, !l.builtin_tools,
                "{}: builtin_tools must agree with the launch arguments",
                r.id
            );
        }
        assert!(
            agents
                .roles()
                .any(|id| agents.for_role(&id.into()).is_some_and(|l| l.builtin_tools)),
            "the proving run declares one role with built-in tools"
        );
        let set = crate::profiles::read_file(&proving("profiles.json"))?;
        assert_eq!(set.profiles.len(), 1);
        assert!(
            set.read_roots.iter().any(|r| r.ends_with(".rustup")),
            "toolchain is readable"
        );
        Ok(())
    }
}
