//! The A2A protocol data model (spec Section 4), transliterated field-for-
//! field from the normative `specification/a2a.proto` definition, with
//! `camelCase` JSON field naming per spec Section 5.5.

mod agent_card;
mod events;
pub mod jsonrpc;
mod message;
mod push_notification;
mod requests;
mod security;
mod task;

pub use agent_card::{
    AgentCapabilities, AgentCard, AgentCardSignature, AgentExtension, AgentInterface, AgentProvider,
    AgentSkill,
};
pub use events::{SendMessageResult, StreamResponse, TaskArtifactUpdateEvent, TaskStatusUpdateEvent};
pub use message::{Message, Part, PartContent, Role};
pub use push_notification::{AuthenticationInfo, TaskPushNotificationConfig};
pub use requests::{
    CancelTaskRequest, DeleteTaskPushNotificationConfigRequest, GetExtendedAgentCardRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, ListTaskPushNotificationConfigsRequest,
    ListTaskPushNotificationConfigsResponse, ListTasksRequest, ListTasksResponse, SendMessageConfiguration,
    SendMessageRequest, SubscribeToTaskRequest,
};
pub use security::{
    ApiKeySecurityScheme, AuthorizationCodeOAuthFlow, ClientCredentialsOAuthFlow, DeviceCodeOAuthFlow,
    HttpAuthSecurityScheme, ImplicitOAuthFlow, MutualTlsSecurityScheme, OAuth2SecurityScheme, OAuthFlows,
    OpenIdConnectSecurityScheme, PasswordOAuthFlow, SecurityRequirement, SecurityScheme, StringList,
};
pub use task::{Artifact, Task, TaskState, TaskStatus};
