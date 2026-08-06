//! The A2A error model (spec Section 3.3.2) and its mapping onto the
//! JSON-RPC and HTTP protocol bindings (spec Section 5.4).

use serde::{Deserialize, Serialize};

/// A protocol-level A2A error.
///
/// Variants prefixed with the exact names used by the specification
/// (`TaskNotFound`, `TaskNotCancelable`, ...) are the nine A2A-specific
/// error types defined in Section 3.3.2. The remaining variants cover the
/// generic error categories (`Authentication`, `Authorization`,
/// `InvalidParams`, ...) and the standard JSON-RPC 2.0 errors, all of which
/// every A2A protocol binding must also be able to represent.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "type", content = "message")]
pub enum A2aError {
    /// `TaskNotFoundError` - the task id does not correspond to an existing
    /// or accessible task.
    #[error("task not found: {0}")]
    TaskNotFound(String),

    /// `TaskNotCancelableError` - the task is already in a terminal state.
    #[error("task not cancelable: {0}")]
    TaskNotCancelable(String),

    /// `PushNotificationNotSupportedError` - the agent does not support
    /// push notifications (`AgentCard.capabilities.pushNotifications`).
    #[error("push notifications are not supported by this agent")]
    PushNotificationNotSupported,

    /// `UnsupportedOperationError` - the requested operation, or some
    /// aspect of it, is not supported by this agent.
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    /// `ContentTypeNotSupportedError` - a media type in the request is not
    /// supported by the agent or the invoked skill.
    #[error("content type not supported: {0}")]
    ContentTypeNotSupported(String),

    /// `InvalidAgentResponseError` - an agent produced a response that does
    /// not conform to the specification for the current method.
    #[error("invalid agent response: {0}")]
    InvalidAgentResponse(String),

    /// `ExtendedAgentCardNotConfiguredError` - the agent declares support
    /// for an extended card but has none configured.
    #[error("extended agent card not configured")]
    ExtendedAgentCardNotConfigured,

    /// `ExtensionSupportRequiredError` - the server requires an extension
    /// the client did not declare support for.
    #[error("extension support required: {0}")]
    ExtensionSupportRequired(String),

    /// `VersionNotSupportedError` - the requested `A2A-Version` is not
    /// supported by this interface.
    #[error("A2A protocol version not supported: {0}")]
    VersionNotSupported(String),

    /// Authentication credentials were missing or invalid.
    #[error("authentication required: {0}")]
    Unauthenticated(String),

    /// The authenticated caller lacks permission for the requested
    /// operation.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// The request could not be parsed as valid JSON.
    #[error("invalid JSON payload")]
    ParseError,

    /// The request was not a valid JSON-RPC / A2A request object.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// The requested method does not exist or is not available.
    #[error("method not found: {0}")]
    MethodNotFound(String),

    /// Request parameters failed validation.
    #[error("invalid parameters: {0}")]
    InvalidParams(String),

    /// An unexpected internal error occurred while processing the request.
    #[error("internal error: {0}")]
    Internal(String),
}

impl A2aError {
    /// The JSON-RPC 2.0 numeric error code for this error, per spec
    /// Section 5.4 for A2A-specific errors and Section 9.5 for the
    /// standard JSON-RPC codes. Codes for the generic `Unauthenticated`
    /// and `PermissionDenied` categories are not standardized by the spec
    /// ("JSON-RPC custom error"); this crate assigns them adjacent, unused
    /// slots in the A2A reserved range (`-32001`..`-32099`).
    pub fn json_rpc_code(&self) -> i64 {
        match self {
            A2aError::TaskNotFound(_) => -32001,
            A2aError::TaskNotCancelable(_) => -32002,
            A2aError::PushNotificationNotSupported => -32003,
            A2aError::UnsupportedOperation(_) => -32004,
            A2aError::ContentTypeNotSupported(_) => -32005,
            A2aError::InvalidAgentResponse(_) => -32006,
            A2aError::ExtendedAgentCardNotConfigured => -32007,
            A2aError::ExtensionSupportRequired(_) => -32008,
            A2aError::VersionNotSupported(_) => -32009,
            A2aError::Unauthenticated(_) => -32010,
            A2aError::PermissionDenied(_) => -32011,
            A2aError::ParseError => -32700,
            A2aError::InvalidRequest(_) => -32600,
            A2aError::MethodNotFound(_) => -32601,
            A2aError::InvalidParams(_) => -32602,
            A2aError::Internal(_) => -32603,
        }
    }

    /// The HTTP status code this error maps to per spec Section 5.4.
    pub fn http_status(&self) -> u16 {
        match self {
            A2aError::TaskNotFound(_) => 404,
            A2aError::TaskNotCancelable(_) => 400,
            A2aError::PushNotificationNotSupported => 400,
            A2aError::UnsupportedOperation(_) => 400,
            A2aError::ContentTypeNotSupported(_) => 400,
            A2aError::InvalidAgentResponse(_) => 500,
            A2aError::ExtendedAgentCardNotConfigured => 400,
            A2aError::ExtensionSupportRequired(_) => 400,
            A2aError::VersionNotSupported(_) => 400,
            A2aError::Unauthenticated(_) => 401,
            A2aError::PermissionDenied(_) => 403,
            A2aError::ParseError => 400,
            A2aError::InvalidRequest(_) => 400,
            A2aError::MethodNotFound(_) => 404,
            A2aError::InvalidParams(_) => 400,
            A2aError::Internal(_) => 500,
        }
    }

    /// The `google.rpc.ErrorInfo.reason` value for this error: the A2A
    /// error type name in `UPPER_SNAKE_CASE` with the `Error` suffix
    /// dropped, e.g. `TaskNotFoundError` -> `TASK_NOT_FOUND` (spec Section
    /// 11.6). Returns `None` for the generic, non-A2A-specific variants.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            A2aError::TaskNotFound(_) => Some("TASK_NOT_FOUND"),
            A2aError::TaskNotCancelable(_) => Some("TASK_NOT_CANCELABLE"),
            A2aError::PushNotificationNotSupported => Some("PUSH_NOTIFICATION_NOT_SUPPORTED"),
            A2aError::UnsupportedOperation(_) => Some("UNSUPPORTED_OPERATION"),
            A2aError::ContentTypeNotSupported(_) => Some("CONTENT_TYPE_NOT_SUPPORTED"),
            A2aError::InvalidAgentResponse(_) => Some("INVALID_AGENT_RESPONSE"),
            A2aError::ExtendedAgentCardNotConfigured => Some("EXTENDED_AGENT_CARD_NOT_CONFIGURED"),
            A2aError::ExtensionSupportRequired(_) => Some("EXTENSION_SUPPORT_REQUIRED"),
            A2aError::VersionNotSupported(_) => Some("VERSION_NOT_SUPPORTED"),
            _ => None,
        }
    }

    /// The message sent over the wire in the JSON-RPC/HTTP error object.
    ///
    /// For the five standard JSON-RPC codes (`-32700`..`-32603`) this is
    /// the fixed string from the spec's error table (Section 9.5). For the
    /// nine A2A-specific errors (and this crate's `Unauthenticated` /
    /// `PermissionDenied` extensions), the spec doesn't mandate a fixed
    /// string, so the message *is* the variant's own detail string
    /// verbatim (rather than the longer `Display` sentence) - this lets
    /// [`jsonrpc_error_to_a2a`](crate::types::jsonrpc::jsonrpc_error_to_a2a)
    /// reconstruct an equivalent error client-side instead of embedding
    /// `Display`'s own error-name prefix into the reconstructed detail.
    pub fn standard_message(&self) -> String {
        match self {
            A2aError::ParseError => "Invalid JSON payload".to_string(),
            A2aError::InvalidRequest(_) => "Request payload validation error".to_string(),
            A2aError::MethodNotFound(_) => "Method not found".to_string(),
            A2aError::InvalidParams(_) => "Invalid parameters".to_string(),
            A2aError::Internal(_) => "Internal error".to_string(),
            A2aError::TaskNotFound(detail)
            | A2aError::TaskNotCancelable(detail)
            | A2aError::UnsupportedOperation(detail)
            | A2aError::ContentTypeNotSupported(detail)
            | A2aError::InvalidAgentResponse(detail)
            | A2aError::ExtensionSupportRequired(detail)
            | A2aError::VersionNotSupported(detail)
            | A2aError::Unauthenticated(detail)
            | A2aError::PermissionDenied(detail) => detail.clone(),
            A2aError::PushNotificationNotSupported | A2aError::ExtendedAgentCardNotConfigured => {
                self.to_string()
            }
        }
    }
}

pub type Result<T> = std::result::Result<T, A2aError>;
