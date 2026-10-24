//! The moderator loop: `bbp mod`. The core decides who holds the turn; this
//! loop makes it happen. A pass runs when the task's log changes, a child it
//! started exits or a deadline, the wall limit or an end grace comes due, and
//! at least every `poll` (see [`crate::waker`] and `next_wait`). Each pass
//! re-reads the store, fires deadlines, starts the runner for a pending
//! selected run, launches one agent harness for a newly granted turn with a
//! `bbp mcp` binding in its environment, and reaps a harness whose turn has
//! ended. Human gates are states with no turn, so the loop simply waits
//! through them.
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

use crate::confine::{HarnessSandbox, Turn};
use crate::runner::Confinement;
use crate::slots::{self, Class, Slot};
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
    /// Run this role's harness confined, reaching only what this declares
    /// (Linux). See [`crate::confine`].
    #[rusty_serde(default)]
    pub sandbox: Option<HarnessSandbox>,
    /// Keep one provider conversation for the role across its turns. Each
    /// turn is still its own harness process with its own `bbp mcp`, killed
    /// when the turn ends; every one gets the role's session id (`{session}`,
    /// `BBP_SESSION_ID`) and `BBP_SESSION_RESUME` (`yes` once a turn of the
    /// role was launched with it), so it can resume the conversation. The
    /// next turn's harness starts only after the last one has exited. With a
    /// `sandbox`, the session keeps one home across its turns; under a cost
    /// cap, a resumed turn is priced from what its own process counted (see
    /// [`crate::tokens`]). Its args cannot use `{turn}`.
    #[rusty_serde(default)]
    pub session: bool,
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

    /// Every role with its launcher.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Launcher)> {
        self.by_role.iter().map(|(r, l)| (r.as_str(), l))
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
    /// The longest wait between passes. A pass runs sooner when the task's
    /// log changes, a harness or the runner exits, or a turn or request
    /// deadline, the wall limit or an end grace comes due; with no event the
    /// cost cap and slot admission are checked this often.
    pub poll: Duration,
    /// Give up after this long; the task is left where it stands.
    pub max_wall: Duration,
    /// How long a harness whose turn ended (its own terminal call, or a
    /// revocation) may keep running before it is killed, so it can print its
    /// final `result` line. A harness killed before that line leaves its
    /// turn's cost unknown, which fails an evidence run. Cap and wall stops
    /// kill at once.
    pub end_grace: Duration,
    /// Stop the run when its harnesses have cost more than this.
    pub cost_cap: Option<CostCap>,
    /// Machine-wide admission. With slots, a granted turn's harness starts
    /// only when an agent slot is free, and a selected run's runner only when
    /// a validator slot is; until then the turn or run waits.
    pub slots: Option<crate::slots::Slots>,
}

/// A live dollar cap on one run. Checked each poll from the harness output the
/// moderator keeps, so it lags by a poll and by what a harness has printed: it
/// cuts a run short, it does not bound spend exactly. The API key's spend limit
/// is the hard stop.
#[derive(Clone, Debug)]
pub struct CostCap {
    pub usd: f64,
    pub criteria: crate::tokens::Criteria,
    /// The sha256 of the criteria file, recorded with the cap when the task
    /// starts so a resume must use the same prices and the same cap.
    pub criteria_sha256: String,
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
    /// Folding the whole log reproduces the live state at the terminal
    /// observation: the replay a metric is computed from is the run it saw.
    pub replay_equals_live: bool,
    /// Why the run was stopped before it ended, when it was.
    pub stop: Option<Stop>,
}

/// A run the moderator stopped: over its dollar cap, or past its wall limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopKind {
    Cap,
    Wall,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    pub kind: StopKind,
    pub reason: String,
}

/// What a finished or stopped invocation reports.
struct Tally {
    launched: u64,
    ran: u64,
    started: Instant,
}

/// The outcome at `state`: the visits, and whether the log still folds to the
/// live state. A stopped run gets the same account as a finished one.
fn outcome<S: Store>(
    cfg: &Config,
    d: &Driver<S>,
    visits: Visits,
    seen: Instant,
    tally: &Tally,
    stop: Option<Stop>,
) -> Result<Outcome, String> {
    let events = d.store.events(&cfg.task, 0).map_err(|e| format!("{e:?}"))?;
    let replay = TaskState::fold_with(cfg.task.clone(), d.state.master, &events);
    Ok(Outcome {
        replay_equals_live: replay == d.state,
        visits: visits.finish(seen),
        state: d.state.state,
        turns_launched: tally.launched,
        runs: tally.ran,
        elapsed: tally.started.elapsed(),
        stop,
    })
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

/// The provider session id every turn of `role` shares: a UUID keyed by the
/// store's master (see [`crate::master`]) and derived from task and role, so it
/// is stable across moderator restarts of one store and distinct between
/// stores that reuse the same task and role names.
pub fn session_id(master: &[u8; 32], task: &TaskId, role: &RoleId) -> String {
    let label = format!("|bbp-session|{}|{}", task.0, role.0);
    let mut b = Sha256::of(&[master.as_slice(), label.as_bytes()].concat()).0;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = Sha256(b).hex();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Present once a turn was launched with session id `id`.
fn session_marker(dir: &Path, task: &TaskId, id: &str) -> PathBuf {
    task_dir(dir, "agents", task).join(format!("session-{id}.started"))
}

/// Present when `turn`'s harness was launched to resume a provider session:
/// its output's running totals then carry the session's earlier turns.
pub fn resumed_path(dir: &Path, task: &TaskId, turn: TurnId) -> PathBuf {
    task_dir(dir, "agents", task).join(format!("turn-{}.resumed", turn.0))
}

/// The file whose exclusive lock is the right to moderate `task`.
pub fn lock_path(dir: &Path, task: &TaskId) -> PathBuf {
    task_dir(dir, "mod", task).with_extension("lock")
}

/// Whether nothing `task`'s moderators started still runs: no moderator,
/// no runner, and no recorded harness group with a live member. While a
/// group's holder lives its members are killed here; once it is gone they
/// are left alone and the group is not quiet until none remains (see
/// [`crate::groups`]). Only an ended task is quiesced: a live one is its
/// moderator's to resume.
///
/// # Errors
/// When the task is not closed or cancelled, its state cannot be read, or
/// its harnesses ran before their groups were recorded.
pub fn quiesce(dir: &Path, task: &TaskId) -> Result<bool, String> {
    if !cfg!(target_os = "linux") {
        return Err("quiesce is supported on Linux only".into());
    }
    if !open_driver(dir, task)?.state.state.terminal() {
        return Err(format!("task {} has not ended", task.0));
    }
    // Held, so no moderator starts a group while the records are read.
    let Some(_lock) = try_lock(&lock_path(dir, task))? else {
        return Ok(false);
    };
    let mut quiet = try_lock(&crate::runner::lock_path(dir, task))?.is_some();
    for record in crate::groups::records(dir, task)? {
        quiet &= settle(&record)?;
    }
    Ok(quiet)
}

#[cfg(target_os = "linux")]
fn settle(record: &crate::groups::Record) -> Result<bool, String> {
    crate::groups::settle(record)
}

#[cfg(not(target_os = "linux"))]
fn settle(_record: &crate::groups::Record) -> Result<bool, String> {
    Err("harness groups are only cleaned up on Linux".into())
}

/// The exclusive lock on `path` if no one else holds it, `None` if someone does.
fn try_lock(path: &Path) -> Result<Option<File>, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
    }
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
    /// The role whose provider session this harness resumes, if it does.
    session: Option<RoleId>,
    child: Child,
    /// A confined harness's MCP broker, in the harness's process group.
    broker: Option<Child>,
    confined: Option<Turn>,
    /// When the moderator first saw this harness's turn no longer live.
    ended: Option<Instant>,
    /// The moderator's handle on the child's agent slot, dropped when the
    /// harness leaves the list (after it is reaped). The child holds the same
    /// lock through its stdin, so the slot stays taken while it lives.
    _slot: Option<Slot>,
    /// The `bbp slot-hold` process that leads the harness's process group
    /// and keeps the slot while the group has another member, should this
    /// moderator die first.
    hold: Option<Child>,
    /// The group's record, removed once the group is reaped.
    record: Option<PathBuf>,
}

/// Every process the loop started. Dropping it kills the harness and its
/// whole process group, so an error path cleans up like a normal one.
struct ActiveRun {
    run: RunId,
    child: Child,
    _slot: Option<Slot>,
}

/// A free slot of `class` with the stdio that hands it to the child, or
/// `None` when admission is configured and every slot is held. Without
/// admission the child gets a null stdio in its place.
fn admit(cfg: &Config, class: Class) -> Result<Option<(Option<Slot>, Stdio)>, String> {
    let Some(slots) = &cfg.slots else {
        return Ok(Some((None, Stdio::null())));
    };
    let Some(slot) = slots.try_acquire(class)? else {
        return Ok(None);
    };
    let stdio = slot.stdio()?;
    Ok(Some((Some(slot), stdio)))
}

/// How long a stopped harness's group may take to empty before its record
/// is left for quiesce to settle.
const DRAIN: Duration = Duration::from_secs(2);

impl Harness {
    /// The harness's process group: its slot holder's, or its own.
    fn group(&self) -> u32 {
        self.hold.as_ref().map_or(self.child.id(), Child::id)
    }

    /// Kill the harness's process group (its broker and slot holder
    /// included) and reap them all, the group's leader last.
    fn stop(&mut self) -> Result<(), String> {
        let group = self.group();
        let harness = kill_tree(&mut self.child, group);
        let broker = self.broker.as_mut().map_or(Ok(()), |b| kill_tree(b, group));
        // The holder stays unreaped, so the group's id stays this group's,
        // until every member killed with it has exited too.
        let drained = self.record.is_none() || crate::groups::drained(group, DRAIN);
        let hold = self.hold.as_mut().map_or(Ok(()), |h| kill_tree(h, group));
        // Nothing runs in the worktree any more; a leftover is replaced by
        // the next turn of the same number, so failing to remove it is not fatal.
        if let Some(Err(e)) = self.confined.as_ref().map(Turn::finish) {
            eprintln!("bbp mod: turn {} worktree not removed: {e}", self.turn.0);
        }
        // A record is kept while its group may have members; quiesce settles it.
        match self.record.take() {
            Some(record) if drained => {
                if let Err(e) = crate::groups::forget(&record) {
                    eprintln!("bbp mod: group record kept: {e}");
                }
            }
            Some(record) => {
                eprintln!(
                    "bbp mod: group {group} not yet empty; {} kept",
                    record.display()
                );
            }
            None => {}
        }
        harness.and(broker).and(hold)
    }
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
            first = first.and(h.stop());
        }
        first
    }

    /// Let harnesses whose turns have ended exit on their own, for at most
    /// `grace`, then kill the rest.
    fn drain_harnesses(&mut self, grace: Duration) -> Result<(), String> {
        let deadline = Instant::now() + grace;
        loop {
            // Exited harnesses leave the list; on a wait error every one left
            // is still killed before the error is returned.
            let mut failed = None;
            self.harnesses.retain_mut(|h| match exited(&mut h.child) {
                Ok(Some(_)) => {
                    if let Err(e) = h.stop() {
                        failed.get_or_insert(e);
                    }
                    false
                }
                Ok(None) => true,
                Err(e) => {
                    failed.get_or_insert(format!("harness wait: {e}"));
                    true
                }
            });
            if let Some(e) = failed {
                self.kill_harnesses()?;
                return Err(e);
            }
            if self.harnesses.is_empty() || Instant::now() >= deadline {
                return self.kill_harnesses();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Kill the harness of `turn`, if one is running.
    fn kill_harness_of(&mut self, turn: TurnId) -> Result<(), String> {
        match self.harnesses.iter().position(|h| h.turn == turn) {
            Some(i) => self.harnesses.remove(i).stop(),
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

/// The child's exit status once it has exited, without reaping it: an
/// unreaped child keeps its process-group id reserved, so [`kill_tree`] can
/// still kill what it forked without reaching a group that reused the id.
/// (A slot holder, the group's leader, is reaped after the harness.)
#[cfg(target_os = "linux")]
fn exited(child: &mut Child) -> Result<Option<String>, String> {
    use rusty_libc::wait::{waitid, CLD_EXITED, P_PID, WEXITED, WNOHANG, WNOWAIT};
    let pid = i32::try_from(child.id()).map_err(|_| "pid out of range".to_owned())?;
    let info = waitid(P_PID, pid, WEXITED | WNOHANG | WNOWAIT)
        .map_err(|e| format!("harness wait: {e:?}"))?;
    Ok((info.si_pid != 0).then(|| match info.si_code {
        CLD_EXITED => format!("exit status: {}", info.si_status),
        _ => format!("signal: {}", info.si_status),
    }))
}

#[cfg(not(target_os = "linux"))]
fn exited(child: &mut Child) -> Result<Option<String>, String> {
    child
        .try_wait()
        .map(|status| status.map(|s| s.to_string()))
        .map_err(|e| format!("harness wait: {e}"))
}

/// `SIGKILL` process group `group` (the harness and everything it forked),
/// then the child, and reap the child. A group that is already gone is fine.
fn kill_tree(child: &mut Child, group: u32) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let group = i32::try_from(group).map_err(|_| "pid out of range".to_owned())?;
        match rusty_libc::process::killpg(group, rusty_libc::signal::SIGKILL) {
            Ok(()) | Err(rusty_libc::Errno::ESRCH) => {}
            Err(e) => return Err(format!("killpg({group}): {e:?}")),
        }
    }
    // Elsewhere only the child itself is killed.
    #[cfg(not(target_os = "linux"))]
    let _ = group;
    match child.kill() {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {}
        Err(e) => return Err(format!("kill: {e}")),
    }
    child.wait().map(|_| ()).map_err(|e| format!("wait: {e}"))
}

/// [`run_to_stop`], with a stop as the error: the reason.
pub fn run(cfg: &Config) -> Result<Outcome, String> {
    let out = run_to_stop(cfg)?;
    match out.stop {
        Some(s) => Err(s.reason),
        None => Ok(out),
    }
}

/// Run the loop until the task is closed or cancelled, or `max_wall` passes.
/// A run the cap or the wall limit stops is an `Ok` outcome carrying the stop,
/// so the caller can still print what happened; [`run`] turns it into an error.
pub fn run_to_stop(cfg: &Config) -> Result<Outcome, String> {
    check_sessions(cfg).map_err(|e| format!("moderator: refusing to start: {e}"))?;
    // An evidence run (one with a cost cap and its prices) starts only if every
    // role pins a priced model.
    if let Some(cap) = &cfg.cost_cap {
        crate::pins::check_run(&cfg.launchers, &cap.criteria)
            .map_err(|e| format!("moderator: refusing to start: {e}"))?;
    }
    // The lock first: a startup refused for it must not leave a marker behind.
    let _lock = lock(&cfg.dir, &cfg.task)?;
    check_evidence_marker(cfg)?;
    // Before any harness, so every group this task ever has is recorded.
    let recording = crate::groups::enable(&cfg.dir, &cfg.task)?;
    let mut d = open_driver(&cfg.dir, &cfg.task)?;
    crate::moderator_health::inspect(&d, &cfg.dir, &cfg.task)?.check()?;
    let started = Instant::now();
    let mut visits = Visits::new(d.state.state, started);
    let mut sup = Supervision {
        harnesses: Vec::new(),
        runner: None,
    };
    let mut launched: HashSet<TurnId> = HashSet::new();
    let mut waiting: HashSet<TurnId> = HashSet::new();
    let mut ran: HashSet<RunId> = HashSet::new();
    let mut waker = crate::waker::Waker::new(&log_path_for(&cfg.dir, &cfg.task));

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
        // Before the terminal return, so spend that lands between the last poll
        // and the close is still held against the cap.
        if let Some(cap) = &cfg.cost_cap {
            if let Err(reason) = enforce_cap(cfg, &d, cap, false) {
                sup.kill_harnesses()?;
                sup.stop_runner()?;
                d.sync().map_err(|e| format!("{e:?}"))?;
                let tally = Tally {
                    launched: launched.len() as u64,
                    ran: ran.len() as u64,
                    started,
                };
                let stop = Stop {
                    kind: StopKind::Cap,
                    reason,
                };
                return outcome(cfg, &d, visits, seen, &tally, Some(stop));
            }
        }
        let tally = || Tally {
            launched: launched.len() as u64,
            ran: ran.len() as u64,
            started,
        };
        if matches!(state, State::Closed | State::Cancelled) {
            sup.drain_harnesses(cfg.end_grace)?;
            sup.stop_runner()?;
            d.sync().map_err(|e| format!("{e:?}"))?;
            let stop = check_terminal(cfg, &d).err().map(|reason| Stop {
                kind: StopKind::Cap,
                reason,
            });
            return outcome(cfg, &d, visits, seen, &tally(), stop);
        }
        if started.elapsed() > cfg.max_wall {
            sup.kill_harnesses()?;
            sup.stop_runner()?;
            d.sync().map_err(|e| format!("{e:?}"))?;
            let reason = format!("moderator: wall limit reached in state {state:?}");
            let stop = Stop {
                kind: StopKind::Wall,
                reason,
            };
            return outcome(cfg, &d, visits, seen, &tally(), Some(stop));
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
            // Without a validator slot the run stays pending for a later poll.
            if let Some((slot, stdout)) = admit(cfg, Class::Validator)? {
                ran.insert(run);
                sup.runner = Some(ActiveRun {
                    run,
                    child: spawn_runner(cfg, stdout)?,
                    _slot: slot,
                });
            }
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
            match exited(&mut h.child) {
                Ok(Some(status)) => {
                    eprintln!("bbp mod: harness for turn {} exited: {status}", h.turn.0);
                    if let Some(why) = h.confined.as_ref().and_then(Turn::setup_error) {
                        eprintln!(
                            "bbp mod: turn {} was not confined, so nothing ran: {why}",
                            h.turn.0
                        );
                    }
                    forfeited = Some((h.turn, is_live));
                    break;
                }
                Ok(None) if !is_live => {
                    let ended = *h.ended.get_or_insert_with(Instant::now);
                    if ended.elapsed() >= cfg.end_grace {
                        eprintln!("bbp mod: turn {} ended; stopping its harness", h.turn.0);
                        forfeited = Some((h.turn, false));
                        break;
                    }
                }
                Ok(None) => {}
                Err(e) => return Err(e),
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
            let Some(l) = cfg.launchers.for_role(&role) else {
                launched.insert(turn);
                eprintln!(
                    "bbp mod: no launcher for {role}; turn {} waits for its deadline",
                    turn.0
                );
                continue;
            };
            // One process per session at a time: the next turn waits until
            // the last one's harness has exited or been stopped.
            if l.session
                && sup
                    .harnesses
                    .iter()
                    .any(|h| h.session.as_ref() == Some(&role))
            {
                if waiting.insert(turn) {
                    eprintln!(
                        "bbp mod: turn {} waits for {role}'s last harness to end",
                        turn.0
                    );
                }
                continue;
            }
            // Without an agent slot the turn waits for a later poll, or its deadline.
            let Some((slot, stdin)) = admit(cfg, Class::Agent)? else {
                if waiting.insert(turn) {
                    eprintln!("bbp mod: turn {} waits for an agent slot", turn.0);
                }
                continue;
            };
            launched.insert(turn);
            let principal = d
                .state
                .assigned
                .get(&role)
                .cloned()
                .ok_or_else(|| format!("{role} has no principal"))?;
            // The holder is up, and its group recorded, before the harness, so
            // neither the slot nor the group's cleanup ever rests on this
            // moderator alone.
            // On Linux every harness has a holder, so its group is recorded
            // with or without admission.
            let mut hold = (slot.is_some() || cfg!(target_os = "linux"))
                .then(|| slots::holder(&cfg.bbp, slot.as_ref()))
                .transpose()?;
            let group = hold.as_ref().map(Child::id);
            let record = match group
                .filter(|_| recording)
                .map(|g| crate::groups::record(&cfg.dir, &cfg.task, g))
            {
                Some(Err(e)) => {
                    if let Some(Err(k)) = hold.as_mut().map(|h| kill_tree(h, h.id())) {
                        eprintln!("bbp mod: slot holder for turn {} not stopped: {k}", turn.0);
                    }
                    return Err(e);
                }
                Some(Ok(record)) => Some(record),
                None => None,
            };
            let facts = crate::launchers::Facts::of(&d.state, &principal, turn);
            let launched = match spawn(cfg, l, &role, &principal, turn, stdin, group, &facts) {
                Ok(launched) => launched,
                Err(e) => {
                    if let Some(Err(k)) = hold.as_mut().map(|h| kill_tree(h, h.id())) {
                        eprintln!("bbp mod: slot holder for turn {} not stopped: {k}", turn.0);
                    }
                    return Err(e);
                }
            };
            // The group has its member: the holder may start watching it.
            if let Some(h) = hold.as_mut() {
                drop(h.stdin.take());
            }
            sup.harnesses.push(Harness {
                turn,
                session: l.session.then(|| role.clone()),
                child: launched.child,
                broker: launched.broker,
                confined: launched.confined,
                ended: None,
                _slot: slot,
                hold,
                record,
            });
        }
        let children: Vec<u32> = sup
            .harnesses
            .iter()
            .map(|h| h.child.id())
            .chain(sup.runner.as_ref().map(|r| r.child.id()))
            .collect();
        let graces = sup
            .harnesses
            .iter()
            .filter_map(|h| h.ended)
            .map(|ended| cfg.end_grace.saturating_sub(ended.elapsed()));
        let wall = cfg.max_wall.saturating_sub(started.elapsed());
        let timeout = next_wait(cfg.poll, now(), d.state.next_deadline(), wall, graces);
        waker.wait(&children, timeout)?;
    }
}

/// A session launcher's args name the session, not the turn.
fn check_sessions(cfg: &Config) -> Result<(), String> {
    for (role, l) in cfg.launchers.iter().filter(|(_, l)| l.session) {
        if std::iter::once(&l.program)
            .chain(&l.args)
            .any(|a| a.contains("{turn}"))
        {
            return Err(format!(
                "{role}: a session launcher names its session; drop {{turn}}"
            ));
        }
    }
    Ok(())
}

/// How long a pass may wait for an event: until the nearest turn or request
/// deadline, the wall limit or the end of a grace comes due, and never longer
/// than `poll`. A quiet harness produces no event, so these must wake the loop
/// themselves. The wall check is strict (`>`), hence the extra millisecond.
fn next_wait(
    poll: Duration,
    now: Time,
    deadline: Option<Time>,
    wall_left: Duration,
    graces: impl IntoIterator<Item = Duration>,
) -> Duration {
    let until_deadline = deadline.map(|d| Duration::from_millis(d.0.saturating_sub(now.0)));
    let past_wall = wall_left + Duration::from_millis(1);
    graces
        .into_iter()
        .chain(until_deadline)
        .fold(poll.min(past_wall), Duration::min)
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

/// The end of a run is not exempt from the cap: output that arrived since the
/// last poll, or from a model with no price, must not close as a success.
/// Called once the harnesses are stopped, with the final usage on disk; a
/// no-op for a task that is not in a terminal state, whose cap is polled.
pub fn check_terminal<S: Store>(cfg: &Config, d: &Driver<S>) -> Result<(), String> {
    match (&cfg.cost_cap, d.state.state) {
        (Some(cap), State::Closed | State::Cancelled) => enforce_cap(cfg, d, cap, true),
        _ => Ok(()),
    }
}

/// Fail closed: over the cap, a model that cannot be priced, or a harness log
/// the cap cannot read, stops the run. A harness that has printed nothing yet
/// is let alone while the run is live (it may be starting) but not at the end,
/// and a log with no stream-json in it never is: the cap would be blind to it.
fn enforce_cap<S: Store>(
    cfg: &Config,
    d: &Driver<S>,
    cap: &CostCap,
    terminal: bool,
) -> Result<(), String> {
    use crate::tokens::LogState;
    let events = d.store.events(&cfg.task, 0).map_err(|e| format!("{e:?}"))?;
    let read = if terminal {
        crate::tokens::of
    } else {
        crate::tokens::of_live
    };
    let t = read(&cfg.dir, &cfg.task, &events, Some(&cap.criteria))
        .map_err(|e| format!("moderator: cannot enforce the cost cap: {e}"))?;
    // The floor: a complete turn's cost, and for a turn still running what its
    // printed messages show. It can only rise, so crossing the cap on it is
    // crossing the cap.
    let usd = t.usd_floor.unwrap_or(0.0);
    if usd > cap.usd {
        return Err(format!(
            "moderator: cost cap reached (${usd:.4} over ${:.4})",
            cap.usd
        ));
    }
    // A turn granted but never launched has no log yet; at the end that is a
    // turn nothing can say the cost of.
    if terminal && t.without_output > 0 {
        return Err(format!(
            "moderator: cannot enforce the cost cap: {} granted turn(s) have no log",
            t.without_output
        ));
    }
    for turn in &t.turns {
        let blind = match turn.log {
            LogState::Readable => false,
            LogState::Unreadable => true,
            // Not printed anything the cap can read yet: fine while the run is
            // live (a harness takes a moment to start), blind at the end.
            LogState::Empty | LogState::Started => terminal,
        };
        if blind {
            return Err(format!(
                "moderator: cannot enforce the cost cap: turn {} ({}) printed no stream-json the cap can read",
                turn.turn, turn.role
            ));
        }
    }
    Ok(())
}

/// A task first started with a cap is an evidence run: every later start or
/// resume must carry the same cap and the same price file, or it is refused,
/// so a cap cannot be dropped by restarting.
fn check_evidence_marker(cfg: &Config) -> Result<(), String> {
    let file = task_dir(&cfg.dir, "evidence", &cfg.task).with_extension("json");
    let recorded = std::fs::read_to_string(&file).ok();
    match (&cfg.cost_cap, recorded) {
        (None, None) => Ok(()),
        (None, Some(_)) => Err(
            "moderator: refusing to start: this task was started as an evidence run; \
             resume it with the same --max-usd and --criteria"
                .into(),
        ),
        (Some(cap), None) => {
            // Only a task nothing has run on is an evidence run: a segment run
            // without a cap, or without the pre-run checks, would be counted.
            let d = open_driver(&cfg.dir, &cfg.task)?;
            let events = d.store.events(&cfg.task, 0).map_err(|e| format!("{e:?}"))?;
            if events
                .iter()
                .any(|e| matches!(e, Event::TurnGranted { .. }))
            {
                return Err("moderator: refusing to start: this task already ran without a cap, so it is not an evidence run; open a new task for the run".into());
            }
            write_evidence_marker(&cfg.dir, &cfg.task, cap.usd, &cap.criteria_sha256)
        }
        (Some(cap), Some(text)) if text == marker_text(cap.usd, &cap.criteria_sha256) => Ok(()),
        (Some(_), Some(_)) => Err(
            "moderator: refusing to start: this task was started with a different --max-usd or \
             price file"
                .into(),
        ),
    }
}

/// Record that the task is an evidence run under this cap and price file.
pub fn write_evidence_marker(
    dir: &Path,
    task: &TaskId,
    usd: f64,
    criteria_sha256: &str,
) -> Result<(), String> {
    let file = task_dir(dir, "evidence", task).with_extension("json");
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, marker_text(usd, criteria_sha256))
        .map_err(|e| format!("{}: {e}", file.display()))
}

fn marker_text(usd: f64, criteria_sha256: &str) -> String {
    format!("{{\"max_usd\":{usd},\"criteria_sha256\":\"{criteria_sha256}\"}}\n")
}

/// The run was started as an evidence run under exactly this cap and price
/// file. A run started without a cap has no marker, so it cannot be counted.
pub fn require_evidence_marker(
    dir: &Path,
    task: &TaskId,
    usd: f64,
    criteria_sha256: &str,
) -> Result<(), String> {
    let file = task_dir(dir, "evidence", task).with_extension("json");
    match std::fs::read_to_string(&file) {
        Ok(t) if t == marker_text(usd, criteria_sha256) => Ok(()),
        Ok(_) => Err(format!(
            "{}: started under a different cap or price file",
            task.0
        )),
        Err(_) => Err(format!(
            "{}: not started as an evidence run (no cap)",
            task.0
        )),
    }
}

/// `stdout` is the validator slot when admission is configured: the runner
/// then holds it until it exits.
fn spawn_runner(cfg: &Config, stdout: Stdio) -> Result<Child, String> {
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
        .stdout(stdout)
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().map_err(|e| format!("starting runner: {e}"))
}

/// A started harness: its process, and for a confined one the broker that
/// serves its MCP relay and the places of its turn.
struct Launched {
    child: Child,
    broker: Option<Child>,
    confined: Option<Turn>,
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    cfg: &Config,
    l: &Launcher,
    role: &RoleId,
    principal: &PrincipalId,
    turn: TurnId,
    stdin: Stdio,
    group: Option<u32>,
    facts: &crate::launchers::Facts,
) -> Result<Launched, String> {
    let bbp = &cfg.bbp;
    let session = match l.session {
        true => Some(session_id(&crate::master::load(&cfg.dir)?, &cfg.task, role)),
        false => None,
    };
    let marker = session
        .as_deref()
        .map(|id| session_marker(&cfg.dir, &cfg.task, id));
    let resume = marker.as_ref().is_some_and(|m| m.exists());
    let confined = l
        .sandbox
        .as_ref()
        .map(|sandbox| Turn::prepare(cfg, sandbox, turn, session.as_deref(), facts))
        .transpose()?;
    let mcp = match &confined {
        Some(t) => t.mcp_config.clone(),
        None => write_mcp_config(&cfg.dir, &cfg.task, bbp, principal, turn)?,
    };
    let role_name = role.0.as_str();
    let fill = |a: &str| {
        a.replace("{dir}", &cfg.dir.to_string_lossy())
            .replace("{task}", &cfg.task.0)
            .replace("{principal}", &principal.0)
            .replace("{turn}", &turn.0.to_string())
            .replace("{role}", role_name)
            .replace("{bbp}", &bbp.to_string_lossy())
            .replace("{mcp_config}", &mcp.to_string_lossy())
            .replace("{session}", session.as_deref().unwrap_or("{session}"))
    };
    let log_path = log_path(&cfg.dir, &cfg.task, turn);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let log = File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    // Recorded before the harness runs, so its cost is never read as a
    // fresh process's.
    if resume {
        let path = resumed_path(&cfg.dir, &cfg.task, turn);
        File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let broker_log = log.try_clone().map_err(|e| e.to_string())?;
    eprintln!("bbp mod: launching {role_name} for turn {}", turn.0);
    let mut env: Vec<(String, String)> = vec![
        ("BBP_TASK".into(), cfg.task.0.clone()),
        ("BBP_PRINCIPAL".into(), principal.0.clone()),
        ("BBP_TURN".into(), turn.0.to_string()),
        ("BBP_ROLE".into(), role_name.into()),
        ("BBP_BIN".into(), bbp.to_string_lossy().into_owned()),
        ("BBP_MCP_CONFIG".into(), mcp.to_string_lossy().into_owned()),
    ];
    if let Some(id) = &session {
        env.push(("BBP_SESSION_ID".into(), id.clone()));
        env.push((
            "BBP_SESSION_RESUME".into(),
            if resume { "yes" } else { "no" }.into(),
        ));
    }
    // An evidence run pins the cache lifetime for the main agent and for
    // subagents: they default to different lifetimes, and the price file
    // prices cache writes at the 1-hour rate.
    if cfg.cost_cap.is_some() {
        for var in crate::tokens::CACHE_TTL_ENV {
            env.push((var.into(), crate::tokens::PINNED_CACHE_TTL.into()));
        }
        // A subagent, if one starts despite the launcher's restrictions, runs
        // on the role's own model, which is priced.
        if let Ok(Some(model)) = crate::pins::pinned_model(l) {
            env.push(("CLAUDE_CODE_SUBAGENT_MODEL".into(), model));
        }
    }
    let program = fill(&l.program);
    let args: Vec<String> = l.args.iter().map(|a| fill(a)).collect();
    let mut cmd = match (&l.sandbox, &confined) {
        (Some(sandbox), Some(t)) => {
            // Its own MCP channel is the relay; the task directory is out of reach.
            env.push((
                crate::broker::RELAY_ENV.into(),
                t.pipes.to_string_lossy().into_owned(),
            ));
            env.push((
                crate::launchers::FACTS_ENV.into(),
                t.facts.to_string_lossy().into_owned(),
            ));
            t.command(cfg, sandbox, program, args, env)?
        }
        _ => {
            let mut cmd = Proc::new(program);
            cmd.args(args).env("BBP_DIR", &cfg.dir).envs(env);
            cmd
        }
    };
    cmd.stdin(stdin)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err));
    // Its slot holder's process group, or its own, so everything the
    // harness forks dies with it.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(
        &mut cmd,
        i32::try_from(group.unwrap_or(0)).map_err(|_| "pid out of range".to_owned())?,
    );
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", l.program))?;
    let group = group.unwrap_or(child.id());
    // Later turns of the session resume what this one starts.
    if let Some(marker) = &marker {
        if let Err(e) = File::create(marker) {
            kill_tree(&mut child, group)?;
            return Err(format!("{}: {e}", marker.display()));
        }
    }
    let broker = match &confined {
        None => None,
        Some(t) => match t.broker(
            cfg,
            principal,
            turn,
            child.id(),
            group,
            Stdio::from(broker_log),
        ) {
            Ok(broker) => Some(broker),
            Err(e) => {
                kill_tree(&mut child, group)?;
                return Err(e);
            }
        },
    };
    Ok(Launched {
        child,
        broker,
        confined,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_ends_at_whatever_comes_due_first() {
        let ms = Duration::from_millis;
        let poll = ms(60_000);
        let now = Time(10_000);
        let wall = ms(120_000);
        let none: [Duration; 0] = [];
        // Nothing due: the poll.
        assert_eq!(next_wait(poll, now, None, wall, none), poll);
        // A turn deadline 1 s away.
        assert_eq!(
            next_wait(poll, now, Some(Time(11_000)), wall, none),
            ms(1_000)
        );
        // A deadline already passed: no wait, the next tick fires it.
        assert_eq!(next_wait(poll, now, Some(Time(9_000)), wall, none), ms(0));
        // The wall limit, one millisecond past it since its check is strict.
        assert_eq!(next_wait(poll, now, None, ms(2_000), none), ms(2_001));
        // The shortest remaining grace.
        assert_eq!(
            next_wait(poll, now, None, wall, [ms(700), ms(300)]),
            ms(300)
        );
        // A short poll still bounds everything.
        assert_eq!(
            next_wait(ms(50), now, Some(Time(11_000)), wall, [ms(700)]),
            ms(50)
        );
    }

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
