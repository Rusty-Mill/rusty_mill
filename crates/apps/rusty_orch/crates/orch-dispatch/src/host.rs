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
    /// Invalid human input: replace the answered question so the human can correct it.
    Rejected(Progress),
}
/// The authority a host command is issued under: one generation of an
/// orchestrator assignment, held by one process.
///
/// A host admits the first command of a higher generation for a task and
/// from then on refuses lower generations, and other holders of the same
/// generation, as [`ClassifiedError::Unavailable`]: a superseded
/// orchestrator stops driving the task without failing it. A different
/// assignment for the same task is refused as permanent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    /// The assignment that owns the hosted task.
    pub assignment: Text,
    /// The assignment generation, persisted before any command uses it.
    pub generation: u64,
    /// Unique per holding process, so two holders that reached the same
    /// generation from diverged records cannot both act.
    pub holder: Text,
}
/// Idempotent, crash-recoverable task boundary. Poll and relay are unmetered.
/// Every command carries the caller's [`Fence`].
pub trait TaskHost: std::fmt::Debug {
    fn open(
        &mut self,
        task: &Task,
        board: &Board,
        fence: &Fence,
    ) -> Result<HostHandle, ClassifiedError>;
    fn poll(&mut self, handle: &HostHandle, fence: &Fence) -> Result<Progress, ClassifiedError>;
    fn relay(
        &mut self,
        handle: &HostHandle,
        gate: &Entry,
        answer: &Text,
        fence: &Fence,
    ) -> Result<Relayed, ClassifiedError>;
    /// Cancel the hosted execution of `task`, and return only once nothing
    /// it started still runs, so its capacity is free. The host finds the
    /// execution itself, without a recorded [`HostHandle`]: an open may have
    /// started work before its handle reached the board. An execution never
    /// opened is left unable to start. Idempotent: an execution that already
    /// ended returns its terminal progress unchanged, which may be `Closed`.
    fn cancel(
        &mut self,
        task: &Task,
        board: &Board,
        reason: &Text,
        fence: &Fence,
    ) -> Result<Progress, ClassifiedError>;
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
