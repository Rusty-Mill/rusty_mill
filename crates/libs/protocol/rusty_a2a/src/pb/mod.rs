//! The A2A gRPC binding's messages (spec Section 10), written out from the vendored
//! `spec/a2a.proto` on [`rusty_prost`]: one table entry per proto field, in `macros`. There is
//! no code generator and no `protoc`; `tests/pb_wire.rs` holds the reference implementation's
//! bytes for every message and checks these against them.
//!
//! The shapes follow what `prost` generated, so `grpc_convert` reads the same: a message
//! field is `Option<M>`, an enum field is its `i32` (a value the schema does not know is kept,
//! see [`TaskState`]), a `oneof` is an `Option` of the enum in a module named for the message
//! (`part::Content`), `optional` scalars are `Option`s, and a map is a `BTreeMap` (sorted, so
//! encoding is deterministic). Unknown fields are dropped on decode.

#[macro_use]
mod macros;
mod service;

use std::collections::BTreeMap;

pub use rusty_prost::wkt::{Empty, Struct, Timestamp, Value};
pub use service::{a2a_service_client, a2a_service_server};

/// A value the schema has no enum member for (see the enums' `TryFrom<i32>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownEnumValue(pub i32);

impl std::fmt::Display for UnknownEnumValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown enumeration value {}", self.0)
    }
}

impl std::error::Error for UnknownEnumValue {}

/// A proto enum. Fields hold the `i32`, so a member added by a newer peer survives; `try_from`
/// is how the known ones are told apart.
macro_rules! open_enum {
    ($(#[$meta:meta])* $name:ident { $($var:ident = $num:literal : $str:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(i32)]
        pub enum $name {
            $(
                #[allow(missing_docs)]
                $var = $num,
            )*
        }

        impl $name {
            /// The name in the `.proto` (`TASK_STATE_WORKING`).
            pub fn as_str_name(&self) -> &'static str {
                match self {
                    $( Self::$var => $str, )*
                }
            }

            /// The member named `name` in the `.proto`.
            pub fn from_str_name(name: &str) -> Option<Self> {
                match name {
                    $( $str => Some(Self::$var), )*
                    _ => None,
                }
            }
        }

        impl TryFrom<i32> for $name {
            type Error = UnknownEnumValue;

            fn try_from(value: i32) -> Result<Self, UnknownEnumValue> {
                match value {
                    $( $num => Ok(Self::$var), )*
                    other => Err(UnknownEnumValue(other)),
                }
            }
        }
    };
}

open_enum! {
    /// The lifecycle state of a [`Task`].
    TaskState {
        Unspecified = 0: "TASK_STATE_UNSPECIFIED",
        Submitted = 1: "TASK_STATE_SUBMITTED",
        Working = 2: "TASK_STATE_WORKING",
        Completed = 3: "TASK_STATE_COMPLETED",
        Failed = 4: "TASK_STATE_FAILED",
        Canceled = 5: "TASK_STATE_CANCELED",
        InputRequired = 6: "TASK_STATE_INPUT_REQUIRED",
        Rejected = 7: "TASK_STATE_REJECTED",
        AuthRequired = 8: "TASK_STATE_AUTH_REQUIRED",
    }
}

open_enum! {
    /// Who sent a [`Message`].
    Role {
        Unspecified = 0: "ROLE_UNSPECIFIED",
        User = 1: "ROLE_USER",
        Agent = 2: "ROLE_AGENT",
    }
}

// ---- tasks and messages ---------------------------------------------------------------------

message! {
    /// `SendMessageConfiguration`.
    SendMessageConfiguration {
        1 accepted_output_modes: Vec<String> = rep_string,
        2 task_push_notification_config: Option<TaskPushNotificationConfig> = msg,
        3 history_length: Option<i32> = opt_int32,
        4 return_immediately: bool = bool,
    }
}

message! {
    /// `Task`.
    Task {
        1 id: String = string,
        2 context_id: String = string,
        3 status: Option<TaskStatus> = msg,
        4 artifacts: Vec<Artifact> = rep_msg,
        5 history: Vec<Message> = rep_msg,
        6 metadata: Option<Struct> = msg,
    }
}

message! {
    /// `TaskStatus`.
    TaskStatus {
        1 state: i32 = enumv,
        2 message: Option<Message> = msg,
        3 timestamp: Option<Timestamp> = msg,
    }
}

message! {
    /// `Part`.
    Part {
        oneof content: part::Content;
        5 metadata: Option<Struct> = msg,
        6 filename: String = string,
        7 media_type: String = string,
    }
}

/// Nested types of [`Part`].
pub mod part {
    use super::*;

    oneof! {
        /// `Part.content`.
        Content {
            1 Text(String) = string,
            2 Raw(Vec<u8>) = bytes,
            3 Url(String) = string,
            4 Data(Value) = msg,
        }
    }
}

message! {
    /// `Message`.
    Message {
        1 message_id: String = string,
        2 context_id: String = string,
        3 task_id: String = string,
        4 role: i32 = enumv,
        5 parts: Vec<Part> = rep_msg,
        6 metadata: Option<Struct> = msg,
        7 extensions: Vec<String> = rep_string,
        8 reference_task_ids: Vec<String> = rep_string,
    }
}

message! {
    /// `Artifact`.
    Artifact {
        1 artifact_id: String = string,
        2 name: String = string,
        3 description: String = string,
        4 parts: Vec<Part> = rep_msg,
        5 metadata: Option<Struct> = msg,
        6 extensions: Vec<String> = rep_string,
    }
}

message! {
    /// `TaskStatusUpdateEvent`.
    TaskStatusUpdateEvent {
        1 task_id: String = string,
        2 context_id: String = string,
        3 status: Option<TaskStatus> = msg,
        4 metadata: Option<Struct> = msg,
    }
}

message! {
    /// `TaskArtifactUpdateEvent`.
    TaskArtifactUpdateEvent {
        1 task_id: String = string,
        2 context_id: String = string,
        3 artifact: Option<Artifact> = msg,
        4 append: bool = bool,
        5 last_chunk: bool = bool,
        6 metadata: Option<Struct> = msg,
    }
}

// ---- the agent card -------------------------------------------------------------------------

message! {
    /// `AuthenticationInfo`.
    AuthenticationInfo {
        1 scheme: String = string,
        2 credentials: String = string,
    }
}

message! {
    /// `AgentInterface`.
    AgentInterface {
        1 url: String = string,
        2 protocol_binding: String = string,
        3 tenant: String = string,
        4 protocol_version: String = string,
    }
}

message! {
    /// `AgentCard`.
    AgentCard {
        1 name: String = string,
        2 description: String = string,
        3 supported_interfaces: Vec<AgentInterface> = rep_msg,
        4 provider: Option<AgentProvider> = msg,
        5 version: String = string,
        6 documentation_url: Option<String> = opt_string,
        7 capabilities: Option<AgentCapabilities> = msg,
        8 security_schemes: BTreeMap<String, SecurityScheme> = map_msg,
        9 security_requirements: Vec<SecurityRequirement> = rep_msg,
        10 default_input_modes: Vec<String> = rep_string,
        11 default_output_modes: Vec<String> = rep_string,
        12 skills: Vec<AgentSkill> = rep_msg,
        13 signatures: Vec<AgentCardSignature> = rep_msg,
        14 icon_url: Option<String> = opt_string,
    }
}

message! {
    /// `AgentProvider`.
    AgentProvider {
        1 url: String = string,
        2 organization: String = string,
    }
}

message! {
    /// `AgentCapabilities`.
    AgentCapabilities {
        1 streaming: Option<bool> = opt_bool,
        2 push_notifications: Option<bool> = opt_bool,
        3 extensions: Vec<AgentExtension> = rep_msg,
        4 extended_agent_card: Option<bool> = opt_bool,
    }
}

message! {
    /// `AgentExtension`.
    AgentExtension {
        1 uri: String = string,
        2 description: String = string,
        3 required: bool = bool,
        4 params: Option<Struct> = msg,
    }
}

message! {
    /// `AgentSkill`.
    AgentSkill {
        1 id: String = string,
        2 name: String = string,
        3 description: String = string,
        4 tags: Vec<String> = rep_string,
        5 examples: Vec<String> = rep_string,
        6 input_modes: Vec<String> = rep_string,
        7 output_modes: Vec<String> = rep_string,
        8 security_requirements: Vec<SecurityRequirement> = rep_msg,
    }
}

message! {
    /// `AgentCardSignature`.
    AgentCardSignature {
        1 protected: String = string,
        2 signature: String = string,
        3 header: Option<Struct> = msg,
    }
}

message! {
    /// `TaskPushNotificationConfig`.
    TaskPushNotificationConfig {
        1 tenant: String = string,
        2 id: String = string,
        3 task_id: String = string,
        4 url: String = string,
        5 token: String = string,
        6 authentication: Option<AuthenticationInfo> = msg,
    }
}

// ---- security -------------------------------------------------------------------------------

message! {
    /// `StringList`.
    StringList {
        1 list: Vec<String> = rep_string,
    }
}

message! {
    /// `SecurityRequirement`.
    SecurityRequirement {
        1 schemes: BTreeMap<String, StringList> = map_msg,
    }
}

message! {
    /// `SecurityScheme`.
    SecurityScheme {
        oneof scheme: security_scheme::Scheme;
    }
}

/// Nested types of [`SecurityScheme`].
pub mod security_scheme {
    use super::*;

    oneof! {
        /// `SecurityScheme.scheme`.
        Scheme {
            1 ApiKeySecurityScheme(ApiKeySecurityScheme) = msg,
            2 HttpAuthSecurityScheme(HttpAuthSecurityScheme) = msg,
            3 Oauth2SecurityScheme(OAuth2SecurityScheme) = msg,
            4 OpenIdConnectSecurityScheme(OpenIdConnectSecurityScheme) = msg,
            5 MtlsSecurityScheme(MutualTlsSecurityScheme) = msg,
        }
    }
}

message! {
    /// `APIKeySecurityScheme`.
    ApiKeySecurityScheme {
        1 description: String = string,
        2 location: String = string,
        3 name: String = string,
    }
}

message! {
    /// `HTTPAuthSecurityScheme`.
    HttpAuthSecurityScheme {
        1 description: String = string,
        2 scheme: String = string,
        3 bearer_format: String = string,
    }
}

message! {
    /// `OAuth2SecurityScheme`.
    OAuth2SecurityScheme {
        1 description: String = string,
        2 flows: Option<OAuthFlows> = msg,
        3 oauth2_metadata_url: String = string,
    }
}

message! {
    /// `OpenIdConnectSecurityScheme`.
    OpenIdConnectSecurityScheme {
        1 description: String = string,
        2 open_id_connect_url: String = string,
    }
}

message! {
    /// `MutualTlsSecurityScheme`.
    MutualTlsSecurityScheme {
        1 description: String = string,
    }
}

message! {
    /// `OAuthFlows`.
    OAuthFlows {
        oneof flow: o_auth_flows::Flow;
    }
}

/// Nested types of [`OAuthFlows`].
pub mod o_auth_flows {
    use super::*;

    oneof! {
        /// `OAuthFlows.flow`.
        Flow {
            1 AuthorizationCode(AuthorizationCodeOAuthFlow) = msg,
            2 ClientCredentials(ClientCredentialsOAuthFlow) = msg,
            3 Implicit(ImplicitOAuthFlow) = msg,
            4 Password(PasswordOAuthFlow) = msg,
            5 DeviceCode(DeviceCodeOAuthFlow) = msg,
        }
    }
}

message! {
    /// `AuthorizationCodeOAuthFlow`.
    AuthorizationCodeOAuthFlow {
        1 authorization_url: String = string,
        2 token_url: String = string,
        3 refresh_url: String = string,
        4 scopes: BTreeMap<String, String> = map_str,
        5 pkce_required: bool = bool,
    }
}

message! {
    /// `ClientCredentialsOAuthFlow`.
    ClientCredentialsOAuthFlow {
        1 token_url: String = string,
        2 refresh_url: String = string,
        3 scopes: BTreeMap<String, String> = map_str,
    }
}

message! {
    /// `ImplicitOAuthFlow`.
    ImplicitOAuthFlow {
        1 authorization_url: String = string,
        2 refresh_url: String = string,
        3 scopes: BTreeMap<String, String> = map_str,
    }
}

message! {
    /// `PasswordOAuthFlow`.
    PasswordOAuthFlow {
        1 token_url: String = string,
        2 refresh_url: String = string,
        3 scopes: BTreeMap<String, String> = map_str,
    }
}

message! {
    /// `DeviceCodeOAuthFlow`.
    DeviceCodeOAuthFlow {
        1 device_authorization_url: String = string,
        2 token_url: String = string,
        3 refresh_url: String = string,
        4 scopes: BTreeMap<String, String> = map_str,
    }
}

// ---- requests and responses -----------------------------------------------------------------

message! {
    /// `SendMessageRequest`.
    SendMessageRequest {
        1 tenant: String = string,
        2 message: Option<Message> = msg,
        3 configuration: Option<SendMessageConfiguration> = msg,
        4 metadata: Option<Struct> = msg,
    }
}

message! {
    /// `GetTaskRequest`.
    GetTaskRequest {
        1 tenant: String = string,
        2 id: String = string,
        3 history_length: Option<i32> = opt_int32,
    }
}

message! {
    /// `ListTasksRequest`.
    ListTasksRequest {
        1 tenant: String = string,
        2 context_id: String = string,
        3 status: i32 = enumv,
        4 page_size: Option<i32> = opt_int32,
        5 page_token: String = string,
        6 history_length: Option<i32> = opt_int32,
        7 status_timestamp_after: Option<Timestamp> = msg,
        8 include_artifacts: Option<bool> = opt_bool,
    }
}

message! {
    /// `ListTasksResponse`.
    ListTasksResponse {
        1 tasks: Vec<Task> = rep_msg,
        2 next_page_token: String = string,
        3 page_size: i32 = int32,
        4 total_size: i32 = int32,
    }
}

message! {
    /// `CancelTaskRequest`.
    CancelTaskRequest {
        1 tenant: String = string,
        2 id: String = string,
        3 metadata: Option<Struct> = msg,
    }
}

message! {
    /// `GetTaskPushNotificationConfigRequest`.
    GetTaskPushNotificationConfigRequest {
        1 tenant: String = string,
        2 task_id: String = string,
        3 id: String = string,
    }
}

message! {
    /// `DeleteTaskPushNotificationConfigRequest`.
    DeleteTaskPushNotificationConfigRequest {
        1 tenant: String = string,
        2 task_id: String = string,
        3 id: String = string,
    }
}

message! {
    /// `SubscribeToTaskRequest`.
    SubscribeToTaskRequest {
        1 tenant: String = string,
        2 id: String = string,
    }
}

message! {
    /// `ListTaskPushNotificationConfigsRequest`.
    ListTaskPushNotificationConfigsRequest {
        1 task_id: String = string,
        2 page_size: i32 = int32,
        3 page_token: String = string,
        4 tenant: String = string,
    }
}

message! {
    /// `GetExtendedAgentCardRequest`.
    GetExtendedAgentCardRequest {
        1 tenant: String = string,
    }
}

message! {
    /// `SendMessageResponse`.
    SendMessageResponse {
        oneof payload: send_message_response::Payload;
    }
}

/// Nested types of [`SendMessageResponse`].
pub mod send_message_response {
    use super::*;

    oneof! {
        /// `SendMessageResponse.payload`.
        Payload {
            1 Task(Task) = msg,
            2 Message(Message) = msg,
        }
    }
}

message! {
    /// `StreamResponse`.
    StreamResponse {
        oneof payload: stream_response::Payload;
    }
}

/// Nested types of [`StreamResponse`].
pub mod stream_response {
    use super::*;

    oneof! {
        /// `StreamResponse.payload`.
        Payload {
            1 Task(Task) = msg,
            2 Message(Message) = msg,
            3 StatusUpdate(TaskStatusUpdateEvent) = msg,
            4 ArtifactUpdate(TaskArtifactUpdateEvent) = msg,
        }
    }
}

message! {
    /// `ListTaskPushNotificationConfigsResponse`.
    ListTaskPushNotificationConfigsResponse {
        1 configs: Vec<TaskPushNotificationConfig> = rep_msg,
        2 next_page_token: String = string,
    }
}
