//! The JSON-RPC 2.0 envelope used by the JSON-RPC protocol binding (spec
//! Section 9). Method names are PascalCase, matching gRPC service method
//! names (Section 9.1).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::A2aError;

/// A JSON-RPC 2.0 request id: either a string or a number, per spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    String(String),
    Number(i64),
}

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        RequestId::String(s.to_string())
    }
}

impl From<i64> for RequestId {
    fn from(n: i64) -> Self {
        RequestId::Number(n)
    }
}

/// The PascalCase JSON-RPC method names defined in spec Section 9.4 /
/// Section 5.3 (Method Mapping Reference).
pub mod methods {
    pub const SEND_MESSAGE: &str = "SendMessage";
    pub const SEND_STREAMING_MESSAGE: &str = "SendStreamingMessage";
    pub const GET_TASK: &str = "GetTask";
    pub const LIST_TASKS: &str = "ListTasks";
    pub const CANCEL_TASK: &str = "CancelTask";
    pub const SUBSCRIBE_TO_TASK: &str = "SubscribeToTask";
    pub const CREATE_TASK_PUSH_NOTIFICATION_CONFIG: &str = "CreateTaskPushNotificationConfig";
    pub const GET_TASK_PUSH_NOTIFICATION_CONFIG: &str = "GetTaskPushNotificationConfig";
    pub const LIST_TASK_PUSH_NOTIFICATION_CONFIGS: &str = "ListTaskPushNotificationConfigs";
    pub const DELETE_TASK_PUSH_NOTIFICATION_CONFIG: &str = "DeleteTaskPushNotificationConfig";
    pub const GET_EXTENDED_AGENT_CARD: &str = "GetExtendedAgentCard";
}

/// A JSON-RPC 2.0 request envelope (spec Section 9.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: JsonRpcVersion,
    pub id: RequestId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: impl Into<RequestId>, method: impl Into<String>, params: Value) -> Self {
        JsonRpcRequest {
            jsonrpc: JsonRpcVersion,
            id: id.into(),
            method: method.into(),
            params: Some(params),
        }
    }
}

/// The literal string `"2.0"`, enforced at the type level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonRpcVersion;

impl Serialize for JsonRpcVersion {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for JsonRpcVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        if s == "2.0" {
            Ok(JsonRpcVersion)
        } else {
            Err(serde::de::Error::custom(format!(
                "unsupported jsonrpc version: {s}"
            )))
        }
    }
}

/// A JSON-RPC 2.0 error object (spec Section 9.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub data: Vec<Value>,
}

impl From<&A2aError> for JsonRpcErrorObject {
    fn from(err: &A2aError) -> Self {
        let mut data = Vec::new();
        if let Some(reason) = err.reason() {
            data.push(serde_json::json!({
                "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                "reason": reason,
                "domain": "a2a-protocol.org",
            }));
        }
        JsonRpcErrorObject {
            code: err.json_rpc_code(),
            message: err.standard_message(),
            data,
        }
    }
}

/// A JSON-RPC 2.0 response envelope: either `result` or `error` is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: JsonRpcVersion,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<JsonRpcErrorObject>,
}

impl JsonRpcResponse {
    pub fn success(id: RequestId, result: Value) -> Self {
        JsonRpcResponse {
            jsonrpc: JsonRpcVersion,
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: RequestId, err: &A2aError) -> Self {
        JsonRpcResponse {
            jsonrpc: JsonRpcVersion,
            id,
            result: None,
            error: Some(err.into()),
        }
    }

    /// Converts this response into a `Result`, mapping a JSON-RPC error
    /// object back to an [`A2aError`] on a best-effort basis (matching by
    /// numeric code).
    pub fn into_result(self) -> Result<Value, A2aError> {
        if let Some(err) = self.error {
            Err(jsonrpc_error_to_a2a(err))
        } else {
            Ok(self.result.unwrap_or(Value::Null))
        }
    }
}

/// Best-effort mapping from a JSON-RPC error object back to an
/// [`A2aError`], for clients consuming another implementation's error
/// responses. Falls back to [`A2aError::Internal`] for unrecognized codes.
pub fn jsonrpc_error_to_a2a(err: JsonRpcErrorObject) -> A2aError {
    match err.code {
        -32001 => A2aError::TaskNotFound(err.message),
        -32002 => A2aError::TaskNotCancelable(err.message),
        -32003 => A2aError::PushNotificationNotSupported,
        -32004 => A2aError::UnsupportedOperation(err.message),
        -32005 => A2aError::ContentTypeNotSupported(err.message),
        -32006 => A2aError::InvalidAgentResponse(err.message),
        -32007 => A2aError::ExtendedAgentCardNotConfigured,
        -32008 => A2aError::ExtensionSupportRequired(err.message),
        -32009 => A2aError::VersionNotSupported(err.message),
        -32010 => A2aError::Unauthenticated(err.message),
        -32011 => A2aError::PermissionDenied(err.message),
        -32700 => A2aError::ParseError,
        -32600 => A2aError::InvalidRequest(err.message),
        -32601 => A2aError::MethodNotFound(err.message),
        -32602 => A2aError::InvalidParams(err.message),
        _ => A2aError::Internal(err.message),
    }
}
