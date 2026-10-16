//! The deterministic loop that drives a [`Plan`] to completion.

use std::collections::HashSet;
use std::fmt;
use std::num::NonZeroU32;

use orch_core::board::{Author, Board, BoardError, EntryKind, NewEntry};
use orch_core::goal::Goal;
use orch_core::task::{Agent, Plan, PlanError, Role, Status, Task, TaskState};
use orch_core::{EntryId, Ref, TaskId, Text};

use crate::{AgentError, AgentRunner, ClassifiedError, Ledger, Output, Routing};

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
    /// The agent failed. Its [`ClassifiedError`] classification determines
    /// metering and whether the card remains `Running` or becomes `Failed`.
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
    /// Hosted tasks are still working; checkpoint and poll in a later run.
    Waiting(Vec<TaskId>),
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
    host: Option<Box<dyn crate::TaskHost>>,
}

impl<R: AgentRunner> Dispatcher<R> {
    /// A dispatcher over `routing` and `runner`.
    pub fn new(routing: Routing, runner: R) -> Self {
        Self {
            routing,
            runner,
            host: None,
        }
    }

    /// Enable long-lived hosting for Implement cards. Other roles keep their runner.
    pub fn with_host(mut self, host: impl crate::TaskHost + 'static) -> Self {
        self.host = Some(Box::new(host));
        self
    }

    /// Supply an optional host at a composition root.
    pub fn with_boxed_host(mut self, host: Option<Box<dyn crate::TaskHost>>) -> Self {
        self.host = host;
        self
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
        if self.host.is_none() {
            for task in plan.tasks() {
                if crate::hosted_handle(board, task.id()).is_none() {
                    continue;
                }
                let agent = match task.state() {
                    TaskState::Running { agent } | TaskState::Blocked { agent, .. } => *agent,
                    _ => continue,
                };
                return Err(DispatchError::Agent {
                    task: task.id(),
                    agent,
                    source: AgentError("hosted task requires its TaskHost configuration".into()),
                });
            }
        }
        let mut polled = HashSet::new();
        loop {
            self.poll_hosted(plan, board, &mut polled)?;
            resume_answered(plan, board)?;
            let Some(id) = next_task(plan, board) else {
                let waiting: Vec<_> = plan
                    .tasks()
                    .iter()
                    .filter(|t| t.state().status() == Status::Running)
                    .filter(|t| crate::hosted_handle(board, t.id()).is_some())
                    .map(Task::id)
                    .collect();
                if !waiting.is_empty()
                    && !plan
                        .tasks()
                        .iter()
                        .any(|t| t.state().status() == Status::Blocked)
                {
                    return Ok(Outcome::Waiting(waiting));
                }
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
        if role == Role::Implement && self.host.is_some() {
            return self.open_hosted(goal, id, agent, plan, board, ledger);
        }
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
        let task = plan.get(id).ok_or(PlanError::UnknownTask(id))?;
        ledger.count(id);
        let outputs = match self.runner.run_classified(agent, task, board) {
            Ok(outputs) => outputs,
            Err(failure) => {
                let permanent = matches!(failure, ClassifiedError::Permanent(_));
                let unavailable = matches!(failure, ClassifiedError::Unavailable(_));
                if unavailable {
                    ledger.rollback(id);
                }
                let failed = DispatchError::Agent {
                    task: id,
                    agent,
                    source: failure.into_error(),
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

    fn open_hosted(
        &mut self,
        goal: &Goal,
        id: TaskId,
        agent: Agent,
        plan: &mut Plan,
        board: &mut Board,
        ledger: &mut Ledger,
    ) -> Result<(), DispatchError> {
        let task = plan.get(id).ok_or(PlanError::UnknownTask(id))?;
        ledger.check(goal, task)?;
        if task.state().status() == Status::Pending {
            plan.start(id, agent)?;
        }
        let task = plan.get(id).ok_or(PlanError::UnknownTask(id))?;
        let Some(host) = self.host.as_mut() else {
            return Err(DispatchError::Stuck);
        };
        let handle = host
            .open(task, board)
            .map_err(|e| host_error(plan, id, agent, e))?;
        append_atomically(
            board,
            id,
            agent,
            vec![Output {
                kind: EntryKind::Artifact,
                body: Text::new("hosted task").ok_or(DispatchError::Stuck)?,
                refs: vec![Ref::Url(handle.0)],
                supersedes: None,
            }],
        )?;
        // A failed/uncertain open is adopted on retry; only the durable handle is charged.
        ledger.count(id);
        Ok(())
    }

    fn poll_hosted(
        &mut self,
        plan: &mut Plan,
        board: &mut Board,
        polled: &mut HashSet<TaskId>,
    ) -> Result<(), DispatchError> {
        let Some(host) = self.host.as_mut() else {
            return Ok(());
        };
        let tasks: Vec<_> = plan
            .tasks()
            .iter()
            .filter_map(|t| {
                let agent = match t.state() {
                    TaskState::Running { agent } | TaskState::Blocked { agent, .. } => *agent,
                    _ => return None,
                };
                Some((t.id(), agent, crate::hosted_handle(board, t.id())?))
            })
            .collect();
        for (id, agent, handle) in tasks {
            if !polled.insert(id) {
                continue;
            }
            let state = plan
                .get(id)
                .ok_or(PlanError::UnknownTask(id))?
                .state()
                .clone();
            let mut supersedes = None;
            let progress = if let TaskState::Blocked { question, .. } = state {
                let gate = board.get(question).ok_or(DispatchError::Stuck)?;
                let answer = board.live().find(|e| {
                    e.content().author == Author::Human
                        && e.content().kind == EntryKind::Answer { to: question }
                });
                let Some(answer) = answer else {
                    // Supervise without replacing the persisted question's fence.
                    let progress = host
                        .poll(&handle)
                        .map_err(|e| host_error(plan, id, agent, e))?;
                    if matches!(
                        progress,
                        crate::Progress::Closed { .. } | crate::Progress::Cancelled { .. }
                    ) {
                        board.append(NewEntry {
                            task: Some(id),
                            author: Author::Agent(agent),
                            kind: EntryKind::Answer { to: question },
                            body: Text::new("host recorded terminal outcome")
                                .ok_or(DispatchError::Stuck)?,
                            refs: vec![Ref::Url(handle.0.clone())],
                            supersedes: None,
                        })?;
                        plan.resume(id)?;
                        apply_progress(plan, board, id, agent, &handle, progress, None)?;
                    }
                    continue;
                };
                let relayed = host
                    .relay(&handle, gate, &answer.content().body)
                    .map_err(|e| host_error(plan, id, agent, e))?;
                supersedes = Some(question);
                plan.resume(id)?;
                match relayed {
                    crate::Relayed::Applied => host.poll(&handle),
                    crate::Relayed::Stale(p) | crate::Relayed::Rejected(p) => Ok(p),
                }
            } else {
                host.poll(&handle)
            }
            .map_err(|e| host_error(plan, id, agent, e))?;
            apply_progress(plan, board, id, agent, &handle, progress, supersedes)?;
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
            TaskState::Blocked { question, .. }
                if !open.contains(question) && crate::hosted_handle(board, t.id()).is_none() =>
            {
                Some(t.id())
            }
            _ => None,
        })
        .collect();
    for id in answered {
        plan.resume(id)?;
    }
    Ok(())
}

/// Lowest id among resumed (`Running`) and ready cards.
fn next_task(plan: &Plan, board: &Board) -> Option<TaskId> {
    let running = plan
        .tasks()
        .iter()
        .filter(|t| t.state().status() == Status::Running)
        .filter(|t| crate::hosted_handle(board, t.id()).is_none());
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

fn host_error(plan: &mut Plan, id: TaskId, agent: Agent, e: ClassifiedError) -> DispatchError {
    let permanent = matches!(e, ClassifiedError::Permanent(_));
    let error = DispatchError::Agent {
        task: id,
        agent,
        source: e.into_error(),
    };
    if permanent {
        if matches!(
            plan.get(id).map(Task::state),
            Some(TaskState::Blocked { .. })
        ) {
            if let Err(e) = plan.resume(id) {
                return e.into();
            }
        }
        if let Err(e) = fail_terminal(plan, id, &error) {
            return e;
        }
    }
    error
}

fn apply_progress(
    plan: &mut Plan,
    board: &mut Board,
    id: TaskId,
    agent: Agent,
    handle: &crate::HostHandle,
    progress: crate::Progress,
    supersedes: Option<EntryId>,
) -> Result<(), DispatchError> {
    match progress {
        crate::Progress::Working => Ok(()),
        crate::Progress::Gate(gate) => {
            let rev = Text::new(&format!("host:rev:{}", gate.rev)).ok_or(DispatchError::Stuck)?;
            let mut refs = vec![Ref::Url(handle.0.clone()), Ref::Url(rev)];
            refs.extend(gate.ids.into_iter().map(Ref::Url));
            let ids = append_atomically(
                board,
                id,
                agent,
                vec![Output {
                    kind: EntryKind::Question,
                    body: gate.prompt,
                    refs,
                    supersedes,
                }],
            )?;
            let question = *ids.first().ok_or(DispatchError::Stuck)?;
            plan.block(id, question)?;
            Ok(())
        }
        crate::Progress::Closed { outputs } => {
            let ids = append_atomically(board, id, agent, outputs)?;
            Ok(plan.complete(id, ids)?)
        }
        crate::Progress::Cancelled { reason } => Err(host_error(
            plan,
            id,
            agent,
            ClassifiedError::Permanent(AgentError(reason.as_str().into())),
        )),
    }
}
