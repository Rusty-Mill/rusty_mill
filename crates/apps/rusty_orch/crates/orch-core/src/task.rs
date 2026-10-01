//! Task cards and the plan that owns their lifecycle.
//!
//! All state changes go through [`Plan`], which enforces the invariants:
//! dependencies exist (so the graph is acyclic by construction: a card can
//! only depend on cards added before it), work starts only when its
//! prerequisites are done, completions return blackboard refs rather than
//! content, and no agent reviews its own output.

use std::fmt;
use std::num::NonZeroU32;

use crate::{EntryId, GoalId, Ref, TaskId, Text};

/// A model backend the dispatcher can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Agent {
    Claude,
    Codex,
    Gemini,
    Local,
}

/// The kind of work a card asks for. Routing maps roles to agents, so a
/// card never names an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Research,
    Design,
    Implement,
    Triage,
    /// Review another task's output. The target is an implicit prerequisite.
    Review {
        target: TaskId,
    },
}

/// Work as the orchestrator proposes it, before it joins a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSpec {
    pub role: Role,
    pub instruction: Text,
    /// Must be non-empty; checked by [`Plan::add`].
    pub acceptance: Vec<Text>,
    pub refs: Vec<Ref>,
    pub depends_on: Vec<TaskId>,
    pub max_calls: NonZeroU32,
}

/// Lifecycle position without payload, for errors and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Running,
    Blocked,
    Done,
    Failed,
}

/// Lifecycle state. Every non-pending state records which agent holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    Running {
        agent: Agent,
    },
    /// Waiting on a `Question` entry.
    Blocked {
        agent: Agent,
        question: EntryId,
    },
    /// `outputs` is non-empty: the entries the agent wrote.
    Done {
        agent: Agent,
        outputs: Vec<EntryId>,
    },
    Failed {
        agent: Agent,
        reason: Text,
    },
}

impl TaskState {
    /// The payload-free status.
    pub fn status(&self) -> Status {
        match self {
            Self::Pending => Status::Pending,
            Self::Running { .. } => Status::Running,
            Self::Blocked { .. } => Status::Blocked,
            Self::Done { .. } => Status::Done,
            Self::Failed { .. } => Status::Failed,
        }
    }

    fn agent(&self) -> Option<Agent> {
        match self {
            Self::Pending => None,
            Self::Running { agent }
            | Self::Blocked { agent, .. }
            | Self::Done { agent, .. }
            | Self::Failed { agent, .. } => Some(*agent),
        }
    }
}

/// A task card inside a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    id: TaskId,
    spec: TaskSpec,
    state: TaskState,
}

impl Task {
    /// Id, unique within its plan.
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// What was asked.
    pub fn spec(&self) -> &TaskSpec {
        &self.spec
    }

    /// Where it is in its lifecycle.
    pub fn state(&self) -> &TaskState {
        &self.state
    }

    /// Explicit dependencies plus the review target, if any.
    pub fn prerequisites(&self) -> impl Iterator<Item = TaskId> + '_ {
        let target = match self.spec.role {
            Role::Review { target } => Some(target),
            _ => None,
        };
        self.spec.depends_on.iter().copied().chain(target)
    }
}

/// Why a plan operation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    UnknownTask(TaskId),
    NoAcceptance,
    UnknownDependency(TaskId),
    NotReady(TaskId),
    SelfReview {
        task: TaskId,
        agent: Agent,
    },
    InvalidTransition {
        task: TaskId,
        from: Status,
        action: &'static str,
    },
    NoOutputs(TaskId),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTask(id) => write!(f, "{id} does not exist"),
            Self::NoAcceptance => f.write_str("task card needs at least one acceptance criterion"),
            Self::UnknownDependency(id) => write!(f, "prerequisite {id} does not exist"),
            Self::NotReady(id) => write!(f, "{id} has unfinished prerequisites"),
            Self::SelfReview { task, agent } => {
                write!(f, "{agent:?} cannot review its own work in {task}")
            }
            Self::InvalidTransition { task, from, action } => {
                write!(f, "cannot {action} {task} while {from:?}")
            }
            Self::NoOutputs(id) => write!(f, "{id} must return at least one blackboard entry"),
        }
    }
}

impl std::error::Error for PlanError {}

/// The task cards for one goal, and the only way to change their state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    goal: GoalId,
    tasks: Vec<Task>,
}

impl Plan {
    /// An empty plan for `goal`.
    pub fn new(goal: GoalId) -> Self {
        Self {
            goal,
            tasks: Vec::new(),
        }
    }

    /// The goal this plan serves.
    pub fn goal(&self) -> GoalId {
        self.goal
    }

    /// All cards in insertion order.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// Look up a card.
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.tasks.get(index(id)?)
    }

    /// Add a card. Prerequisites must already be in the plan.
    pub fn add(&mut self, spec: TaskSpec) -> Result<TaskId, PlanError> {
        if spec.acceptance.is_empty() {
            return Err(PlanError::NoAcceptance);
        }
        let id = TaskId::from_raw(self.tasks.len() as u64 + 1);
        let task = Task {
            id,
            spec,
            state: TaskState::Pending,
        };
        if let Some(missing) = task.prerequisites().find(|dep| self.get(*dep).is_none()) {
            return Err(PlanError::UnknownDependency(missing));
        }
        self.tasks.push(task);
        Ok(id)
    }

    /// Pending cards whose prerequisites are all done.
    pub fn ready(&self) -> impl Iterator<Item = &Task> {
        self.tasks
            .iter()
            .filter(|t| t.state == TaskState::Pending && self.prerequisites_done(t))
    }

    /// True once every card is done (and there is at least one).
    pub fn is_finished(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|t| t.state.status() == Status::Done)
    }

    /// Hand a ready card to `agent`. Reviews reject the target's author.
    pub fn start(&mut self, id: TaskId, agent: Agent) -> Result<(), PlanError> {
        let task = self.task(id)?;
        if task.state != TaskState::Pending {
            return Err(invalid(task, "start"));
        }
        if !self.prerequisites_done(task) {
            return Err(PlanError::NotReady(id));
        }
        if let Role::Review { target } = task.spec.role {
            if self.task(target)?.state.agent() == Some(agent) {
                return Err(PlanError::SelfReview { task: id, agent });
            }
        }
        self.set(id, TaskState::Running { agent })
    }

    /// Park a running card on an open question.
    pub fn block(&mut self, id: TaskId, question: EntryId) -> Result<(), PlanError> {
        let agent = self.running_agent(id, "block")?;
        self.set(id, TaskState::Blocked { agent, question })
    }

    /// Return a blocked card to its agent.
    pub fn resume(&mut self, id: TaskId) -> Result<(), PlanError> {
        let task = self.task(id)?;
        let TaskState::Blocked { agent, .. } = task.state else {
            return Err(invalid(task, "resume"));
        };
        self.set(id, TaskState::Running { agent })
    }

    /// Finish a running card with the entries it wrote.
    pub fn complete(&mut self, id: TaskId, outputs: Vec<EntryId>) -> Result<(), PlanError> {
        let agent = self.running_agent(id, "complete")?;
        if outputs.is_empty() {
            return Err(PlanError::NoOutputs(id));
        }
        self.set(id, TaskState::Done { agent, outputs })
    }

    /// Mark a running card failed.
    pub fn fail(&mut self, id: TaskId, reason: Text) -> Result<(), PlanError> {
        let agent = self.running_agent(id, "fail")?;
        self.set(id, TaskState::Failed { agent, reason })
    }

    fn task(&self, id: TaskId) -> Result<&Task, PlanError> {
        self.get(id).ok_or(PlanError::UnknownTask(id))
    }

    fn set(&mut self, id: TaskId, state: TaskState) -> Result<(), PlanError> {
        let task = index(id)
            .and_then(|i| self.tasks.get_mut(i))
            .ok_or(PlanError::UnknownTask(id))?;
        task.state = state;
        Ok(())
    }

    fn running_agent(&self, id: TaskId, action: &'static str) -> Result<Agent, PlanError> {
        let task = self.task(id)?;
        match task.state {
            TaskState::Running { agent } => Ok(agent),
            _ => Err(invalid(task, action)),
        }
    }

    fn prerequisites_done(&self, task: &Task) -> bool {
        task.prerequisites().all(|dep| {
            self.get(dep)
                .is_some_and(|t| t.state.status() == Status::Done)
        })
    }
}

/// Ids are 1-based positions in the plan.
fn index(id: TaskId) -> Option<usize> {
    usize::try_from(id.get().checked_sub(1)?).ok()
}

fn invalid(task: &Task, action: &'static str) -> PlanError {
    PlanError::InvalidTransition {
        task: task.id,
        from: task.state.status(),
        action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Text {
        Text::new(s).expect("non-blank")
    }

    fn spec(role: Role, depends_on: Vec<TaskId>) -> TaskSpec {
        TaskSpec {
            role,
            instruction: text("do the thing"),
            acceptance: vec![text("tests pass")],
            refs: vec![],
            depends_on,
            max_calls: NonZeroU32::new(3).expect("non-zero"),
        }
    }

    fn e(n: u64) -> EntryId {
        EntryId::from_raw(n)
    }

    fn plan() -> Plan {
        Plan::new(GoalId::from_raw(1))
    }

    #[test]
    fn add_assigns_sequential_ids() {
        let mut p = plan();
        assert_eq!(p.add(spec(Role::Research, vec![])), Ok(TaskId::from_raw(1)));
        assert_eq!(
            p.add(spec(Role::Design, vec![TaskId::from_raw(1)])),
            Ok(TaskId::from_raw(2))
        );
    }

    #[test]
    fn add_rejects_missing_acceptance() {
        let mut s = spec(Role::Research, vec![]);
        s.acceptance.clear();
        assert_eq!(plan().add(s), Err(PlanError::NoAcceptance));
    }

    #[test]
    fn add_rejects_unknown_dependency_and_review_target() {
        let mut p = plan();
        let ghost = TaskId::from_raw(9);
        assert_eq!(
            p.add(spec(Role::Design, vec![ghost])),
            Err(PlanError::UnknownDependency(ghost))
        );
        assert_eq!(
            p.add(spec(Role::Review { target: ghost }, vec![])),
            Err(PlanError::UnknownDependency(ghost))
        );
        assert!(p.tasks().is_empty());
    }

    #[test]
    fn ready_waits_for_prerequisites() {
        let mut p = plan();
        let a = p.add(spec(Role::Research, vec![])).expect("add");
        let b = p.add(spec(Role::Design, vec![a])).expect("add");
        assert_eq!(p.ready().map(Task::id).collect::<Vec<_>>(), vec![a]);
        assert_eq!(p.start(b, Agent::Claude), Err(PlanError::NotReady(b)));

        p.start(a, Agent::Gemini).expect("start");
        p.complete(a, vec![e(1)]).expect("complete");
        assert_eq!(p.ready().map(Task::id).collect::<Vec<_>>(), vec![b]);
    }

    #[test]
    fn review_rejects_author_and_accepts_other_agent() {
        let mut p = plan();
        let work = p.add(spec(Role::Implement, vec![])).expect("add");
        let review = p
            .add(spec(Role::Review { target: work }, vec![]))
            .expect("add");
        p.start(work, Agent::Codex).expect("start");
        p.complete(work, vec![e(1)]).expect("complete");

        assert_eq!(
            p.start(review, Agent::Codex),
            Err(PlanError::SelfReview {
                task: review,
                agent: Agent::Codex
            })
        );
        assert_eq!(p.start(review, Agent::Claude), Ok(()));
    }

    #[test]
    fn complete_requires_outputs() {
        let mut p = plan();
        let a = p.add(spec(Role::Research, vec![])).expect("add");
        p.start(a, Agent::Gemini).expect("start");
        assert_eq!(p.complete(a, vec![]), Err(PlanError::NoOutputs(a)));
    }

    #[test]
    fn rejects_out_of_order_transitions() {
        let mut p = plan();
        let a = p.add(spec(Role::Research, vec![])).expect("add");
        assert_eq!(
            p.complete(a, vec![e(1)]),
            Err(PlanError::InvalidTransition {
                task: a,
                from: Status::Pending,
                action: "complete"
            })
        );
        assert_eq!(
            p.resume(a),
            Err(PlanError::InvalidTransition {
                task: a,
                from: Status::Pending,
                action: "resume"
            })
        );
    }

    #[test]
    fn block_and_resume_keep_the_agent() {
        let mut p = plan();
        let a = p.add(spec(Role::Design, vec![])).expect("add");
        p.start(a, Agent::Claude).expect("start");
        p.block(a, e(4)).expect("block");
        assert_eq!(p.get(a).map(|t| t.state().status()), Some(Status::Blocked));
        p.resume(a).expect("resume");
        assert_eq!(
            p.get(a).map(|t| t.state().clone()),
            Some(TaskState::Running {
                agent: Agent::Claude
            })
        );
    }

    #[test]
    fn finished_only_when_all_done() {
        let mut p = plan();
        assert!(!p.is_finished());
        let a = p.add(spec(Role::Triage, vec![])).expect("add");
        p.start(a, Agent::Local).expect("start");
        p.fail(a, text("model timeout")).expect("fail");
        assert!(!p.is_finished());
    }

    #[test]
    fn unknown_task_is_reported() {
        let ghost = TaskId::from_raw(0);
        assert_eq!(
            plan().start(ghost, Agent::Claude),
            Err(PlanError::UnknownTask(ghost))
        );
    }
}
