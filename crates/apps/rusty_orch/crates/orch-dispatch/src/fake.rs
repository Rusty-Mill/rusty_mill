//! A scripted [`AgentRunner`] for tests. Replies are consumed in call order
//! and every call is recorded, so a test can assert both what the dispatcher
//! did and whom it asked.

use std::collections::VecDeque;

use orch_core::board::{Board, EntryKind};
use orch_core::task::{Agent, Task};
use orch_core::{TaskId, Text};

use crate::{AgentError, AgentRunner, Output};

/// What the fake returns for one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Write one entry per kind, with a generated body and no refs.
    Write(Vec<EntryKind>),
    /// Fail the call.
    Fail(String),
}

/// Scripted agent. An exhausted script fails the call rather than panicking.
#[derive(Debug, Default)]
pub struct FakeAgent {
    script: VecDeque<Reply>,
    calls: Vec<(Agent, TaskId)>,
}

impl FakeAgent {
    /// A fake that answers calls with `script`, in order.
    pub fn new(script: impl IntoIterator<Item = Reply>) -> Self {
        Self {
            script: script.into_iter().collect(),
            calls: Vec::new(),
        }
    }

    /// Every `(agent, task)` pair the dispatcher asked for, in order.
    pub fn calls(&self) -> &[(Agent, TaskId)] {
        &self.calls
    }
}

impl AgentRunner for FakeAgent {
    fn run(&mut self, agent: Agent, task: &Task, _: &Board) -> Result<Vec<Output>, AgentError> {
        self.calls.push((agent, task.id()));
        match self.script.pop_front() {
            None => Err(AgentError("fake agent: script exhausted".to_owned())),
            Some(Reply::Fail(reason)) => Err(AgentError(reason)),
            Some(Reply::Write(kinds)) => kinds.into_iter().map(output).collect(),
        }
    }
}

fn output(kind: EntryKind) -> Result<Output, AgentError> {
    let body = Text::new(&format!("fake {kind:?}"))
        .ok_or_else(|| AgentError("fake agent: blank body".to_owned()))?;
    Ok(Output {
        kind,
        body,
        refs: Vec::new(),
        supersedes: None,
    })
}
