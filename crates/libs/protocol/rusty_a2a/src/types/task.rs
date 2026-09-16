//! `Task`, `TaskStatus`, `TaskState` and `Artifact` (spec Section
//! 4.1.1-4.1.3, 4.1.7 / proto `Task`, `TaskStatus`, `TaskState`,
//! `Artifact`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::message::{Message, Part};

/// The lifecycle state of a [`Task`] (proto `TaskState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    #[serde(rename = "TASK_STATE_UNSPECIFIED")]
    Unspecified,
    /// The task has been successfully submitted and acknowledged.
    #[serde(rename = "TASK_STATE_SUBMITTED")]
    Submitted,
    /// The task is actively being processed by the agent.
    #[serde(rename = "TASK_STATE_WORKING")]
    Working,
    /// Terminal: the task finished successfully.
    #[serde(rename = "TASK_STATE_COMPLETED")]
    Completed,
    /// Terminal: the task finished with an error.
    #[serde(rename = "TASK_STATE_FAILED")]
    Failed,
    /// Terminal: the task was canceled before completion.
    #[serde(rename = "TASK_STATE_CANCELED")]
    Canceled,
    /// Interrupted: the agent requires additional user input to proceed.
    #[serde(rename = "TASK_STATE_INPUT_REQUIRED")]
    InputRequired,
    /// Terminal: the agent decided not to perform the task.
    #[serde(rename = "TASK_STATE_REJECTED")]
    Rejected,
    /// Interrupted: authentication is required to proceed.
    #[serde(rename = "TASK_STATE_AUTH_REQUIRED")]
    AuthRequired,
}

impl TaskState {
    /// Terminal states never transition further:
    /// `COMPLETED`, `FAILED`, `CANCELED`, `REJECTED`.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Failed | TaskState::Canceled | TaskState::Rejected
        )
    }

    /// Interrupted states are non-terminal but require more input before
    /// the task can proceed: `INPUT_REQUIRED`, `AUTH_REQUIRED`.
    pub fn is_interrupted(&self) -> bool {
        matches!(self, TaskState::InputRequired | TaskState::AuthRequired)
    }

    /// A blocking `SendMessage` call returns once the task reaches a
    /// terminal or interrupted state (spec Section 3.2.2).
    pub fn is_final_for_blocking_send(&self) -> bool {
        self.is_terminal() || self.is_interrupted()
    }
}

/// The current status of a [`Task`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatus {
    pub state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<Message>,
    #[serde(
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub timestamp: Option<DateTime<Utc>>,
}

impl TaskStatus {
    pub fn new(state: TaskState) -> Self {
        TaskStatus {
            state,
            message: None,
            timestamp: Some(Utc::now()),
        }
    }

    pub fn with_message(mut self, message: Message) -> Self {
        self.message = Some(message);
        self
    }
}

/// A task output (spec Section 4.1.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    #[serde(rename = "artifactId")]
    pub artifact_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub extensions: Vec<String>,
}

impl Artifact {
    pub fn new(artifact_id: impl Into<String>, parts: Vec<Part>) -> Self {
        Artifact {
            artifact_id: artifact_id.into(),
            name: None,
            description: None,
            parts,
            metadata: None,
            extensions: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// The core unit of action for A2A (spec Section 4.1.1). Has a current
/// [`TaskStatus`]; results are stored as [`Artifact`]s; multi-turn
/// exchanges are recorded in `history`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    #[serde(rename = "contextId", skip_serializing_if = "Option::is_none", default)]
    pub context_id: Option<String>,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub artifacts: Vec<Artifact>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub history: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata: Option<Map<String, Value>>,
}

impl Task {
    pub fn new(id: impl Into<String>, context_id: impl Into<String>, state: TaskState) -> Self {
        Task {
            id: id.into(),
            context_id: Some(context_id.into()),
            status: TaskStatus::new(state),
            artifacts: Vec::new(),
            history: Vec::new(),
            metadata: None,
        }
    }
}
