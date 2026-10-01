//! Mapping between ADK's tool shapes and MCP's, independent of transport.
//!
//! The wire protocol itself (JSON-RPC framing, the `initialize` handshake,
//! version negotiation) is `rmcp`'s, the workspace's shared MCP stack. What
//! stays here is the translation only ADK needs: schema type casing and the
//! result envelope.

use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, ProtocolVersion, Tool};
use serde_json::{json, Map, Value};

/// The MCP revision this crate speaks, as the server and as the client.
///
/// Other-language ADK SDKs speak it, and it is the newest revision whose
/// semantics this crate is tested against (`tests/conformance.rs`).
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// [`PROTOCOL_VERSION`] as `rmcp`'s type.
pub(crate) const fn protocol_version() -> ProtocolVersion {
    ProtocolVersion::V_2025_06_18
}

/// The revisions the server accepts: the older ones a client may ask for,
/// up to [`PROTOCOL_VERSION`]. A client asking for anything newer is
/// answered with [`PROTOCOL_VERSION`].
pub(crate) const SUPPORTED_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2024_11_05,
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2025_06_18,
];

/// Renders a tool declaration as an MCP tool.
///
/// MCP uses lower-case JSON Schema type names, whereas ADK's `Schema`
/// serializes the upper-case `google.genai` spelling, so the types are folded
/// on the way out.
pub(crate) fn tool_entry(declaration: &adk_core::FunctionDeclaration) -> Tool {
    let schema = match &declaration.parameters {
        Some(params) => {
            lowercase_types(&serde_json::to_value(params).unwrap_or_else(|_| json!({})))
        }
        None => json!({"type": "object", "properties": {}}),
    };
    let schema = match schema {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    Tool::new(
        declaration.name.clone(),
        declaration.description.clone(),
        Arc::new(schema),
    )
}

/// Recursively lower-cases `type` values in a JSON Schema document.
pub(crate) fn lowercase_types(value: &Value) -> Value {
    recase_types(value, str::to_lowercase)
}

/// Recursively upper-cases `type` values, restoring ADK's spelling.
pub(crate) fn uppercase_types(value: &Value) -> Value {
    recase_types(value, str::to_uppercase)
}

fn recase_types(value: &Value, recase: fn(&str) -> String) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, val)| match (key.as_str(), val.as_str()) {
                    ("type", Some(name)) => (key.clone(), json!(recase(name))),
                    _ => (key.clone(), recase_types(val, recase)),
                })
                .collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| recase_types(v, recase)).collect())
        }
        other => other.clone(),
    }
}

/// Wraps a tool result in the MCP content envelope.
///
/// MCP carries results as content blocks; ADK tools return JSON, so the value
/// is serialized into a single text block. The error flag is set from ADK's
/// `status` convention so an MCP client sees a failure as a failure.
pub(crate) fn tool_result(value: &Value) -> CallToolResult {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    let content = vec![ContentBlock::text(text)];
    if value.get("status").and_then(Value::as_str) == Some("error") {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{FunctionDeclaration, Schema};

    #[test]
    fn schema_types_are_lower_cased_for_mcp() {
        let declaration = FunctionDeclaration::new("get_weather", "Gets weather.")
            .with_parameters(Schema::object().property("city", Schema::string()));
        let tool = tool_entry(&declaration);
        assert_eq!(tool.input_schema["type"], "object");
        assert_eq!(tool.input_schema["properties"]["city"]["type"], "string");
        assert_eq!(tool.name, "get_weather");
    }

    #[test]
    fn a_tool_without_parameters_still_declares_an_object_schema() {
        let tool = tool_entry(&FunctionDeclaration::new("ping", "Pings."));
        assert_eq!(tool.input_schema["type"], "object");
    }

    #[test]
    fn an_error_status_sets_the_mcp_error_flag() {
        let ok = tool_result(&json!({"status": "success", "v": 1}));
        assert_eq!(ok.is_error, Some(false));
        let bad = tool_result(&json!({"status": "error", "error_message": "nope"}));
        assert_eq!(bad.is_error, Some(true));
    }

    #[test]
    fn schema_types_round_trip() {
        let adk = json!({"type": "OBJECT", "properties": {"a": {"type": "STRING"}}});
        assert_eq!(uppercase_types(&lowercase_types(&adk)), adk);
    }
}
