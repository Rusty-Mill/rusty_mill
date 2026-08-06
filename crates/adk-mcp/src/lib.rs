//! Model Context Protocol transports for the Rust ADK.
//!
//! ADK has no language-neutral wire protocol for tools, but every ADK SDK can
//! consume an **MCP** server. That makes MCP the interoperability path: serve
//! Rust tools with [`McpServer`] and an ADK agent in Python, Go, TypeScript,
//! Java, or Kotlin can call them; consume someone else's server with
//! [`McpToolset`] and a Rust agent gains their tools.
//!
//! # Serving Rust tools
//!
//! ```no_run
//! # use adk_core::{Schema, Services};
//! # use adk_mcp::{serve_stdio, McpServer};
//! # use adk_sessions::InMemorySessionService;
//! # use adk_tools::FunctionTool;
//! # use std::sync::Arc;
//! # #[tokio::main]
//! # async fn main() -> adk_core::Result<()> {
//! let weather = FunctionTool::new(
//!     "get_weather",
//!     "Retrieves the current weather for a city.",
//!     Schema::object().property("city", Schema::string()),
//!     |args, _ctx| {
//!         Box::pin(async move {
//!             let city = args.get("city").and_then(|v| v.as_str()).unwrap_or("?");
//!             Ok(adk_tools::success(serde_json::json!({ "report": format!("Sunny in {city}") })))
//!         })
//!     },
//! );
//!
//! let services = Services::new(Arc::new(InMemorySessionService::new()));
//! let server = McpServer::new("rust-weather", vec![weather.shared()], services);
//! serve_stdio(&server).await
//! # }
//! ```
//!
//! A Python ADK agent then reaches it with the standard `McpToolset`:
//!
//! ```python
//! McpToolset(connection_params=StdioConnectionParams(
//!     server_params=StdioServerParameters(command="./rust-weather-server", args=[]),
//! ))
//! ```

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod protocol;
pub mod server;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "http")]
pub mod http;
#[cfg(feature = "stdio")]
pub mod stdio;

pub use protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, PROTOCOL_VERSION};
pub use server::{serve_tools, McpServer};

#[cfg(feature = "client")]
pub use client::{BoundMcpToolset, ConnectionParams, McpToolset};
#[cfg(feature = "http")]
pub use http::{router, serve_http};
#[cfg(feature = "stdio")]
pub use stdio::{serve_stdio, serve_stream};

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{Schema, Services};
    use adk_sessions::InMemorySessionService;
    use adk_tools::{FunctionTool, SharedTool};
    use serde_json::{json, Value};
    use std::sync::Arc;

    fn test_server() -> McpServer {
        let weather = FunctionTool::new(
            "get_weather",
            "Retrieves the current weather for a city.",
            Schema::object().property("city", Schema::string().describe("The city name.")),
            |args, _ctx| {
                Box::pin(async move {
                    let city = args.get("city").and_then(Value::as_str).unwrap_or("?");
                    Ok(adk_tools::success(
                        json!({"report": format!("Sunny in {city}")}),
                    ))
                })
            },
        );
        let failing = FunctionTool::new("boom", "Always fails.", Schema::object(), |_a, _c| {
            Box::pin(async { Err(adk_core::AdkError::tool("boom", "exploded")) })
        });

        let tools: Vec<SharedTool> = vec![weather.shared(), failing.shared()];
        McpServer::new(
            "test-server",
            tools,
            Services::new(Arc::new(InMemorySessionService::new())),
        )
    }

    async fn call(server: &McpServer, body: Value) -> Value {
        let text = server
            .handle_raw(&body.to_string())
            .await
            .expect("expected a response");
        serde_json::from_str(&text).unwrap()
    }

    #[tokio::test]
    async fn initialize_reports_tool_capability_and_protocol_version() {
        let response = call(
            &test_server(),
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
        )
        .await;
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert_eq!(response["result"]["serverInfo"]["name"], "test-server");
    }

    #[tokio::test]
    async fn tools_list_returns_declarations_with_lowercase_schema_types() {
        let response = call(
            &test_server(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .await;
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);

        let weather = tools.iter().find(|t| t["name"] == "get_weather").unwrap();
        assert_eq!(weather["inputSchema"]["type"], "object");
        assert_eq!(
            weather["inputSchema"]["properties"]["city"]["type"],
            "string"
        );
        assert_eq!(weather["inputSchema"]["required"], json!(["city"]));
    }

    #[tokio::test]
    async fn tools_call_runs_the_tool_and_wraps_the_result() {
        let response = call(
            &test_server(),
            json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "get_weather", "arguments": {"city": "Paris"}},
            }),
        )
        .await;

        assert_eq!(response["result"]["isError"], false);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["status"], "success");
        assert_eq!(payload["report"], "Sunny in Paris");
    }

    #[tokio::test]
    async fn a_failing_tool_is_a_tool_error_not_a_protocol_error() {
        // MCP distinguishes "the tool ran and failed" from "the request was
        // malformed"; clients rely on that difference.
        let response = call(
            &test_server(),
            json!({
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {"name": "boom", "arguments": {}},
            }),
        )
        .await;
        assert!(response.get("error").is_none());
        assert_eq!(response["result"]["isError"], true);
    }

    #[tokio::test]
    async fn an_unknown_tool_is_a_protocol_error() {
        let response = call(
            &test_server(),
            json!({
                "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": {"name": "ghost", "arguments": {}},
            }),
        )
        .await;
        assert_eq!(response["error"]["code"], protocol::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn a_call_with_invalid_arguments_is_rejected_before_the_tool_runs() {
        let response = call(
            &test_server(),
            json!({
                "jsonrpc": "2.0", "id": 6, "method": "tools/call",
                "params": {"name": "get_weather", "arguments": {}},
            }),
        )
        .await;
        // The schema check turns this into a tool error carrying the reason.
        assert_eq!(response["result"]["isError"], true);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("city"), "got: {text}");
    }

    #[tokio::test]
    async fn a_notification_is_not_answered() {
        let server = test_server();
        let response = server
            .handle_raw(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .await;
        assert!(response.is_none());
    }

    #[tokio::test]
    async fn an_unknown_method_is_reported() {
        let response = call(
            &test_server(),
            json!({"jsonrpc": "2.0", "id": 7, "method": "resources/list"}),
        )
        .await;
        assert_eq!(response["error"]["code"], protocol::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn malformed_json_produces_a_parse_error() {
        let server = test_server();
        let text = server.handle_raw("{not json").await.unwrap();
        let response: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(response["error"]["code"], protocol::PARSE_ERROR);
    }

    #[cfg(feature = "stdio")]
    #[tokio::test]
    async fn the_stdio_transport_answers_line_delimited_requests() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
        );
        let mut output: Vec<u8> = Vec::new();
        serve_stream(&test_server(), input.as_bytes(), &mut output)
            .await
            .unwrap();

        let text = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();

        // Two requests, one notification: exactly two responses.
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first["id"], 1);
        assert_eq!(second["id"], 2);
        assert_eq!(second["result"]["tools"].as_array().unwrap().len(), 2);
    }
}
