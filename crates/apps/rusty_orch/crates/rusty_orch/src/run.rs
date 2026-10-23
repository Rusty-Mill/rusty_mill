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
use orch_core::factory::{ExecutionLink, ExecutionState, Factory, FactoryError, Grant, Identity};
use orch_core::goal::Goal;
use orch_core::task::Plan;
use orch_core::{EntryId, GoalId, TaskId, Text};
use orch_dispatch::{
    AgentRunner, DispatchError, Dispatcher, Fence, Ledger, Outcome, Progress, Routing,
    RoutingError, TaskHost,
};
use orch_store::{GoalState, Store, StoreError};

use crate::input::{build_plan, InputError, Spec};
use crate::millboard::{self, CardLinks};

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
    /// Hosted tasks are still running in BBP.
    Waiting(Vec<TaskId>),
    /// These cards wait on questions nobody answered.
    Blocked(Vec<TaskId>),
    /// The dispatcher stopped with a typed error.
    Failed(DispatchError),
    /// The goal's wall-clock budget ran out between runs.
    WallClock(Duration),
    /// The goal's hosted tasks were cancelled and its assignment retired.
    Cancelled,
}

impl fmt::Display for Ended {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finished => f.write_str("finished"),
            Self::Waiting(ids) => write!(f, "waiting on {} hosted card(s)", ids.len()),
            Self::Blocked(ids) => write!(f, "blocked on {} card(s)", ids.len()),
            Self::Failed(e) => write!(f, "failed: {e}"),
            Self::WallClock(d) => write!(f, "wall clock of {}s exceeded", d.as_secs()),
            Self::Cancelled => f.write_str("cancelled"),
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
    /// The goal's orchestrator assignment could not be taken over, so no
    /// hosted command may run.
    Lease(String),
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
            Self::Lease(why) => write!(f, "cannot hold the goal's hosted tasks: {why}"),
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
    resume: Option<Resume<'_>>,
) -> Result<Summary, RunError> {
    execute_hosted(spec, runner, console, now, resume, None)
}

/// The one orchestrator assignment a goal's state holds. Each process that
/// hosts the goal takes it over at the next generation.
pub const GOAL_ORCHESTRATOR: &str = "goal-orchestrator";

/// A token unique to this process, distinguishing it from any other holder
/// that reaches the same generation from a diverged copy of the state.
///
/// # Errors
///
/// When the platform's random source fails.
pub fn holder() -> Result<Text, String> {
    let bytes = rusty_rand::bytes(16).map_err(|e| format!("random holder: {e}"))?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Text::new(&hex).ok_or_else(|| "empty holder".to_owned())
}

/// The attempt a card's execution link names. A run resumes the goal's
/// hosted tasks rather than re-attempting them, so there is one.
pub const ATTEMPT: &str = "1";

/// A long-lived task host, and the Millboard card the goal is hosted for.
pub struct Hosting {
    pub host: Box<dyn TaskHost>,
    pub card: Option<CardHosting>,
    /// Cancel the goal's hosted tasks for this reason instead of running.
    pub cancel: Option<Text>,
}

/// The Millboard card a hosted goal works for.
pub struct CardHosting {
    /// Millboard's card id.
    pub card: String,
    /// The goal's BBP namespace: its logical execution, and the prefix every
    /// one of its BBP task ids derives from.
    pub namespace: String,
    pub board: Box<dyn CardLinks>,
}

/// Take over the goal's orchestrator assignment at its next generation and
/// make that durable before any hosted command can use it, so every earlier
/// process holding the goal is fenced out by the host. With a card, the
/// generation is then recorded with the card, so the card always names the
/// generation that may act for it.
fn take_over(
    resume: &mut Option<Resume<'_>>,
    goal: &Goal,
    goal_id: GoalId,
    card: Option<&mut CardHosting>,
) -> Result<Fence, RunError> {
    let mut factory = match resume {
        Some(r) => r.store.load_factory(goal_id)?,
        None => Factory::default(),
    };
    let lease = |e: FactoryError| RunError::Lease(format!("{e:?}"));
    if let (None, Some(linked)) = (&card, factory.links().next()) {
        return Err(RunError::Lease(format!(
            "this state is linked to card {}; resume it with --card and --board",
            linked.card
        )));
    }
    let generation = factory
        .take_over(
            GOAL_ORCHESTRATOR,
            Identity {
                id: "rusty_orch".into(),
                target: "rusty_orch run".into(),
            },
            Grant {
                projects: Vec::new(),
                delegation_budget: 0,
                call_budget: goal.budget().max_calls().get(),
            },
        )
        .map_err(lease)?;
    let link = card
        .as_ref()
        .map(|c| link_card(&mut factory, c, goal_id, generation))
        .transpose()?;
    if let Some(r) = resume {
        r.store.save_factory(goal_id, &factory)?;
    }
    if let (Some(card), Some(link)) = (card, link) {
        millboard::publish(card.board.as_mut(), &link).map_err(RunError::Lease)?;
    }
    Ok(Fence {
        assignment: Text::new(GOAL_ORCHESTRATOR)
            .ok_or_else(|| RunError::Lease("blank assignment".into()))?,
        generation,
        holder: holder().map_err(RunError::Lease)?,
    })
}

/// Bind `card` to the goal's execution at `generation` in the factory. One
/// state directory hosts one goal, so it serves one card.
fn link_card(
    factory: &mut Factory,
    card: &CardHosting,
    goal_id: GoalId,
    generation: u64,
) -> Result<ExecutionLink, RunError> {
    if let Some(other) = factory.links().find(|l| l.card != card.card) {
        return Err(RunError::Lease(format!(
            "this state is linked to card {}",
            other.card
        )));
    }
    factory
        .initiate_or_adopt(ExecutionLink {
            card: card.card.clone(),
            execution: card.namespace.clone(),
            goal: goal_id.get().to_string(),
            bbp_task: card.namespace.clone(),
            attempt: ATTEMPT.into(),
            assignment: GOAL_ORCHESTRATOR.into(),
            generation,
            state: ExecutionState::Running,
        })
        .map_err(|e| RunError::Lease(format!("{e:?}")))
}

/// Resumable execution with an optional long-lived task host. With a host,
/// this process first takes over the goal's orchestrator assignment, records
/// it with the hosting card if there is one, and issues every hosted command
/// under that generation.
pub fn execute_hosted<R: AgentRunner>(
    spec: Spec,
    runner: R,
    console: &mut dyn Console,
    now: impl Fn() -> Instant,
    mut resume: Option<Resume<'_>>,
    hosting: Option<Hosting>,
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
    let goal = spec.goal;
    let host = match hosting {
        Some(mut h) if h.cancel.is_some() => {
            let ended = cancel(&mut resume, &goal, goal_id, &mut h, &board, &plan)?;
            return Ok(Summary {
                goal,
                ended,
                plan,
                board,
                ledger,
            });
        }
        Some(mut h) => {
            let fence = take_over(&mut resume, &goal, goal_id, h.card.as_mut())?;
            Some((h.host, fence))
        }
        None => None,
    };
    let mut dispatcher = Dispatcher::new(routing, runner).with_boxed_host(host);
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
            Ok(Outcome::Waiting(ids)) => break Ended::Waiting(ids),
            Ok(Outcome::Blocked(ids)) => {
                let answers = answer_questions(&mut board, &plan, console);
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

/// Cancel every hosted task of the goal, then retire its assignment.
///
/// The run first takes the assignment over, so every earlier holder is
/// fenced out before anything is cancelled. Each Implement card's host
/// execution is cancelled whether or not its handle reached the board, and
/// each host returns only after the execution's processes are gone. Only then is the card's execution reconciled
/// as cancelled and the assignment retired, durably, so no later run can
/// resume the goal. Every step is idempotent: a cancel interrupted at any
/// point is completed by running it again, and one that already finished
/// does nothing.
fn cancel(
    resume: &mut Option<Resume<'_>>,
    goal: &Goal,
    goal_id: GoalId,
    hosting: &mut Hosting,
    board: &Board,
    plan: &Plan,
) -> Result<Ended, RunError> {
    let lease = |e: FactoryError| RunError::Lease(format!("{e:?}"));
    let reason = hosting
        .cancel
        .clone()
        .ok_or_else(|| RunError::Lease("no cancel reason".into()))?;
    let retired = match resume {
        Some(r) => r
            .store
            .load_factory(goal_id)?
            .assignment(GOAL_ORCHESTRATOR)
            .is_some_and(|a| !a.active),
        None => return Err(RunError::Lease("cancel needs durable --state".into())),
    };
    if retired {
        return Ok(Ended::Cancelled);
    }
    let fence = take_over(resume, goal, goal_id, hosting.card.as_mut())?;
    // Every card a host would serve, handle or not: an open can start work
    // before its handle is checkpointed, and the host finds that work itself.
    let hosted = plan
        .tasks()
        .iter()
        .filter(|t| t.spec().role == orch_core::task::Role::Implement);
    for task in hosted {
        match hosting.host.cancel(task, board, &reason, &fence) {
            Ok(Progress::Cancelled { .. } | Progress::Closed { .. }) => {}
            Ok(_) => {
                return Err(RunError::Lease(format!(
                    "hosted task {} did not end",
                    task.id()
                )))
            }
            Err(e) => return Err(RunError::Lease(format!("cancel {}: {e:?}", task.id()))),
        }
    }
    let Some(r) = resume else {
        return Err(RunError::Lease("cancel needs durable --state".into()));
    };
    let mut factory = r.store.load_factory(goal_id)?;
    if let Some(card) = &hosting.card {
        factory
            .reconcile(
                &card.card,
                GOAL_ORCHESTRATOR,
                fence.generation,
                ExecutionState::Cancelled,
            )
            .map_err(lease)?;
    }
    factory
        .retire(GOAL_ORCHESTRATOR, fence.generation)
        .map_err(lease)?;
    r.store.save_factory(goal_id, &factory)?;
    Ok(Ended::Cancelled)
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
        Ok(Outcome::Waiting(ids)) => format!("waiting on {} hosted card(s)", ids.len()),
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
fn answer_questions(
    board: &mut Board,
    plan: &Plan,
    console: &mut dyn Console,
) -> io::Result<Answers> {
    let mut questions = board.open_questions();
    for task in plan.tasks() {
        let orch_core::task::TaskState::Blocked { question, .. } = task.state() else {
            continue;
        };
        if orch_dispatch::hosted_handle(board, task.id()).is_none() {
            continue;
        }
        let human_answered = board.live().any(|e| {
            e.content().author == Author::Human
                && e.content().kind == EntryKind::Answer { to: *question }
        });
        if human_answered || questions.iter().any(|q| q.id() == *question) {
            continue;
        }
        if let Some(question) = board.get(*question) {
            questions.push(question);
        }
    }
    questions.sort_by_key(|q| q.id());
    let open: Vec<(EntryId, Option<TaskId>, String)> = questions
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
