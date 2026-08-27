//! Per-operation request/response objects (spec Section 3.2, 3.1, proto
//! `SendMessageRequest`, `GetTaskRequest`, `ListTasksRequest`, ...).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::message::Message;
use super::push_notification::TaskPushNotificationConfig;
use super::task::{Task, TaskState};

/// Configuration for a `SendMessage` / `SendStreamingMessage` request
/// (spec Section 3.2.2).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SendMessageConfiguration {
    /// Media types the client is prepared to accept for response parts.
    #[serde(
        rename = "acceptedOutputModes",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    pub accepted_output_modes: Vec<String>,
    #[serde(
        rename = "taskPushNotificationConfig",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub task_push_notification_config: Option<TaskPushNotificationConfig>,
    /// Maximum number of most recent history messages to include in the
    /// response. See `SendMessageRequest` docs for full semantics.
    #[serde(rename = "historyLength", skip_serializing_if = "Option::is_none", default)]
    pub history_length: Option<i32>,
    /// If `true`, return immediately after creating the task rather than
    /// blocking until it reaches a terminal/interrupted state.
    #[serde(rename = "returnImmediately", default)]
    pub return_immediately: bool,
}

/// Parameters for `SendMessage` / `SendStreamingMessage`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    pub message: Message,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub configuration: Option<SendMessageConfiguration>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata: Option<Map<String, Value>>,
}

impl SendMessageRequest {
    pub fn new(message: Message) -> Self {
        SendMessageRequest {
            tenant: None,
            message,
            configuration: None,
            metadata: None,
        }
    }
}

/// Parameters for `GetTask`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTaskRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    pub id: String,
    #[serde(rename = "historyLength", skip_serializing_if = "Option::is_none", default)]
    pub history_length: Option<i32>,
}

/// Parameters for `ListTasks`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListTasksRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    #[serde(rename = "contextId", skip_serializing_if = "Option::is_none", default)]
    pub context_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub status: Option<TaskState>,
    #[serde(rename = "pageSize", skip_serializing_if = "Option::is_none", default)]
    pub page_size: Option<i32>,
    #[serde(rename = "pageToken", skip_serializing_if = "Option::is_none", default)]
    pub page_token: Option<String>,
    #[serde(rename = "historyLength", skip_serializing_if = "Option::is_none", default)]
    pub history_length: Option<i32>,
    #[serde(
        rename = "statusTimestampAfter",
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub status_timestamp_after: Option<DateTime<Utc>>,
    #[serde(rename = "includeArtifacts", skip_serializing_if = "Option::is_none", default)]
    pub include_artifacts: Option<bool>,
}

/// Result of `ListTasks`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTasksResponse {
    pub tasks: Vec<Task>,
    #[serde(rename = "nextPageToken", default)]
    pub next_page_token: String,
    #[serde(rename = "pageSize")]
    pub page_size: i32,
    #[serde(rename = "totalSize")]
    pub total_size: i32,
}

/// Parameters for `CancelTask`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelTaskRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata: Option<Map<String, Value>>,
}

/// Parameters for `SubscribeToTask`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeToTaskRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    pub id: String,
}

/// Parameters for `GetTaskPushNotificationConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTaskPushNotificationConfigRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    #[serde(rename = "taskId")]
    pub task_id: String,
    pub id: String,
}

/// Parameters for `DeleteTaskPushNotificationConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteTaskPushNotificationConfigRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    #[serde(rename = "taskId")]
    pub task_id: String,
    pub id: String,
}

/// Parameters for `ListTaskPushNotificationConfigs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTaskPushNotificationConfigsRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    #[serde(rename = "taskId")]
    pub task_id: String,
    #[serde(rename = "pageSize", skip_serializing_if = "Option::is_none", default)]
    pub page_size: Option<i32>,
    #[serde(rename = "pageToken", skip_serializing_if = "Option::is_none", default)]
    pub page_token: Option<String>,
}

/// Result of `ListTaskPushNotificationConfigs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTaskPushNotificationConfigsResponse {
    #[serde(default)]
    pub configs: Vec<TaskPushNotificationConfig>,
    #[serde(rename = "nextPageToken", default)]
    pub next_page_token: String,
}

/// Parameters for `GetExtendedAgentCard`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GetExtendedAgentCardRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
}
