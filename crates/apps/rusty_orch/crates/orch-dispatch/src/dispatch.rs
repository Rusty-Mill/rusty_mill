//! The deterministic loop that drives a [`Plan`] to completion.

use std::fmt;
use std::num::NonZeroU32;

use orch_core::board::{Author, Board, BoardError, EntryKind, NewEntry};
use orch_core::goal::Goal;
use orch_core::task::{Agent, Plan, PlanError, Role, Status, Task, TaskState};
use orch_core::{EntryId, TaskId, Text};

use crate::{AgentError, AgentRunner, Ledger, Output, Routing};

/// Which call ceiling was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ceiling {
    /// `Budget::max_calls` across the whole goal.
    Goal,
    /// `TaskSpec::max_calls` for one card.
    Task,
}

/// Why the loop stopped before the plan finished or blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    Plan(PlanError),
    /// An output failed board validation. Nothing from that call was
    /// appended; the card stays `Running` and the call is still counted.
    Board(BoardError),
    /// The agent failed. On a [`AgentError::Transient`] the card stays
    /// `Running`, so a later `run` retries it under the same ceilings. On a
    /// [`AgentError::Permanent`] the card is marked `Failed` first: retrying
    /// cannot help, so its remaining budget is not spent.
    Agent {
        task: TaskId,
        agent: Agent,
        source: AgentError,
    },
    /// The selected adapter cannot serve this role. The card is failed
    /// without counting or invoking a model call.
    UnsupportedRole {
        task: TaskId,
        agent: Agent,
        role: Role,
    },
    /// Running `task` would exceed `ceiling`. No call was made. For the
    /// task ceiling on a card already `Running`, the card is marked
    /// `Failed` first: its retries are exhausted.
    CeilingReached {
        ceiling: Ceiling,
        task: TaskId,
        limit: NonZeroU32,
    },
    /// The routing table has no agent for this card. Cannot happen with a
    /// validated [`Routing`]; kept typed rather than panicking.
    Unroutable {
        task: TaskId,
    },
    /// Nothing is ready, nothing is blocked, and the plan is not finished:
    /// a failed card has dependents.
    Stuck,
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(e) => write!(f, "plan: {e}"),
            Self::Board(e) => write!(f, "board: {e}"),
            Self::Agent {
                task,
                agent,
                source,
            } => write!(f, "{agent:?} failed {task}: {source}"),
            Self::UnsupportedRole { task, agent, role } => {
                write!(f, "{agent:?} cannot serve {role:?} for {task}")
            }
            Self::CeilingReached {
                ceiling,
                task,
                limit,
            } => write!(f, "{ceiling:?} ceiling of {limit} calls reached at {task}"),
            Self::Unroutable { task } => write!(f, "no agent routes {task}"),
            Self::Stuck => f.write_str("no task is ready or blocked, and the plan is unfinished"),
        }
    }
}

impl std::error::Error for DispatchError {}

impl From<PlanError> for DispatchError {
    fn from(e: PlanError) -> Self {
        Self::Plan(e)
    }
}

impl From<BoardError> for DispatchError {
    fn from(e: BoardError) -> Self {
        Self::Board(e)
    }
}

/// How a `run` ended without error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every card is done.
    Finished,
    /// These cards wait on open questions. Answer them on the board and
    /// call `run` again.
    Blocked(Vec<TaskId>),
}

/// Drives one goal's plan: route, check, start, count, run, record.
///
/// Holds only routing and the runner. All state lives in the plan, board,
/// and [`Ledger`] the caller passes to [`Dispatcher::run`], so a dispatcher
/// can be dropped and rebuilt mid-goal.
///
/// Tasks run one at a time in `TaskId` order. That is a deliberate P1
/// choice: it makes every run reproducible from the plan alone. Parallel
/// fan-out over independent ready cards is the trigger for an async
/// dispatcher later.
#[derive(Debug)]
pub struct Dispatcher<R> {
    routing: Routing,
    runner: R,
}

impl<R: AgentRunner> Dispatcher<R> {
    /// A dispatcher over `routing` and `runner`.
    pub fn new(routing: Routing, runner: R) -> Self {
        Self { routing, runner }
    }

    /// The runner, e.g. to inspect a fake in tests.
    pub fn runner(&self) -> &R {
        &self.runner
    }

    /// Run until the plan finishes, every remaining card is blocked on a
    /// question, or an error stops the loop. Resumable: call again after
    /// answering questions or after an agent error.
    ///
    /// A card that blocks and later resumes is called again; `Done.outputs`
    /// then holds only the final call's entries. Earlier entries (the
    /// question itself) stay on the board under the same task id.
    pub fn run(
        &mut self,
        goal: &Goal,
        plan: &mut Plan,
        board: &mut Board,
        ledger: &mut Ledger,
    ) -> Result<Outcome, DispatchError> {
        loop {
            resume_answered(plan, board)?;
            let Some(id) = next_task(plan) else {
                return outcome(plan);
            };
            self.step(goal, id, plan, board, ledger)?;
        }
    }

    /// Check → start → count → run → record.
    fn step(
        &mut self,
        goal: &Goal,
        id: TaskId,
        plan: &mut Plan,
        board: &mut Board,
        ledger: &mut Ledger,
    ) -> Result<(), DispatchError> {
        let task = plan.get(id).ok_or(PlanError::UnknownTask(id))?;
        let agent = match task.state() {
            TaskState::Pending => self.route(task, plan)?,
            TaskState::Running { agent } => *agent,
            other => {
                return Err(PlanError::InvalidTransition {
                    task: id,
                    from: other.status(),
                    action: "run",
                }
                .into())
            }
        };
        let status = task.state().status();
        let role = task.spec().role;
        if !self.runner.supports(agent, role) {
            let refused = DispatchError::UnsupportedRole {
                task: id,
                agent,
                role,
            };
            if status == Status::Pending {
                plan.start(id, agent)?;
            }
            fail_terminal(plan, id, &refused)?;
            return Err(refused);
        }
        if let Err(refused) = ledger.check(goal, task) {
            return Err(exhaust(plan, id, status, refused)?);
        }
        if status == Status::Pending {
            plan.start(id, agent)?;
        }
        ledger.count(id);
        let task = plan.get(id).ok_or(PlanError::UnknownTask(id))?;
        let outputs = match self.runner.run(agent, task, board) {
            Ok(outputs) => outputs,
            Err(source) => {
                let permanent = source.is_permanent();
                let failed = DispatchError::Agent {
                    task: id,
                    agent,
                    source,
                };
                if permanent {
                    fail_terminal(plan, id, &failed)?;
                }
                return Err(failed);
            }
        };
        let ids = append_atomically(board, id, agent, outputs)?;
        match first_question(board, &ids) {
            Some(question) => plan.block(id, question)?,
            None => plan.complete(id, ids)?,
        }
        Ok(())
    }

    fn route(&self, task: &Task, plan: &Plan) -> Result<Agent, DispatchError> {
        let unroutable = DispatchError::Unroutable { task: task.id() };
        let Role::Review { target } = task.spec().role else {
            return self.routing.work(task.spec().role).ok_or(unroutable);
        };
        let author = match plan.get(target).map(Task::state) {
            Some(TaskState::Done { agent, .. }) => *agent,
            _ => return Err(PlanError::NotReady(task.id()).into()),
        };
        self.routing.reviewer(author).ok_or(unroutable)
    }
}

/// Fail a card for a deterministic refusal that retrying cannot change.
fn fail_terminal(
    plan: &mut Plan,
    task: TaskId,
    refused: &DispatchError,
) -> Result<(), DispatchError> {
    // Every dispatcher error has a non-blank Display representation.
    if let Some(reason) = Text::new(&refused.to_string()) {
        plan.fail(task, reason)?;
    }
    Ok(())
}

/// A `Running` card refused by its own ceiling has exhausted its retries:
/// mark it `Failed` with the refusal as the reason, then hand back the
/// refusal. Any other refusal passes through untouched.
fn exhaust(
    plan: &mut Plan,
    task: TaskId,
    status: Status,
    refused: DispatchError,
) -> Result<DispatchError, DispatchError> {
    let exhausted = matches!(
        refused,
        DispatchError::CeilingReached {
            ceiling: Ceiling::Task,
            ..
        }
    ) && status == Status::Running;
    if !exhausted {
        return Ok(refused);
    }
    // `Display` of a `CeilingReached` is never blank, so `Text::new` cannot
    // return `None` here; the `if let` keeps that fact out of the panic path.
    if let Some(reason) = Text::new(&refused.to_string()) {
        plan.fail(task, reason)?;
    }
    Ok(refused)
}

/// Return every blocked card whose question now has a live answer.
fn resume_answered(plan: &mut Plan, board: &Board) -> Result<(), DispatchError> {
    let open: Vec<EntryId> = board.open_questions().iter().map(|e| e.id()).collect();
    let answered: Vec<TaskId> = plan
        .tasks()
        .iter()
        .filter_map(|t| match t.state() {
            TaskState::Blocked { question, .. } if !open.contains(question) => Some(t.id()),
            _ => None,
        })
        .collect();
    for id in answered {
        plan.resume(id)?;
    }
    Ok(())
}

/// Lowest id among resumed (`Running`) and ready cards.
fn next_task(plan: &Plan) -> Option<TaskId> {
    let running = plan
        .tasks()
        .iter()
        .filter(|t| t.state().status() == Status::Running);
    running.chain(plan.ready()).map(Task::id).min()
}

fn outcome(plan: &Plan) -> Result<Outcome, DispatchError> {
    if plan.is_finished() {
        return Ok(Outcome::Finished);
    }
    let blocked: Vec<TaskId> = plan
        .tasks()
        .iter()
        .filter(|t| t.state().status() == Status::Blocked)
        .map(Task::id)
        .collect();
    if blocked.is_empty() {
        return Err(DispatchError::Stuck);
    }
    Ok(Outcome::Blocked(blocked))
}

/// Stamp each output with the task and author and append all of them, or
/// none: entries are staged on a copy of the board, which replaces the
/// original only when every append succeeds.
fn append_atomically(
    board: &mut Board,
    task: TaskId,
    agent: Agent,
    outputs: Vec<Output>,
) -> Result<Vec<EntryId>, DispatchError> {
    let mut staged = board.clone();
    let ids = outputs
        .into_iter()
        .map(|o| {
            let entry = NewEntry {
                task: Some(task),
                author: Author::Agent(agent),
                kind: o.kind,
                body: o.body,
                refs: o.refs,
                supersedes: o.supersedes,
            };
            staged.append(entry).map_err(DispatchError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    *board = staged;
    Ok(ids)
}

fn first_question(board: &Board, ids: &[EntryId]) -> Option<EntryId> {
    ids.iter()
        .copied()
        .find(|id| board.get(*id).map(|e| e.content().kind) == Some(EntryKind::Question))
}
