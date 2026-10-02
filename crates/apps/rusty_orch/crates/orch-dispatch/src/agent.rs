//! The agent port: one model call in, blackboard entries out.

use std::fmt;

use orch_core::board::{Board, EntryKind};
use orch_core::task::{Agent, Role, Task};
use orch_core::{EntryId, Ref, Text};

/// One entry an agent wants on the board. The dispatcher stamps the task
/// and author, so an adapter cannot write on behalf of another agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub kind: EntryKind,
    /// Short statement. Detail belongs behind `refs`.
    pub body: Text,
    pub refs: Vec<Ref>,
    pub supersedes: Option<EntryId>,
}

/// Why an agent could not complete a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentError(pub String);

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AgentError {}

/// Port for anything that can run a task card on a model backend.
///
/// One call per invocation: the dispatcher meters calls, so an adapter must
/// not loop internally. The agent reads context from `board` and the card's
/// refs, and returns what it wrote; it never appends to the board itself.
pub trait AgentRunner {
    /// Whether this runner can serve `role` through `agent`.
    ///
    /// The default preserves compatibility for runners that can serve every
    /// route. Adapters with narrower capabilities must override it so the
    /// dispatcher can reject unsupported cards before counting or invoking a
    /// call.
    fn supports(&self, _agent: Agent, _role: Role) -> bool {
        true
    }

    /// Run `task` on `agent` and return the entries it produced.
    fn run(&mut self, agent: Agent, task: &Task, board: &Board) -> Result<Vec<Output>, AgentError>;
}
