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

/// Scripted long-lived host. Clones share operations and counters, so tests
/// can replace dispatchers at arbitrary interruption boundaries.
#[derive(Debug, Clone, Default)]
pub struct FakeHost(pub std::rc::Rc<std::cell::RefCell<HostScript>>);
/// Host responses and observed operations, independent of dispatcher lifetime.
#[derive(Debug, Default)]
pub struct HostScript {
    pub opens: usize,
    pub polls: usize,
    pub answers: Vec<(orch_core::board::Entry, orch_core::Text)>,
    pub progress: std::collections::VecDeque<Result<crate::Progress, crate::ClassifiedError>>,
    pub relays: std::collections::VecDeque<Result<crate::Relayed, crate::ClassifiedError>>,
    /// The fence of every command, in order.
    pub fences: Vec<crate::Fence>,
    /// The reason of every cancel that succeeded, in order.
    pub cancels: Vec<orch_core::Text>,
    /// Errors the next cancels fail with, after admitting their fence.
    pub fail_cancel: std::collections::VecDeque<crate::ClassifiedError>,
}
impl HostScript {
    /// Record `fence`, refusing it like a real host when a later generation,
    /// or another holder of the same one, has already been admitted.
    fn admit(&mut self, fence: &crate::Fence) -> Result<(), crate::ClassifiedError> {
        let superseded = self.fences.iter().any(|seen| {
            seen.generation > fence.generation
                || (seen.generation == fence.generation && seen.holder != fence.holder)
        });
        if superseded {
            return Err(crate::ClassifiedError::Unavailable(crate::AgentError(
                "fake host: superseded fence".into(),
            )));
        }
        self.fences.push(fence.clone());
        Ok(())
    }
}
impl crate::TaskHost for FakeHost {
    fn open(
        &mut self,
        _task: &orch_core::task::Task,
        _board: &orch_core::board::Board,
        fence: &crate::Fence,
    ) -> Result<crate::HostHandle, crate::ClassifiedError> {
        let mut state = self.0.borrow_mut();
        state.admit(fence)?;
        state.opens += 1;
        Ok(crate::HostHandle(
            orch_core::Text::new("fake:durable-task").ok_or_else(|| {
                crate::ClassifiedError::Permanent(crate::AgentError(
                    "invalid fixture handle".into(),
                ))
            })?,
        ))
    }
    fn poll(
        &mut self,
        _handle: &crate::HostHandle,
        fence: &crate::Fence,
    ) -> Result<crate::Progress, crate::ClassifiedError> {
        let mut state = self.0.borrow_mut();
        state.admit(fence)?;
        state.polls += 1;
        state
            .progress
            .pop_front()
            .unwrap_or(Ok(crate::Progress::Working))
    }
    fn relay(
        &mut self,
        _handle: &crate::HostHandle,
        gate: &orch_core::board::Entry,
        answer: &orch_core::Text,
        fence: &crate::Fence,
    ) -> Result<crate::Relayed, crate::ClassifiedError> {
        let mut state = self.0.borrow_mut();
        state.admit(fence)?;
        state.answers.push((gate.clone(), answer.clone()));
        state
            .relays
            .pop_front()
            .unwrap_or(Ok(crate::Relayed::Applied))
    }
    fn cancel(
        &mut self,
        _handle: &crate::HostHandle,
        reason: &orch_core::Text,
        fence: &crate::Fence,
    ) -> Result<crate::Progress, crate::ClassifiedError> {
        let mut state = self.0.borrow_mut();
        state.admit(fence)?;
        if let Some(e) = state.fail_cancel.pop_front() {
            return Err(e);
        }
        state.cancels.push(reason.clone());
        Ok(crate::Progress::Cancelled {
            reason: reason.clone(),
        })
    }
}
