//! Long-lived task hosting, separate from metered model invocations.
use crate::{ClassifiedError, Output};
use orch_core::board::{Board, Entry};
use orch_core::task::Task;
use orch_core::{Ref, TaskId, Text};

/// Opaque durable pointer, stored on the task's Board as a URL reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostHandle(pub Text);
/// The exact context shown to a human. Persisted with the question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateContext {
    pub prompt: Text,
    pub rev: u64,
    pub ids: Vec<Text>,
}
/// A hosted task completes only after its host records a terminal receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Working,
    Gate(GateContext),
    Closed { outputs: Vec<Output> },
    Cancelled { reason: Text },
}
/// A stale answer must be shown again, even when its text still looks valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relayed {
    Applied,
    Stale(Progress),
}
/// Idempotent, crash-recoverable task boundary. Poll and relay are unmetered.
pub trait TaskHost: std::fmt::Debug {
    fn open(&mut self, task: &Task, board: &Board) -> Result<HostHandle, ClassifiedError>;
    fn poll(&mut self, handle: &HostHandle) -> Result<Progress, ClassifiedError>;
    fn relay(
        &mut self,
        handle: &HostHandle,
        gate: &Entry,
        answer: &Text,
    ) -> Result<Relayed, ClassifiedError>;
}
/// Find the host pointer saved by the dispatcher, without adapter memory.
pub fn hosted_handle(board: &Board, task: TaskId) -> Option<HostHandle> {
    board
        .live()
        .filter(|e| e.content().task == Some(task))
        .filter(|e| e.content().kind == orch_core::board::EntryKind::Artifact)
        .filter(|e| e.content().body.as_str() == "hosted task")
        .flat_map(|e| &e.content().refs)
        .find_map(|r| match r {
            Ref::Url(t) => Some(HostHandle(t.clone())),
            _ => None,
        })
}
