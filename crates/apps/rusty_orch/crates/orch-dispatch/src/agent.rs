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

/// Why an agent could not complete a call, and whether trying again can help.
///
/// The dispatcher keys retry policy on the variant alone, never on the
/// message (ADR-0008). Adapters pick `Permanent` only for conditions that
/// no later call on the same card can clear: a missing login, a card the
/// adapter cannot serve. Everything else, including malformed model output,
/// is `Transient`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentError {
    /// May succeed on retry. The card stays `Running` under its ceilings.
    Transient(String),
    /// Will not succeed on retry. The dispatcher fails the card at once.
    Permanent(String),
}

impl AgentError {
    /// The human-readable reason, without the retry classification.
    pub fn message(&self) -> &str {
        match self {
            Self::Transient(m) | Self::Permanent(m) => m,
        }
    }

    /// Whether retrying the call is pointless.
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
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
