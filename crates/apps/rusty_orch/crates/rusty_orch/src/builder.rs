//! `rusty_orch builder run` (#1318 S5): one builder session on one claim,
//! the D167 toy run's entry point. Admits under the store lock, then runs
//! `orch_bbp::builder`'s sequence over [`live`]'s ports and prints the
//! session row as one JSON line.
//!
//! The implementation defaults are D189: the extra flags, the holder's
//! deadline past orch's own end, the fd-4 shell redirect, no `recover` here
//! (#1320's loop owns it), the built-in prompt and the `claude-builder`
//! seat. Until #1571's `resolve` lands, the `claude-builder` profile is
//! hardcoded here: program `claude-code-acp`, `jail` isolation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use orch_lens::builder::{End, Phase, Session};
use orch_lens::card::Card;
use orch_store::{Store, StoreError};
use rusty_bbp::MsgId;
use rusty_json::{Map, Value};

use crate::args::ArgsError;

#[cfg(target_os = "linux")]
pub mod live;

/// The builder's BBP seat and mux `agent` (#1571 §2).
pub const AGENT: &str = "claude-builder";
/// The ACP adapter the jail execs, from the `claude-builder` profile.
pub const PROGRAM: &str = "claude-code-acp";
/// The file the agent writes to submit (D188); never part of the diff.
pub const SUBMIT: &str = ".lel/submit";
/// How long a save waits for another process's store lock.
const LOCK_WAIT: Duration = Duration::from_secs(30);

pub const USAGE: &str = "usage: rusty_orch builder run <config.json> --bbp <bbp binary> --dir <bbp dir> --task <task>\n           --claim <id> --state <orch state dir> --repo <git repo> --base <commit>\n           --cgroup-parent <dir> --slots <dir> --mux-ctl <rusty-mux-ctl> [--mux-socket <path>]\n           [--jail <lel-jail>] [--read <dir>]...\n\nRun one builder session on one claim (#1318): admit it, make its worktree from --base,\nopen its cgroup below --cgroup-parent, take an agent slot from --slots, launch\n`lel-jail` running `claude-code-acp` in a mux block, run one ACP turn, end it and post\nits candidate only when it wrote .lel/submit with a non-empty diff (D188). Prints the\nsession row as one JSON line. --cgroup-parent is the board's `lel-store-<scope>`\ncgroup (`bbp cgroup-scope`), delegated to this user. Each --read adds a read root to the jail. Linux only.\nCalls the Claude CLI under its own subscription login.";

/// What to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub config: PathBuf,
    pub bbp: PathBuf,
    pub dir: PathBuf,
    pub task: String,
    pub claim: MsgId,
    pub state: PathBuf,
    pub repo: PathBuf,
    pub base: String,
    pub cgroup_parent: PathBuf,
    pub slots: PathBuf,
    pub mux_ctl: PathBuf,
    pub mux_socket: Option<PathBuf>,
    pub jail: PathBuf,
    /// Read roots beyond the system ones and `~/.claude`.
    pub read: Vec<PathBuf>,
}

const FLAGS: [&str; 13] = [
    "--bbp",
    "--dir",
    "--task",
    "--claim",
    "--state",
    "--repo",
    "--base",
    "--cgroup-parent",
    "--slots",
    "--mux-ctl",
    "--mux-socket",
    "--jail",
    "--read",
];

/// Parse `run <config> --bbp … --claim … --mux-ctl …`.
///
/// # Errors
///
/// [`ArgsError::Help`] for `-h`, else a usage error naming the problem.
pub fn options(argv: impl IntoIterator<Item = String>) -> Result<Options, ArgsError> {
    let mut args = argv.into_iter();
    match args.next().as_deref() {
        Some("run") => {}
        Some("-h" | "--help") | None => return Err(ArgsError::Help),
        Some(other) => {
            return Err(ArgsError::Usage(format!(
                "unknown builder command {other:?}"
            )))
        }
    }
    let mut config = None;
    let mut flags: BTreeMap<&'static str, String> = BTreeMap::new();
    let mut read = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            return Err(ArgsError::Help);
        }
        let Some(flag) = FLAGS.iter().find(|f| **f == arg) else {
            if arg.starts_with('-') || config.is_some() {
                return Err(ArgsError::Usage(format!("unexpected argument {arg:?}")));
            }
            config = Some(PathBuf::from(arg));
            continue;
        };
        let value = args
            .next()
            .filter(|v| !v.is_empty() && !v.starts_with('-'))
            .ok_or_else(|| ArgsError::Usage(format!("{arg} needs a value")))?;
        if *flag == "--read" {
            read.push(PathBuf::from(value));
        } else if flags.insert(flag, value).is_some() {
            return Err(ArgsError::Usage(format!("duplicate {arg}")));
        }
    }
    let mut take = |f: &str| flags.remove(f);
    let required =
        |v: Option<String>, f: &str| v.ok_or_else(|| ArgsError::Usage(format!("{f} is required")));
    let claim = required(take("--claim"), "--claim")?;
    let claim = claim
        .parse()
        .map(MsgId)
        .map_err(|_| ArgsError::Usage(format!("--claim {claim:?} is not a message id")))?;
    Ok(Options {
        config: config.ok_or_else(|| ArgsError::Usage("a lens config is required".into()))?,
        bbp: required(take("--bbp"), "--bbp")?.into(),
        dir: required(take("--dir"), "--dir")?.into(),
        task: required(take("--task"), "--task")?,
        claim,
        state: required(take("--state"), "--state")?.into(),
        repo: required(take("--repo"), "--repo")?.into(),
        base: required(take("--base"), "--base")?,
        cgroup_parent: required(take("--cgroup-parent"), "--cgroup-parent")?.into(),
        slots: required(take("--slots"), "--slots")?.into(),
        mux_ctl: required(take("--mux-ctl"), "--mux-ctl")?.into(),
        mux_socket: take("--mux-socket").map(PathBuf::from),
        jail: take("--jail").map_or_else(|| PathBuf::from("lel-jail"), PathBuf::from),
        read,
    })
}

/// The builder's one prompt (D189 item 5).
pub fn prompt(card: &Card, worktree: &Path) -> String {
    format!(
        "You are a builder for design claim {claim}:\n\n{text}\n\n\
         Build it in this worktree ({wt}), the only place you can write. Build and run the \
         visible tests as often as you like; those runs cost nothing and are not evidence.\n\n\
         When the change is ready, write the file `{SUBMIT}` (any content) and end your turn: \
         orch then posts your diff as the claim's candidate. If you give up, end your turn \
         without writing it and nothing is posted.",
        claim = card.id,
        text = card.text,
        wt = worktree.display(),
    )
}

/// The session row as one JSON object, with the notes the run made.
pub fn row(session: &Session, notes: &[String]) -> Value {
    let mut m = Map::new();
    m.insert("claim".into(), Value::from(session.claim.0));
    m.insert("n".into(), Value::from(u64::from(session.n)));
    m.insert("started".into(), Value::from(session.started));
    let (state, at, candidate) = match session.phase {
        Phase::Admitted => ("admitted", None, None),
        Phase::Running => ("running", None, None),
        Phase::Ended { end, at } => match end {
            End::Stored(a) => ("stored", Some(at), Some(a.0)),
            End::Failed => ("failed", Some(at), None),
            End::Aborted => ("aborted", Some(at), None),
        },
    };
    m.insert("state".into(), Value::from(state));
    m.insert("ended".into(), at.map_or(Value::Null, Value::from));
    m.insert(
        "candidate".into(),
        candidate.map_or(Value::Null, Value::from),
    );
    m.insert("tokens_in".into(), Value::from(session.tokens_in));
    m.insert("tokens_out".into(), Value::from(session.tokens_out));
    let notes = notes.iter().map(|n| Value::from(n.as_str())).collect();
    m.insert("notes".into(), Value::Array(notes));
    Value::Object(m)
}

/// Run `f` on the store at `state`, waiting up to [`LOCK_WAIT`] while
/// another process holds it. The lock is held only for `f`.
///
/// # Errors
///
/// When the store stays locked, cannot be opened, or `f` fails.
pub fn with_store<T>(
    state: &Path,
    f: impl FnOnce(&mut Store) -> Result<T, String>,
) -> Result<T, String> {
    with_store_waiting(state, LOCK_WAIT, f)
}

/// [`with_store`], waiting up to `wait` for the lock.
fn with_store_waiting<T>(
    state: &Path,
    wait: Duration,
    f: impl FnOnce(&mut Store) -> Result<T, String>,
) -> Result<T, String> {
    let start = std::time::Instant::now();
    loop {
        match Store::open(state) {
            Ok(mut store) => return f(&mut store),
            Err(StoreError::Locked(_)) if start.elapsed() < wait => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// `sessions` with `session` in place of its own row, or added.
pub fn replace(mut sessions: Vec<Session>, session: &Session) -> Vec<Session> {
    match sessions
        .iter_mut()
        .find(|s| s.claim == session.claim && s.n == session.n)
    {
        Some(slot) => *slot = session.clone(),
        None => sessions.push(session.clone()),
    }
    sessions
}

/// Run `o`; Linux only.
///
/// # Errors
///
/// A message when the session could not be admitted or started at all.
#[cfg(target_os = "linux")]
pub fn run(o: &Options) -> Result<String, String> {
    live::run(o)
}

/// Run `o`; Linux only.
///
/// # Errors
///
/// Always: builders need Linux (cgroups, `pidfd_getfd`, the mux).
#[cfg(not(target_os = "linux"))]
pub fn run(_: &Options) -> Result<String, String> {
    Err("builder run needs Linux".into())
}

#[cfg(test)]
mod tests;
