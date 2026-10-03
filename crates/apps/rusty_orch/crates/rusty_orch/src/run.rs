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
    /// A progress line. Never carries prompt or model text.
    fn note(&mut self, line: &str);
    /// Ask the human to answer an open question. `None` means no answer is
    /// coming, so the run stops as blocked.
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
            describe(&outcome),
            ledger.calls()
        ));
        match outcome {
            Ok(Outcome::Finished) => break Ended::Finished,
            Ok(Outcome::Blocked(ids)) => {
                if !answer_questions(&mut board, console).map_err(RunError::Io)? {
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

fn describe(outcome: &Result<Outcome, DispatchError>) -> String {
    match outcome {
        Ok(Outcome::Finished) => "finished".to_owned(),
        Ok(Outcome::Blocked(ids)) => format!("blocked on {} card(s)", ids.len()),
        Err(e) => format!("stopped: {e}"),
    }
}

/// Offer every open question to the console. Returns whether at least one
/// answer landed on the board, i.e. whether another run is worth a try.
fn answer_questions(board: &mut Board, console: &mut dyn Console) -> io::Result<bool> {
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
            break;
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
    Ok(answered)
}
