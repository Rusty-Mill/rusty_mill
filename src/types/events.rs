//! Streaming/result wrapper types: `TaskStatusUpdateEvent`,
//! `TaskArtifactUpdateEvent`, `StreamResponse`, and the `SendMessage`
//! result union (spec Section 4.2, 3.2.3, proto `StreamResponse`,
//! `SendMessageResponse`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::message::Message;
use super::task::{Artifact, Task, TaskStatus};

/// Notifies the client of a change in a task's status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatusUpdateEvent {
    #[serde(rename = "taskId")]
    pub task_id: String,
    #[serde(rename = "contextId")]
    pub context_id: String,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata: Option<Map<String, Value>>,
}

/// Notifies the client that an artifact has been generated or updated for
/// a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskArtifactUpdateEvent {
    #[serde(rename = "taskId")]
    pub task_id: String,
    #[serde(rename = "contextId")]
    pub context_id: String,
    pub artifact: Artifact,
    /// If true, this artifact's content should be appended to a
    /// previously sent artifact with the same id.
    #[serde(default)]
    pub append: bool,
    /// If true, this is the final chunk of the artifact.
    #[serde(rename = "lastChunk", default)]
    pub last_chunk: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata: Option<Map<String, Value>>,
}

/// The result of a (non-streaming) `SendMessage` call: either a `Task` was
/// created/updated, or the agent replied directly with a `Message` and no
/// task was created (proto `SendMessageResponse`, oneof `payload`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SendMessageResult {
    Task { task: Task },
    Message { message: Message },
}

impl SendMessageResult {
    pub fn as_task(&self) -> Option<&Task> {
        match self {
            SendMessageResult::Task { task } => Some(task),
            _ => None,
        }
    }

    pub fn as_message(&self) -> Option<&Message> {
        match self {
            SendMessageResult::Message { message } => Some(message),
            _ => None,
        }
    }
}

/// A single item in a `SendStreamingMessage` / `SubscribeToTask` SSE
/// stream (proto `StreamResponse`, oneof `payload`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StreamResponse {
    Task {
        task: Task,
    },
    Message {
        message: Message,
    },
    StatusUpdate {
        #[serde(rename = "statusUpdate")]
        status_update: TaskStatusUpdateEvent,
    },
    ArtifactUpdate {
        #[serde(rename = "artifactUpdate")]
        artifact_update: TaskArtifactUpdateEvent,
    },
}

impl StreamResponse {
    /// True if this event represents the end of the stream: a terminal or
    /// interrupted task status, or a bare `Message` (which is not
    /// followed by any further events).
    pub fn closes_stream(&self) -> bool {
        match self {
            StreamResponse::Message { .. } => true,
            StreamResponse::StatusUpdate { status_update } => {
                status_update.status.state.is_final_for_blocking_send()
            }
            _ => false,
        }
    }
}

impl From<Task> for StreamResponse {
    fn from(task: Task) -> Self {
        StreamResponse::Task { task }
    }
}

impl From<Message> for StreamResponse {
    fn from(message: Message) -> Self {
        StreamResponse::Message { message }
    }
}

impl From<TaskStatusUpdateEvent> for StreamResponse {
    fn from(status_update: TaskStatusUpdateEvent) -> Self {
        StreamResponse::StatusUpdate { status_update }
    }
}

impl From<TaskArtifactUpdateEvent> for StreamResponse {
    fn from(artifact_update: TaskArtifactUpdateEvent) -> Self {
        StreamResponse::ArtifactUpdate { artifact_update }
    }
}
