//! `rusty_orch consolidate`: D44's scheduled cleanup (D110, D113, D114,
//! D118). It opens a claims task whose erosion profile runs `erosion check
//! --cleanup`, through `bbp open --mode claims`, when either leg is due:
//! [`EVERY`] accepted claims on one repo's claims tasks since the last
//! cleanup, or HEAD's erosion rising by half of Nano's approved limits since
//! the last cleanup's base. A read-only plan by default.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use orch_cli::{CommandRunner, StdCommand};
use rusty_bbp::{Event, FsStore, Mode, Store, TaskState};
use rusty_serde::json::{self, Value};
use rusty_serde::{Deserialize, Serialize};

use crate::args::ArgsError;

type Result<T, E = String> = std::result::Result<T, E>;

/// D44: a cleanup after this many accepted claims.
pub const EVERY: usize = 5;

/// The fixed consolidate intent (D113).
pub const BRIEF: &str = "Consolidate: pay erosion down (D44). Lower the repo-wide concentration or duplication share without raising the other: split complex functions, merge duplicated code. Change no behaviour; the frozen suite and the holdouts must stay green.\n";

pub const USAGE: &str = "usage: rusty_orch consolidate --dir BBP_DIR --repo NAME --repo-path DIR\n       [--erosion-bin BIN] [--bbp-bin BIN] [--execute --profiles FILE --roster FILE --work DIR\n        [--claims FILE] [--holdout-profiles FILE --holdout-root DIR]]\n\nDefault: read-only JSON plan. A cleanup is due after 5 accepted claims on NAME's\nclaims tasks in BBP_DIR since the last cleanup, or, when Nano's limits are\nrecorded (bbp erosion limits), once HEAD's erosion has risen by half a limit\nsince the last cleanup's base. It opens a consolidate claims task, or skips when\nboth erosion shares are already zero (D44). --execute writes the journal and\nruns bbp open. The profile set's erosion profile must use --cleanup; orch sets\nits --base to the measured HEAD.";

const TIMEOUT: Duration = Duration::from_secs(600);
const STATE: &str = "rusty-orch-consolidate";

/// Explicit inputs: nothing is copied from another task (D113).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub dir: PathBuf,
    pub repo: String,
    pub repo_path: PathBuf,
    pub erosion_bin: String,
    /// Reads Nano's limits in both modes; opens tasks with `--execute`.
    pub bbp_bin: String,
    pub execute: Option<Execute>,
}

/// What `--execute` hands to `bbp open`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Execute {
    pub profiles: PathBuf,
    /// `bbp open` flags passed through unchanged: `--roster`, `--work`, and
    /// optionally `--claims` and the holdout pair.
    pub passthrough: Vec<(&'static str, String)>,
}

/// Parse the command line; execution is never the default.
pub fn options(argv: impl IntoIterator<Item = String>) -> Result<Options, ArgsError> {
    let mut flags: BTreeMap<String, String> = BTreeMap::new();
    let mut execute = false;
    let mut args = argv.into_iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--help" | "-h" => return Err(ArgsError::Help),
            "--execute" if !execute => execute = true,
            f if KNOWN.contains(&f) && !flags.contains_key(f) => {
                let value = args
                    .next()
                    .filter(|v| !v.trim().is_empty() && !v.starts_with('-'))
                    .ok_or_else(|| usage(format!("{flag} needs a value")))?;
                flags.insert(flag, value);
            }
            _ => return Err(usage(format!("unexpected or repeated argument {flag:?}"))),
        }
    }
    let mut take = |f: &str| flags.remove(f);
    let required = |v: Option<String>, f: &str| v.ok_or_else(|| usage(format!("{f} is required")));
    let dir = required(take("--dir"), "--dir")?.into();
    let repo = required(take("--repo"), "--repo")?;
    let repo_path = required(take("--repo-path"), "--repo-path")?.into();
    let erosion_bin = take("--erosion-bin").unwrap_or_else(|| "erosion".into());
    let bbp_bin = take("--bbp-bin").unwrap_or_else(|| "bbp".into());
    let profiles = take("--profiles");
    let mut passthrough = Vec::new();
    for f in [
        "--roster",
        "--work",
        "--claims",
        "--holdout-profiles",
        "--holdout-root",
    ] {
        if let Some(v) = take(f) {
            passthrough.push((f, v));
        }
    }
    if !execute {
        let given = profiles.is_some() || !passthrough.is_empty();
        return match given {
            true => Err(usage("bbp open inputs need --execute".into())),
            false => Ok(Options {
                dir,
                repo,
                repo_path,
                erosion_bin,
                bbp_bin,
                execute: None,
            }),
        };
    }
    let has = |f: &str| passthrough.iter().any(|(k, _)| *k == f);
    for f in ["--roster", "--work"] {
        if !has(f) {
            return Err(usage(format!("--execute needs {f}")));
        }
    }
    if has("--holdout-profiles") != has("--holdout-root") {
        return Err(usage(
            "give --holdout-profiles and --holdout-root together".into(),
        ));
    }
    let execute = Execute {
        profiles: required(profiles, "--profiles")?.into(),
        passthrough,
    };
    Ok(Options {
        dir,
        repo,
        repo_path,
        erosion_bin,
        bbp_bin,
        execute: Some(execute),
    })
}

const KNOWN: [&str; 11] = [
    "--dir",
    "--repo",
    "--repo-path",
    "--erosion-bin",
    "--bbp-bin",
    "--profiles",
    "--roster",
    "--work",
    "--claims",
    "--holdout-profiles",
    "--holdout-root",
];

fn usage(message: String) -> ArgsError {
    ArgsError::Usage(message)
}

/// One BBP task as the count leg sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLog {
    pub id: String,
    pub repo: String,
    pub claims: bool,
    pub open: bool,
    /// The log's length, in events.
    pub len: usize,
    /// Indices of its `ClaimAccepted` events.
    pub accepted: Vec<usize>,
}

impl TaskLog {
    /// Summarize a task's log; `None` for a log with no `TaskOpened`.
    pub fn read(id: &str, events: &[Event]) -> Option<TaskLog> {
        let Some(Event::TaskOpened { repo, mode, .. }) = events.first() else {
            return None;
        };
        let state = TaskState::fold(id.into(), events);
        let accepted = events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, Event::ClaimAccepted { .. }))
            .map(|(i, _)| i)
            .collect();
        Some(TaskLog {
            id: id.into(),
            repo: repo.clone(),
            claims: *mode == Mode::Claims,
            open: !state.state.terminal(),
            len: events.len(),
            accepted,
        })
    }
}

/// orch's write-ahead record for one repo (D110's cleanup marker).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub repo: String,
    pub dir: String,
    /// Each task's log length at the last cleanup open or zero-floor skip.
    pub marker: BTreeMap<String, usize>,
    /// Consolidate tasks orch opened; their claims never count.
    pub cleanups: Vec<String>,
    /// An open written ahead, not yet seen in the store.
    #[rusty_serde(default)]
    pub pending: Option<Pending>,
    /// The half-limit leg's start: the last cleanup's base (D118).
    #[rusty_serde(default)]
    pub last_base: Option<Base>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub task: String,
    pub base: Base,
    pub snapshot: BTreeMap<String, usize>,
}

/// A commit and its repo-wide shares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    pub commit: String,
    pub concentration_bp: i64,
    pub duplication_bp: i64,
}

impl Base {
    fn new(commit: &str, levels: Levels) -> Base {
        Base {
            commit: commit.into(),
            concentration_bp: levels.concentration_bp,
            duplication_bp: levels.duplication_bp,
        }
    }
}

impl Journal {
    /// Settle a pending open against the store: if the task is there, the
    /// open landed and the marker moves; if not, it never happened.
    pub fn settle(mut self, tasks: &[TaskLog]) -> Journal {
        let Some(p) = self.pending.take() else {
            return self;
        };
        if tasks.iter().any(|t| t.id == p.task) {
            self.marker = p.snapshot;
            self.cleanups.push(p.task);
            self.last_base = Some(p.base);
        }
        self
    }

    fn counts(&self, t: &TaskLog) -> bool {
        t.repo == self.repo && t.claims && !self.cleanups.contains(&t.id)
    }
}

/// What the count leg says before any measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Due {
    /// A consolidate task for this repo is still open (D110).
    Busy(String),
    /// Fewer than [`EVERY`] accepted claims since the marker.
    Wait(usize),
    /// A cleanup is due; `snapshot` is the next marker.
    Now {
        accepted: usize,
        snapshot: BTreeMap<String, usize>,
    },
}

/// The count leg: accepted claims on the repo's claims tasks since the
/// marker, across tasks, consolidate tasks excluded (D110).
pub fn due(tasks: &[TaskLog], journal: &Journal) -> Due {
    if let Some(t) = tasks
        .iter()
        .find(|t| t.open && journal.cleanups.contains(&t.id))
    {
        return Due::Busy(t.id.clone());
    }
    let counted: Vec<&TaskLog> = tasks.iter().filter(|t| journal.counts(t)).collect();
    let accepted = counted
        .iter()
        .map(|t| {
            let from = journal.marker.get(&t.id).copied().unwrap_or(0);
            t.accepted.iter().filter(|&&i| i >= from).count()
        })
        .sum();
    match accepted < EVERY {
        true => Due::Wait(accepted),
        false => Due::Now {
            accepted,
            snapshot: snapshot(tasks, journal),
        },
    }
}

/// The next marker: each counted task's log length now.
pub fn snapshot(tasks: &[TaskLog], journal: &Journal) -> BTreeMap<String, usize> {
    tasks
        .iter()
        .filter(|t| journal.counts(t))
        .map(|t| (t.id.clone(), t.len))
        .collect()
}

/// Nano's approved limits, from `bbp erosion limits` (D115).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Limits {
    pub concentration_bp: i64,
    pub duplication_bp: i64,
}

impl Limits {
    /// Parse `bbp erosion limits` output: the newest record, or `null`.
    pub fn parse(line: &str) -> Result<Option<Limits>> {
        let v: Value =
            json::from_str(line.trim()).map_err(|e| format!("bbp erosion limits: {e}"))?;
        if v == Value::Null {
            return Ok(None);
        }
        let bp = |k: &str| {
            v.get(k)
                .and_then(Value::as_i64)
                .ok_or_else(|| format!("bbp erosion limits has no {k}"))
        };
        Ok(Some(Limits {
            concentration_bp: bp("concentration_bp")?,
            duplication_bp: bp("duplication_bp")?,
        }))
    }
}

/// The half-limit leg (D114, D118).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Half {
    /// No limits are recorded; only the count leg runs.
    Inactive,
    /// Limits are recorded but there is no base yet: HEAD becomes it.
    Start,
    /// Neither share has risen by half its limit.
    Below,
    /// A share rose by at least half its limit: a cleanup is due.
    Reached,
}

impl Half {
    fn name(self) -> &'static str {
        match self {
            Half::Inactive => "inactive",
            Half::Start => "start",
            Half::Below => "below",
            Half::Reached => "reached",
        }
    }
}

/// Has either share risen by half its limit since `last`? A share that
/// didn't rise never fires, even against a 0 bp limit.
pub fn half(limits: Option<Limits>, last: Option<&Base>, head: Levels) -> Half {
    let Some(limits) = limits else {
        return Half::Inactive;
    };
    let Some(last) = last else {
        return Half::Start;
    };
    let reached = |rise: i64, limit: i64| rise > 0 && rise.saturating_mul(2) >= limit;
    let c = reached(
        head.concentration_bp - last.concentration_bp,
        limits.concentration_bp,
    );
    let d = reached(
        head.duplication_bp - last.duplication_bp,
        limits.duplication_bp,
    );
    match c || d {
        true => Half::Reached,
        false => Half::Below,
    }
}

/// Repo-wide shares at a commit, in basis points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Levels {
    pub concentration_bp: i64,
    pub duplication_bp: i64,
}

impl Levels {
    /// Read the base shares from an `erosion check` line (D113).
    pub fn parse(line: &str) -> Result<Levels> {
        let v: Value = json::from_str(line.trim()).map_err(|e| format!("erosion output: {e}"))?;
        if let Some(e) = v.get("error").and_then(Value::as_str) {
            return Err(format!("erosion: {e}"));
        }
        let base = |k: &str| {
            v.get(k)
                .and_then(|s| s.get("base"))
                .and_then(Value::as_i64)
                .ok_or_else(|| format!("erosion output has no {k}.base"))
        };
        Ok(Levels {
            concentration_bp: base("concentration_bp")?,
            duplication_bp: base("duplication_bp")?,
        })
    }

    /// D44: with both shares at zero there is nothing a cleanup can lower.
    pub fn zero(self) -> bool {
        self.concentration_bp == 0 && self.duplication_bp == 0
    }
}

/// Copy a profile set, pointing its erosion profile's `--base` at `base`.
/// Refuses a set whose erosion profile isn't in `--cleanup` mode (D113).
pub fn cleanup_profiles(text: &str, base: &str) -> Result<String> {
    let mut set: Value = json::from_str(text).map_err(|e| format!("profiles: {e}"))?;
    let Some(Value::Seq(profiles)) = profile_list(&mut set) else {
        return Err("profiles: no profiles list".into());
    };
    let mut erosion = profiles
        .iter_mut()
        .filter(|p| p.get("name").and_then(Value::as_str) == Some("erosion"));
    let (Some(profile), None) = (erosion.next(), erosion.next()) else {
        return Err("profiles: need exactly one erosion profile".into());
    };
    let Some(Value::Seq(args)) = field(profile, "args") else {
        return Err("profiles: the erosion profile has no args".into());
    };
    if !args.iter().any(|a| a.as_str() == Some("--cleanup")) {
        return Err("profiles: the erosion profile must run --cleanup".into());
    }
    let at = args
        .iter()
        .position(|a| a.as_str() == Some("--base"))
        .filter(|i| i + 1 < args.len())
        .ok_or("profiles: the erosion profile has no --base value")?;
    args[at + 1] = Value::String(base.into());
    json::to_string(&set).map_err(|e| e.to_string())
}

fn profile_list(set: &mut Value) -> Option<&mut Value> {
    field(set, "profiles")
}

fn field<'a>(v: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    let Value::Map(entries) = v else {
        return None;
    };
    entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// The command's JSON report.
#[derive(Debug, Serialize)]
struct Report {
    mode: &'static str,
    action: &'static str,
    accepted: usize,
    every: usize,
    task: Option<String>,
    base: Option<String>,
    levels: Option<Levels>,
    /// Which leg made the cleanup due: `count` or `half`.
    trigger: Option<&'static str>,
    half: &'static str,
    limits: Option<Limits>,
    last_base: Option<Base>,
}

/// Run the command with real processes and the clock.
pub fn run(options: &Options) -> Result<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    run_with(options, &StdCommand, now)
}

/// [`run`] over a given runner and time, for tests.
pub fn run_with(options: &Options, runner: &impl CommandRunner, now: u64) -> Result<String> {
    let state = git(
        runner,
        &options.repo_path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let state = PathBuf::from(state).join(STATE);
    let _lock = match options.execute {
        Some(_) => Some(lock(&state)?),
        None => None,
    };
    let path = state.join("journal.json");
    let journal = load(&path, options)?;
    let tasks = read_tasks(&options.dir)?;
    let mut journal = journal.settle(&tasks);
    let mut report = Report {
        mode: if options.execute.is_some() {
            "execute"
        } else {
            "plan"
        },
        action: "wait",
        accepted: 0,
        every: EVERY,
        task: None,
        base: None,
        levels: None,
        trigger: None,
        half: Half::Inactive.name(),
        limits: None,
        last_base: journal.last_base.clone(),
    };
    let (accepted, count_due) = match due(&tasks, &journal) {
        Due::Busy(task) => {
            report.action = "busy";
            report.task = Some(task);
            return finish(report);
        }
        Due::Wait(n) => (n, false),
        Due::Now { accepted, .. } => (accepted, true),
    };
    report.accepted = accepted;
    let limits = read_limits(runner, options)?;
    report.limits = limits;
    if !count_due && limits.is_none() {
        return finish(report);
    }
    let commit = git(
        runner,
        &options.repo_path,
        &["rev-parse", "--verify", "HEAD^{commit}"],
    )?;
    let levels = measure(runner, options, &commit)?;
    let leg = half(limits, journal.last_base.as_ref(), levels);
    let base = Base::new(&commit, levels);
    report.half = leg.name();
    report.base = Some(commit.clone());
    report.levels = Some(levels);
    report.trigger = match (count_due, leg) {
        (true, _) => Some("count"),
        (false, Half::Reached) => Some("half"),
        _ => None,
    };
    let Some(execute) = &options.execute else {
        report.action = match (report.trigger, levels.zero()) {
            (None, _) => "wait",
            (Some(_), true) => "skip",
            (Some(_), false) => "open",
        };
        return finish(report);
    };
    if report.trigger.is_none() {
        if leg == Half::Start {
            journal.last_base = Some(base);
            save(&path, &journal)?;
        }
        return finish(report);
    }
    let snapshot = snapshot(&tasks, &journal);
    if levels.zero() {
        journal.marker = snapshot;
        journal.last_base = Some(base);
        save(&path, &journal)?;
        report.action = "skipped";
        return finish(report);
    }
    let task = format!("consolidate-{}-{now}", &commit[..12.min(commit.len())]);
    let profiles = cleanup_profiles(&read(&execute.profiles)?, &commit)?;
    journal.pending = Some(Pending {
        task: task.clone(),
        base,
        snapshot,
    });
    save(&path, &journal)?;
    open(runner, options, execute, &state, &task, &commit, &profiles)?;
    let journal = journal.settle(&read_tasks(&options.dir)?);
    save(&path, &journal)?;
    if !journal.cleanups.contains(&task) {
        return Err(format!("bbp open did not create {task}"));
    }
    report.action = "opened";
    report.task = Some(task);
    finish(report)
}

/// D118: Nano's limits through the host, which owns the record's format.
fn read_limits(runner: &impl CommandRunner, options: &Options) -> Result<Option<Limits>> {
    let argv = [&options.bbp_bin, "erosion", "limits", "--dir"]
        .map(String::from)
        .into_iter()
        .chain([options.dir.display().to_string()])
        .collect::<Vec<_>>();
    Limits::parse(&command(runner, &options.repo_path, &argv)?)
}

fn finish(report: Report) -> Result<String> {
    json::to_string(&report)
        .map(|s| s + "\n")
        .map_err(|e| e.to_string())
}

/// D113: the base shares of HEAD, from the bin the profile runs.
fn measure(runner: &impl CommandRunner, options: &Options, base: &str) -> Result<Levels> {
    let argv = [
        &options.erosion_bin,
        "check",
        "--base",
        base,
        "--report-only",
    ]
    .map(String::from);
    let out = command(runner, &options.repo_path, &argv)?;
    Levels::parse(&out)
}

fn open(
    runner: &impl CommandRunner,
    options: &Options,
    execute: &Execute,
    state: &Path,
    task: &str,
    base: &str,
    profiles: &str,
) -> Result<()> {
    let files = state.join(task);
    fs::create_dir_all(&files).map_err(|e| format!("{}: {e}", files.display()))?;
    let (profiles_file, brief_file) = (files.join("profiles.json"), files.join("brief.md"));
    write(&profiles_file, profiles)?;
    write(&brief_file, BRIEF)?;
    let mut argv: Vec<String> = [&options.bbp_bin, "open", "--mode", "claims", "--task", task]
        .map(String::from)
        .into();
    let path = |p: &Path| p.display().to_string();
    let fixed = [
        ("--dir", path(&options.dir)),
        ("--repo", options.repo.clone()),
        ("--brief", path(&brief_file)),
        ("--profiles", path(&profiles_file)),
        ("--base", base.into()),
        ("--repo-path", path(&options.repo_path)),
    ];
    let passed = execute.passthrough.iter().map(|(f, v)| (*f, v.clone()));
    for (flag, value) in fixed.into_iter().chain(passed) {
        argv.extend([flag.to_string(), value]);
    }
    command(runner, &options.repo_path, &argv).map(drop)
}

fn read_tasks(dir: &Path) -> Result<Vec<TaskLog>> {
    if !dir.join("tasks").exists() {
        return Ok(Vec::new());
    }
    let store = FsStore::open(dir).map_err(|e| format!("BBP store: {e:?}"))?;
    let mut tasks = Vec::new();
    for id in store.task_ids() {
        let events = store
            .events(&id, 0)
            .map_err(|e| format!("BBP task {id}: {e:?}"))?;
        tasks.extend(TaskLog::read(&id.0, &events));
    }
    Ok(tasks)
}

fn load(path: &Path, options: &Options) -> Result<Journal> {
    let dir = options.dir.display().to_string();
    if !path.exists() {
        return Ok(Journal {
            repo: options.repo.clone(),
            dir,
            ..Journal::default()
        });
    }
    let journal: Journal = json::from_str(&read(path)?).map_err(|e| format!("journal: {e}"))?;
    if journal.repo != options.repo || journal.dir != dir {
        return Err(format!(
            "{} belongs to repo {:?} in {}",
            path.display(),
            journal.repo,
            journal.dir
        ));
    }
    Ok(journal)
}

fn save(path: &Path, journal: &Journal) -> Result<()> {
    write(path, &json::to_string(journal).map_err(|e| e.to_string())?)
}

fn lock(state: &Path) -> Result<fs::File> {
    fs::create_dir_all(state).map_err(|e| format!("{}: {e}", state.display()))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("lock"))
        .map_err(|e| format!("lock: {e}"))?;
    file.try_lock()
        .map_err(|e| format!("another consolidate run owns this repository: {e}"))?;
    Ok(file)
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<()> {
    rusty_atomic_file::write_private(path, text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn git(runner: &impl CommandRunner, repo: &Path, args: &[&str]) -> Result<String> {
    let argv: Vec<String> = std::iter::once("git")
        .chain(args.iter().copied())
        .map(String::from)
        .collect();
    command(runner, repo, &argv).map(|s| s.trim().to_owned())
}

fn command(runner: &impl CommandRunner, cwd: &Path, argv: &[String]) -> Result<String> {
    let exit = runner
        .run_in(Some(cwd), argv, b"", TIMEOUT, &[])
        .map_err(|e| format!("{}: {e}", argv[0]))?;
    if exit.status != 0 {
        return Err(format!(
            "{} exited {}: {}",
            argv[0],
            exit.status,
            orch_cli::excerpt(&exit.stderr)
        ));
    }
    String::from_utf8(exit.stdout).map_err(|_| format!("{}: output is not UTF-8", argv[0]))
}

#[cfg(test)]
mod tests;
