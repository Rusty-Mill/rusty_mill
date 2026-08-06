//! The [`AgentExecutor`] trait: the interface an application implements to
//! define what an A2A agent actually does.

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::Result;
use crate::types::{
    Artifact, Message, StreamResponse, Task, TaskArtifactUpdateEvent, TaskState, TaskStatus,
    TaskStatusUpdateEvent,
};

/// Everything an [`AgentExecutor`] needs to know about one incoming
/// message: the message itself, the task it continues (if any), and the
/// task/context ids to use for any task this invocation creates.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// The inbound message that triggered this execution.
    pub message: Message,
    /// The existing task this message continues, if `message.taskId` was
    /// set (spec Section 3.4.2). `None` for a new conversation turn.
    pub task: Option<Task>,
    /// The id to use if this invocation creates or updates a task: either
    /// `task.id` when continuing an existing task, or a freshly generated
    /// id for a new one.
    pub task_id: String,
    /// The id to use if this invocation creates or updates a task: either
    /// the existing task/message's `contextId`, or a freshly generated one
    /// (spec Section 3.4.1).
    pub context_id: String,
    /// Signaled when the client cancels this task (`CancelTask`).
    /// Long-running executors should check this periodically (or
    /// `select!` against [`CancellationToken::cancelled`]) and stop
    /// promptly when it fires.
    pub cancellation: CancellationToken,
}

/// A handle an [`AgentExecutor`] uses to report progress and results back
/// to the server harness. Emitting a bare [`Message`] and never touching
/// task state produces a task-less `SendMessage` reply (spec Section 3.7:
/// "Agents may send Messages back to the client to request clarification
/// prior to initiating a task"). Emitting a status or artifact update
/// causes the harness to create/update a [`Task`] under `context.task_id`.
#[derive(Clone)]
pub struct EventSink {
    task_id: String,
    context_id: String,
    tx: mpsc::UnboundedSender<StreamResponse>,
}

impl EventSink {
    pub(crate) fn new(
        task_id: String,
        context_id: String,
        tx: mpsc::UnboundedSender<StreamResponse>,
    ) -> Self {
        EventSink {
            task_id,
            context_id,
            tx,
        }
    }

    /// Sends a direct message reply. If this is the first event emitted
    /// for this invocation, no task is created and `SendMessage` returns
    /// this message directly.
    pub fn message(&self, message: Message) {
        let _ = self.tx.send(StreamResponse::Message { message });
    }

    /// Transitions the task to `state`, with no attached status message.
    pub fn status(&self, state: TaskState) {
        self.status_with_message(state, None);
    }

    /// Transitions the task to `state`, attaching a status message (e.g.
    /// an explanation of why the task failed, or what input is required).
    pub fn status_with_message(&self, state: TaskState, message: Option<Message>) {
        let event = TaskStatusUpdateEvent {
            task_id: self.task_id.clone(),
            context_id: self.context_id.clone(),
            status: TaskStatus {
                state,
                message,
                timestamp: Some(Utc::now()),
            },
            metadata: None,
        };
        let _ = self.tx.send(event.into());
    }

    /// Publishes a complete artifact (equivalent to
    /// `artifact_update(artifact, false, true)`).
    pub fn artifact(&self, artifact: Artifact) {
        self.artifact_update(artifact, false, true);
    }

    /// Publishes an artifact chunk. Set `append` to fold this chunk into a
    /// previously sent artifact with the same id; set `last_chunk` once
    /// the artifact is complete.
    pub fn artifact_update(&self, artifact: Artifact, append: bool, last_chunk: bool) {
        let event = TaskArtifactUpdateEvent {
            task_id: self.task_id.clone(),
            context_id: self.context_id.clone(),
            artifact,
            append,
            last_chunk,
            metadata: None,
        };
        let _ = self.tx.send(event.into());
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn context_id(&self) -> &str {
        &self.context_id
    }
}

/// Implement this trait to define an A2A agent's behavior.
///
/// `execute` is invoked once per inbound `SendMessage` /
/// `SendStreamingMessage` call. Use `events` to report progress: call
/// [`EventSink::status`] to transition the task through `Working` and on
/// to a terminal or interrupted state, and [`EventSink::artifact`] to
/// publish results. Returning `Ok(())` without ever reaching a terminal or
/// interrupted state (or without ever emitting anything) is treated as an
/// implementation error by the harness.
#[async_trait]
pub trait AgentExecutor: Send + Sync {
    async fn execute(&self, context: RequestContext, events: EventSink) -> Result<()>;

    /// Invoked when a client calls `CancelTask` for a task this executor
    /// is (or was) processing. The default implementation just reports
    /// `TASK_STATE_CANCELED`; override to release resources or produce a
    /// more specific final message. `context.cancellation` is already
    /// signaled by the time this is called.
    async fn cancel(&self, _context: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Canceled);
        Ok(())
    }
}
