//! #1318 S5: one builder session's launch sequence (build plan §6, slice
//! design §S5), with the worktree, the holder, the mux, ACP and BBP behind
//! one port so the order is tested with fakes. The adapter is the caller's
//! (`rusty_orch builder run`).
//!
//! After admission (the caller's, under the store lock), [`run`] takes an
//! admitted session through:
//!
//! 1. **Prepare**: worktree, `bbp cgroup-open`, `jail.json`.
//! 2. **Hold**: the cgroup holder takes a slot and orch copies it for fd 3.
//!    A refusal ends the session `failed` with nothing launched.
//! 3. **Open**: the mux block; the row moves to `running`.
//! 4. **Run**: one ACP turn, bounded by the wall clock.
//! 5. **End**: `term`, the grace, orch's own `cgroup.kill` if anything is
//!    left, then `release` (D104 §2 step 7). The holder's deadline is only
//!    the backstop for a dead orch.
//! 6. **Submit** (D188): only after an `end_turn`, and only when the agent
//!    wrote `.lel/submit` and the diff is not empty, does a candidate reach
//!    BBP; the row ends `stored`. Anything else ends `failed` with nothing
//!    posted or debited. A lost mux ends it `aborted`.
//!
//! The row is saved after each phase change. A failed save is reported, not
//! fatal: stopping the agent matters more than the row.

use std::fmt;
use std::time::Duration;

use orch_lens::builder::{End, Phase, Session};
use rusty_bbp::ArtId;

use crate::acp::Turn;

/// The grace a block gets after `term` before orch kills its cgroup (D104).
pub const GRACE: Duration = Duration::from_secs(60);
/// How long past orch's own end (wall plus grace) the holder's deadline
/// kill waits, so it fires only when orch is gone.
pub const HOLDER_MARGIN: Duration = Duration::from_secs(30);

/// How the turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    /// The agent answered the prompt.
    Turn(Turn),
    /// The wall clock ran out first.
    Wall,
    /// The mux connection was lost: the session is aborted, not failed.
    Lost(String),
    /// The turn failed (a closed socket, a malformed line, an I/O error).
    Failed(String),
}

/// Why opening the block failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The mux refused the launch, or the launch could not be sent.
    Failed(String),
    /// The mux connection was lost.
    Lost(String),
}

/// Everything the sequence does to the world. Each step names its port.
pub trait Ports {
    /// Now, in ms since the epoch.
    fn now_ms(&mut self) -> u64;
    /// Save this session's row.
    fn save(&mut self, session: &Session) -> Result<(), String>;
    /// Worktree, cgroup and `jail.json` for `session`.
    fn prepare(&mut self, session: &Session) -> Result<(), String>;
    /// Start the holder with its deadline kill at `deadline_ms` and take
    /// its slot for fd 3.
    fn hold(&mut self, deadline_ms: u64) -> Result<(), String>;
    /// Launch the block (and its viewer).
    fn open(&mut self) -> Result<(), Refused>;
    /// Run the ACP turn until it ends or `deadline_ms` passes.
    fn turn(&mut self, deadline_ms: u64) -> Ran;
    /// Send the block `term`.
    fn term(&mut self) -> Result<(), String>;
    /// Wait up to `grace` for the cgroup to empty; true when it did.
    fn empty_within(&mut self, grace: Duration) -> Result<bool, String>;
    /// orch's own kill: write `cgroup.kill` and wait for it to empty.
    fn kill(&mut self) -> Result<(), String>;
    /// `release` the block and let the holder go.
    fn release(&mut self) -> Result<(), String>;
    /// D188: post the candidate if `.lel/submit` was written and the diff
    /// is not empty; `None` when it was not.
    fn submit(&mut self) -> Result<Option<ArtId>, String>;
}

/// A session's final row and what went wrong on the way that did not stop
/// it (a failed save, a refused `term`, a failed kill).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub session: Session,
    pub notes: Vec<String>,
}

/// A phase change the session's own rules refused: a bug in the sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broken(pub String);

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Broken {}

/// Run admitted `session` to its end with a wall clock of `wall`.
///
/// # Errors
///
/// [`Broken`] when `session` is not admitted.
pub fn run<P: Ports>(ports: &mut P, session: Session, wall: Duration) -> Result<Outcome, Broken> {
    if session.phase != Phase::Admitted {
        return Err(Broken(format!(
            "builder session {} #{} is not admitted",
            session.claim, session.n
        )));
    }
    let mut notes = Vec::new();
    let deadline = session.started.saturating_add(millis(wall));
    if let Err(e) = ports.prepare(&session) {
        notes.push(format!("prepare: {e}"));
        return finish(ports, session, End::Failed, notes);
    }
    if let Err(e) = ports.hold(deadline.saturating_add(millis(GRACE + HOLDER_MARGIN))) {
        notes.push(format!("hold: {e}"));
        return finish(ports, session, End::Failed, notes);
    }
    let session = match ports.open() {
        Ok(()) => session.launch().map_err(|e| Broken(e.to_string()))?,
        Err(refused) => {
            let end = match &refused {
                Refused::Failed(_) => End::Failed,
                Refused::Lost(_) => End::Aborted,
            };
            notes.push(format!("open: {refused:?}"));
            stop(ports, &mut notes);
            return finish(ports, session, end, notes);
        }
    };
    save(ports, &session, &mut notes);
    let ran = ports.turn(deadline);
    stop(ports, &mut notes);
    let (session, end) = settle(ports, session, ran, &mut notes);
    finish(ports, session, end, notes)
}

/// The end path: `term`, the grace, orch's kill if anything is left, then
/// `release`. Each failure is noted and the path goes on.
fn stop<P: Ports>(ports: &mut P, notes: &mut Vec<String>) {
    if let Err(e) = ports.term() {
        notes.push(format!("term: {e}"));
    }
    let empty = ports.empty_within(GRACE).unwrap_or_else(|e| {
        notes.push(format!("cgroup: {e}"));
        false
    });
    if !empty {
        if let Err(e) = ports.kill() {
            notes.push(format!("kill: {e}"));
        }
    }
    if let Err(e) = ports.release() {
        notes.push(format!("release: {e}"));
    }
}

/// How the turn ends the session, with its usage recorded (D188).
fn settle<P: Ports>(
    ports: &mut P,
    session: Session,
    ran: Ran,
    notes: &mut Vec<String>,
) -> (Session, End) {
    let turn = match ran {
        Ran::Turn(turn) => turn,
        Ran::Wall => {
            notes.push("wall clock ran out".into());
            return (session, End::Failed);
        }
        Ran::Lost(e) => {
            notes.push(format!("mux lost: {e}"));
            return (session, End::Aborted);
        }
        Ran::Failed(e) => {
            notes.push(format!("turn: {e}"));
            return (session, End::Failed);
        }
    };
    let (tokens_in, tokens_out) = turn.tokens();
    let session = Session {
        tokens_in,
        tokens_out,
        ..session
    };
    if !turn.stop.is_end_turn() {
        notes.push(format!("stopped: {}", turn.stop.0));
        return (session, End::Failed);
    }
    match ports.submit() {
        Ok(Some(candidate)) => (session, End::Stored(candidate)),
        Ok(None) => {
            notes.push("no submit marker or an empty diff: nothing posted".into());
            (session, End::Failed)
        }
        Err(e) => {
            notes.push(format!("submit: {e}"));
            (session, End::Failed)
        }
    }
}

fn finish<P: Ports>(
    ports: &mut P,
    session: Session,
    end: End,
    mut notes: Vec<String>,
) -> Result<Outcome, Broken> {
    let at = ports.now_ms().max(session.started);
    let session = session.end(end, at).map_err(|e| Broken(e.to_string()))?;
    save(ports, &session, &mut notes);
    Ok(Outcome { session, notes })
}

fn save<P: Ports>(ports: &mut P, session: &Session, notes: &mut Vec<String>) {
    if let Err(e) = ports.save(session) {
        notes.push(format!("save: {e}"));
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}
