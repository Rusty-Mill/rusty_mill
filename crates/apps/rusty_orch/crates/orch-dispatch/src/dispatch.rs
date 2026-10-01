//! The deterministic loop that drives a [`Plan`] to completion.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;

use orch_core::board::{Author, Board, BoardError, EntryKind, NewEntry};
use orch_core::goal::Goal;
use orch_core::task::{Agent, Plan, PlanError, Role, Status, Task, TaskState};
use orch_core::{EntryId, TaskId};

use crate::{AgentError, AgentRunner, Output, Routing};

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
    Board(BoardError),
    /// The agent failed. The card stays `Running`, so a later `run` retries
    /// it under the same ceilings; retry policy is out of scope here.
    Agent {
        task: TaskId,
        agent: Agent,
        source: AgentError,
    },
    /// Running `task` would exceed `ceiling`. No call was made.
    CeilingReached {
        ceiling: Ceiling,
        task: TaskId,
        limit: NonZeroU32,
    },
    /// A validated routing table always has a reviewer; this is the typed
    /// fallback for the branch that cannot happen.
    NoReviewer {
        task: TaskId,
        author: Agent,
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
            Self::CeilingReached {
                ceiling,
                task,
                limit,
            } => write!(f, "{ceiling:?} ceiling of {limit} calls reached at {task}"),
            Self::NoReviewer { task, author } => {
                write!(f, "no reviewer other than {author:?} for {task}")
            }
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

/// Drives one goal's plan: route, meter, run, record.
///
/// Tasks run one at a time in `TaskId` order. That is a deliberate P1
/// choice: it makes every run reproducible from the plan alone. Parallel
/// fan-out over independent ready cards is the trigger for an async
/// dispatcher later.
#[derive(Debug)]
pub struct Dispatcher<R> {
    routing: Routing,
    runner: R,
    goal_calls: u32,
    task_calls: BTreeMap<TaskId, u32>,
}

impl<R: AgentRunner> Dispatcher<R> {
    /// A dispatcher with no calls spent yet.
    pub fn new(routing: Routing, runner: R) -> Self {
        Self {
            routing,
            runner,
            goal_calls: 0,
            task_calls: BTreeMap::new(),
        }
    }

    /// Calls made against the goal budget so far.
    pub fn calls(&self) -> u32 {
        self.goal_calls
    }

    /// The runner, e.g. to inspect a fake in tests.
    pub fn runner(&self) -> &R {
        &self.runner
    }

    /// Run until the plan finishes, every remaining card is blocked on a
    /// question, or an error stops the loop. Resumable: call again after
    /// answering questions or after an agent error.
    pub fn run(
        &mut self,
        goal: &Goal,
        plan: &mut Plan,
        board: &mut Board,
    ) -> Result<Outcome, DispatchError> {
        loop {
            resume_answered(plan, board)?;
            let Some(id) = next_task(plan) else {
                return outcome(plan);
            };
            self.step(goal, id, plan, board)?;
        }
    }

    fn step(
        &mut self,
        goal: &Goal,
        id: TaskId,
        plan: &mut Plan,
        board: &mut Board,
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
        self.meter(goal, task)?;
        if task.state() == &TaskState::Pending {
            plan.start(id, agent)?;
        }
        let task = plan.get(id).ok_or(PlanError::UnknownTask(id))?;
        let outputs =
            self.runner
                .run(agent, task, board)
                .map_err(|source| DispatchError::Agent {
                    task: id,
                    agent,
                    source,
                })?;
        let ids = append_all(board, id, agent, outputs)?;
        match first_question(board, &ids) {
            Some(question) => plan.block(id, question)?,
            None => plan.complete(id, ids)?,
        }
        Ok(())
    }

    fn route(&self, task: &Task, plan: &Plan) -> Result<Agent, DispatchError> {
        let Role::Review { target } = task.spec().role else {
            return self
                .routing
                .work(task.spec().role)
                .ok_or(DispatchError::NoReviewer {
                    task: task.id(),
                    author: Agent::Local,
                });
        };
        let author = match plan.get(target).map(Task::state) {
            Some(TaskState::Done { agent, .. }) => *agent,
            _ => return Err(PlanError::NotReady(task.id()).into()),
        };
        self.routing
            .reviewer(author)
            .ok_or(DispatchError::NoReviewer {
                task: task.id(),
                author,
            })
    }

    /// Check both ceilings, then count the call. Nothing is spent on failure.
    fn meter(&mut self, goal: &Goal, task: &Task) -> Result<(), DispatchError> {
        let id = task.id();
        let goal_limit = goal.budget().max_calls();
        if self.goal_calls >= goal_limit.get() {
            return Err(ceiling(Ceiling::Goal, id, goal_limit));
        }
        let task_limit = task.spec().max_calls;
        let spent = self.task_calls.entry(id).or_insert(0);
        if *spent >= task_limit.get() {
            return Err(ceiling(Ceiling::Task, id, task_limit));
        }
        *spent += 1;
        self.goal_calls += 1;
        Ok(())
    }
}

fn ceiling(ceiling: Ceiling, task: TaskId, limit: NonZeroU32) -> DispatchError {
    DispatchError::CeilingReached {
        ceiling,
        task,
        limit,
    }
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

/// Stamp each output with the task and author, then append in order.
fn append_all(
    board: &mut Board,
    task: TaskId,
    agent: Agent,
    outputs: Vec<Output>,
) -> Result<Vec<EntryId>, DispatchError> {
    outputs
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
            board.append(entry).map_err(DispatchError::from)
        })
        .collect()
}

fn first_question(board: &Board, ids: &[EntryId]) -> Option<EntryId> {
    ids.iter()
        .copied()
        .find(|id| board.get(*id).map(|e| e.content().kind) == Some(EntryKind::Question))
}
