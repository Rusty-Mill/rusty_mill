//! The loop around the dispatcher: enforce the goal's wall clock between
//! runs, surface blocked questions, and, when a console is willing, take
//! the human's answers and keep going.

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

/// Why a run could not even start, or lost its console.
#[derive(Debug)]
pub enum RunError {
    Input(InputError),
    Routing(RoutingError),
    Io(io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(e) => write!(f, "{e}"),
            Self::Routing(e) => write!(f, "routing: {e}"),
            Self::Io(e) => write!(f, "console: {e}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Run `spec` to completion, a block, a failure, or the wall clock. `now`
/// is injected so tests can move time.
pub fn execute<R: AgentRunner>(
    spec: Spec,
    runner: R,
    console: &mut dyn Console,
    now: impl Fn() -> Instant,
) -> Result<Summary, RunError> {
    let goal_id = GoalId::from_raw(1);
    let mut plan = build_plan(&spec, goal_id).map_err(RunError::Input)?;
    let mut board = Board::new(goal_id);
    let mut ledger = Ledger::new();
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
        console.note(&format!(
            "run: {} after {} call(s)",
            progress(&outcome),
            ledger.calls()
        ));
        match outcome {
            Ok(Outcome::Finished) => break Ended::Finished,
            Ok(Outcome::Blocked(ids)) => match answer_questions(&mut board, console) {
                Ok(Answers::Recorded) => {}
                Ok(Answers::Stop) => break Ended::Blocked(ids),
                Err(e) => return Err(RunError::Io(e)),
            },
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
