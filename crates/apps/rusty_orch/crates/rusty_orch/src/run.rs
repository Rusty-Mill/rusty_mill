//! The loop around the dispatcher: enforce the goal's wall clock between
//! runs, surface blocked questions, and, when a console is willing, take
//! the human's answers and keep going. With a [`Resume`], the plan, board,
//! and ledger are loaded from the store first and saved after every
//! dispatcher run and every answer round, so the next process continues
//! where this one stopped (ADR-0010).

use std::fmt;
use std::io;
use std::time::{Duration, Instant};

use orch_core::board::{Author, Board, EntryKind, NewEntry};
use orch_core::goal::Goal;
use orch_core::task::Plan;
use orch_core::{EntryId, GoalId, TaskId, Text};
use orch_dispatch::{
    AgentRunner, DispatchError, Dispatcher, Ledger, Outcome, Routing, RoutingError,
};
use orch_store::{GoalState, Store, StoreError};

use crate::input::{build_plan, InputError, Spec};

/// Where answers come from and where progress goes. The binary wires stdin
/// and stderr; tests script it.
pub trait Console {
    /// A progress line. Built only from fixed outcome categories, task ids,
    /// agent names, and counts: never from prompt text, model output, an
    /// adapter's error message, or child-process stderr. Those belong to
    /// the report. Questions go through [`Console::ask`] and do carry
    /// model text.
    fn note(&mut self, line: &str);
    /// Ask the human to answer an open question. `None` means the human is
    /// done answering, so the run stops as blocked, keeping any answers
    /// already recorded on the board and making no further model call.
    fn ask(&mut self, question: &str) -> io::Result<Option<String>>;
}

/// How a run ended. Every variant leaves the plan, board, and ledger
/// readable, so the report can show what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Finished,
    /// These cards wait on questions nobody answered.
    Blocked(Vec<TaskId>),
    /// The dispatcher stopped with a typed error.
    Failed(DispatchError),
    /// The goal's wall-clock budget ran out between runs.
    WallClock(Duration),
}

impl fmt::Display for Ended {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finished => f.write_str("finished"),
            Self::Blocked(ids) => write!(f, "blocked on {} card(s)", ids.len()),
            Self::Failed(e) => write!(f, "failed: {e}"),
            Self::WallClock(d) => write!(f, "wall clock of {}s exceeded", d.as_secs()),
        }
    }
}

/// The run's final state.
#[derive(Debug)]
pub struct Summary {
    pub goal: Goal,
    pub ended: Ended,
    pub plan: Plan,
    pub board: Board,
    pub ledger: Ledger,
}

/// Why a run could not even start, lost its console, or lost its store.
#[derive(Debug)]
pub enum RunError {
    Input(InputError),
    Routing(RoutingError),
    Io(io::Error),
    Store(StoreError),
    /// The store holds state for a different goal file.
    GoalChanged,
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(e) => write!(f, "{e}"),
            Self::Routing(e) => write!(f, "routing: {e}"),
            Self::Io(e) => write!(f, "console: {e}"),
            Self::Store(e) => write!(f, "{e}"),
            Self::GoalChanged => f.write_str(
                "the goal file differs from the one the saved state was built from; \
                 use a fresh --state directory",
            ),
        }
    }
}

impl From<StoreError> for RunError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// Where a run continues from and checkpoints to.
pub struct Resume<'a> {
    pub store: &'a mut Store,
    /// Of the goal file this run was given; see [`orch_store::fingerprint`].
    pub fingerprint: u64,
}

impl Resume<'_> {
    /// The saved state for `goal`, if any, refusing one built from another
    /// goal file.
    fn load(&self, goal: GoalId) -> Result<Option<GoalState>, RunError> {
        match self.store.load(goal)? {
            Some(state) if state.fingerprint != self.fingerprint => Err(RunError::GoalChanged),
            other => Ok(other),
        }
    }

    fn save(&mut self, plan: &Plan, board: &Board, ledger: &Ledger) -> Result<(), RunError> {
        let state = GoalState {
            fingerprint: self.fingerprint,
            plan: plan.clone(),
            board: board.clone(),
            ledger: ledger.clone(),
        };
        Ok(self.store.save(&state)?)
    }
}

impl std::error::Error for RunError {}

/// Run `spec` to completion, a block, a failure, or the wall clock. `now`
/// is injected so tests can move time. State remains in memory; use
/// [`execute_resumable`] to load and checkpoint it. The wall clock is per
/// invocation: a goal waiting on a human does not spend it.
pub fn execute<R: AgentRunner>(
    spec: Spec,
    runner: R,
    console: &mut dyn Console,
    now: impl Fn() -> Instant,
) -> Result<Summary, RunError> {
    execute_resumable(spec, runner, console, now, None)
}

/// Run like [`execute`], loading and checkpointing through `resume` when
/// supplied. This separate entry point preserves the original four-argument
/// API for callers that keep state in memory.
pub fn execute_resumable<R: AgentRunner>(
    spec: Spec,
    runner: R,
    console: &mut dyn Console,
    now: impl Fn() -> Instant,
    mut resume: Option<Resume<'_>>,
) -> Result<Summary, RunError> {
    let goal_id = GoalId::from_raw(1);
    let saved = match &resume {
        Some(r) => r.load(goal_id)?,
        None => None,
    };
    let (mut plan, mut board, mut ledger) = match saved {
        Some(s) => (s.plan, s.board, s.ledger),
        None => (
            build_plan(&spec, goal_id).map_err(RunError::Input)?,
            Board::new(goal_id),
            Ledger::new(),
        ),
    };
    let routing = Routing::try_from(spec.routing).map_err(RunError::Routing)?;
    let mut dispatcher = Dispatcher::new(routing, runner);
    let goal = spec.goal;
    let budget = goal.budget().wall_clock();
    let started = now();

    let ended = loop {
        if now().duration_since(started) >= budget {
            break Ended::WallClock(budget);
        }
        let outcome = dispatcher.run(&goal, &mut plan, &mut board, &mut ledger);
        checkpoint(&mut resume, &plan, &board, &ledger)?;
        console.note(&format!(
            "run: {} after {} call(s)",
            progress(&outcome),
            ledger.calls()
        ));
        match outcome {
            Ok(Outcome::Finished) => break Ended::Finished,
            Ok(Outcome::Blocked(ids)) => {
                let answers = answer_questions(&mut board, console);
                // `ask` can fail after earlier answers were appended. Always
                // attempt to persist the round before returning that error.
                // A checkpoint error wins because it means the caller cannot
                // rely on the newly accepted answers being recoverable.
                checkpoint(&mut resume, &plan, &board, &ledger)?;
                let answers = answers.map_err(RunError::Io)?;
                if answers == Answers::Stop {
                    break Ended::Blocked(ids);
                }
            }
            Err(e) => break Ended::Failed(e),
        }
    };
    Ok(Summary {
        goal,
        ended,
        plan,
        board,
        ledger,
    })
}

/// Save when there is somewhere to save to.
fn checkpoint(
    resume: &mut Option<Resume<'_>>,
    plan: &Plan,
    board: &Board,
    ledger: &Ledger,
) -> Result<(), RunError> {
    match resume {
        Some(r) => r.save(plan, board, ledger),
        None => Ok(()),
    }
}

/// The progress wording for one dispatcher run. Exhaustive over
/// [`DispatchError`] so a new variant is a compile error here, and built
/// only from the error's fixed category and its ids, agents, and limits.
/// The `source` of an agent failure is deliberately dropped: it can carry
/// model output or a child's stderr, and the report is where that goes.
fn progress(outcome: &Result<Outcome, DispatchError>) -> String {
    match outcome {
        Ok(Outcome::Finished) => "finished".to_owned(),
        Ok(Outcome::Blocked(ids)) => format!("blocked on {} card(s)", ids.len()),
        Err(DispatchError::Plan(_)) => "stopped: plan error (see report)".to_owned(),
        Err(DispatchError::Board(_)) => "stopped: board refused an entry (see report)".to_owned(),
        Err(DispatchError::Agent { task, agent, .. }) => {
            format!("stopped: {agent:?} failed {task} (see report)")
        }
        Err(DispatchError::UnsupportedRole { task, agent, role }) => {
            format!("stopped: {agent:?} cannot serve {role:?} for {task}")
        }
        Err(DispatchError::CeilingReached {
            ceiling,
            task,
            limit,
        }) => format!("stopped: {ceiling:?} ceiling of {limit} calls reached at {task}"),
        Err(DispatchError::Unroutable { task }) => format!("stopped: no agent routes {task}"),
        Err(DispatchError::Stuck) => "stopped: no card is ready or blocked".to_owned(),
    }
}

/// What the question round decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answers {
    /// Every open question was offered and at least one answer landed on
    /// the board: run the dispatcher again.
    Recorded,
    /// The human stopped (a `None` from the console) or answered nothing.
    /// Answers already recorded stay on the board; no further call is made.
    Stop,
}

/// Offer every open question to the console, in board order. A `None`
/// from the console stops the round at once, whatever came before.
fn answer_questions(board: &mut Board, console: &mut dyn Console) -> io::Result<Answers> {
    let open: Vec<(EntryId, Option<TaskId>, String)> = board
        .open_questions()
        .iter()
        .map(|e| {
            (
                e.id(),
                e.content().task,
                e.content().body.as_str().to_owned(),
            )
        })
        .collect();
    let mut answered = false;
    for (id, task, body) in open {
        let Some(reply) = console.ask(&format!("{id}: {body}"))? else {
            return Ok(Answers::Stop);
        };
        let Some(body) = Text::new(&reply) else {
            console.note(&format!("{id}: blank answer ignored"));
            continue;
        };
        let entry = NewEntry {
            task,
            author: Author::Human,
            kind: EntryKind::Answer { to: id },
            body,
            refs: Vec::new(),
            supersedes: None,
        };
        match board.append(entry) {
            Ok(_) => answered = true,
            // The question was open a moment ago; refusal here is a bug
            // worth seeing, not a reason to abort the run.
            Err(e) => console.note(&format!("{id}: answer refused: {e}")),
        }
    }
    Ok(if answered {
        Answers::Recorded
    } else {
        Answers::Stop
    })
}
