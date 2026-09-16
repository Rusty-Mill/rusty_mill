//! JSON-RPC 2.0 envelopes and the MCP method handling, independent of transport.
//!
//! Keeping the protocol here means the stdio and HTTP transports share one
//! implementation, and the whole surface is testable without any I/O.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The MCP revision this implementation speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// JSON-RPC error code for a method the server does not implement.
pub const METHOD_NOT_FOUND: i32 = -32601;
/// JSON-RPC error code for malformed parameters.
pub const INVALID_PARAMS: i32 = -32602;
/// JSON-RPC error code for an unexpected server-side failure.
pub const INTERNAL_ERROR: i32 = -32603;
/// JSON-RPC error code for a body that is not valid JSON.
pub const PARSE_ERROR: i32 = -32700;

/// An incoming JSON-RPC request or notification.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    /// Always `"2.0"`.
    #[serde(default)]
    pub jsonrpc: String,
    /// Absent on a notification, which expects no response.
    #[serde(default)]
    pub id: Option<Value>,
    /// The method being invoked.
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    /// True when this is a notification, which must not be answered.
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// A JSON-RPC error payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Machine-readable error code.
    pub code: i32,
    /// Human-readable description.
    pub message: String,
    /// Optional structured detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// An outgoing JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Echoes the request's id.
    pub id: Value,
    /// The result, on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The error, on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    /// Builds a success response.
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Builds an error response.
    pub fn error(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// Renders a tool declaration as an MCP tool entry.
///
/// MCP uses lower-case JSON Schema type names, whereas ADK's `Schema`
/// serializes the upper-case `google.genai` spelling, so the types are folded
/// on the way out.
pub fn tool_entry(declaration: &adk_core::FunctionDeclaration) -> Value {
    let schema = match &declaration.parameters {
        Some(params) => {
            lowercase_types(&serde_json::to_value(params).unwrap_or_else(|_| json!({})))
        }
        None => json!({"type": "object", "properties": {}}),
    };
    json!({
        "name": declaration.name,
        "description": declaration.description,
        "inputSchema": schema,
    })
}

/// Recursively lower-cases `type` values in a JSON Schema document.
pub fn lowercase_types(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, val)| {
                    if key == "type" {
                        if let Some(name) = val.as_str() {
                            return (key.clone(), json!(name.to_lowercase()));
                        }
                    }
                    (key.clone(), lowercase_types(val))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(lowercase_types).collect()),
        other => other.clone(),
    }
}

/// Wraps a tool result in the MCP content envelope.
///
/// MCP carries results as content blocks; ADK tools return JSON, so the value
/// is serialized into a single text block. `is_error` is set from ADK's
/// `status` convention so an MCP client sees a failure as a failure.
pub fn tool_result(value: &Value) -> Value {
    let is_error = value.get("status").and_then(Value::as_str) == Some("error");
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string(value).unwrap_or_else(|_| "null".into()),
        }],
        "isError": is_error,
    })
}

/// The server's `initialize` result.
pub fn initialize_result(server_name: &str, server_version: &str) -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": server_name, "version": server_version},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{FunctionDeclaration, Schema};

    #[test]
    fn a_request_without_an_id_is_a_notification() {
        let req: JsonRpcRequest =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .unwrap();
        assert!(req.is_notification());
    }

    #[test]
    fn schema_types_are_lower_cased_for_mcp() {
        let declaration = FunctionDeclaration::new("get_weather", "Gets weather.")
            .with_parameters(Schema::object().property("city", Schema::string()));
        let entry = tool_entry(&declaration);
        assert_eq!(entry["inputSchema"]["type"], "object");
        assert_eq!(entry["inputSchema"]["properties"]["city"]["type"], "string");
        assert_eq!(entry["name"], "get_weather");
    }

    #[test]
    fn a_tool_without_parameters_still_declares_an_object_schema() {
        let entry = tool_entry(&FunctionDeclaration::new("ping", "Pings."));
        assert_eq!(entry["inputSchema"]["type"], "object");
    }

    #[test]
    fn an_error_status_sets_the_mcp_error_flag() {
        let ok = tool_result(&json!({"status": "success", "v": 1}));
        assert_eq!(ok["isError"], false);
        let bad = tool_result(&json!({"status": "error", "error_message": "nope"}));
        assert_eq!(bad["isError"], true);
    }

    #[test]
    fn a_result_is_carried_as_a_text_content_block() {
        let result = tool_result(&json!({"status": "success", "temp": 20}));
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["temp"], 20);
    }

    #[test]
    fn responses_serialize_without_the_unused_half() {
        let ok = serde_json::to_value(JsonRpcResponse::success(json!(1), json!({}))).unwrap();
        assert!(ok.get("error").is_none());
        let err = serde_json::to_value(JsonRpcResponse::error(json!(1), METHOD_NOT_FOUND, "nope"))
            .unwrap();
        assert!(err.get("result").is_none());
        assert_eq!(err["error"]["code"], METHOD_NOT_FOUND);
    }
}
