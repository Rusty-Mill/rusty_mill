//! Push notification configuration (spec Section 4.3, proto
//! `AuthenticationInfo`, `TaskPushNotificationConfig`).

use serde::{Deserialize, Serialize};

/// Authentication details attached to outbound push notification
/// deliveries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationInfo {
    /// An HTTP authentication scheme name from the IANA registry, e.g.
    /// `"Bearer"`, `"Basic"`. Case-insensitive per RFC 9110 Section 11.1.
    pub scheme: String,
    /// Credentials whose format depends on `scheme` (e.g. a bearer token).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub credentials: Option<String>,
}

/// Associates a push notification configuration with a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPushNotificationConfig {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    /// A unique identifier for this configuration. Server-assigned if
    /// omitted on creation.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub id: Option<String>,
    /// The task this configuration is associated with. Should be empty
    /// when supplied inline in a `SendMessage` request.
    #[serde(rename = "taskId", skip_serializing_if = "Option::is_none", default)]
    pub task_id: Option<String>,
    /// The URL notifications are POSTed to.
    pub url: String,
    /// An opaque token unique to this task or session, echoed back in
    /// notification deliveries so the receiver can correlate them.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub authentication: Option<AuthenticationInfo>,
}

impl TaskPushNotificationConfig {
    pub fn new(url: impl Into<String>) -> Self {
        TaskPushNotificationConfig {
            tenant: None,
            id: None,
            task_id: None,
            url: url.into(),
            token: None,
            authentication: None,
        }
    }
}
