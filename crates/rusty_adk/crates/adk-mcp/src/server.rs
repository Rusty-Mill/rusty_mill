//! The MCP server: exposes ADK tools to any MCP client.
//!
//! [`McpServer::handle`] is transport-independent — it takes a JSON-RPC request
//! and returns the response — so the stdio and HTTP transports are thin
//! wrappers around it, and the whole protocol is testable without I/O.

use adk_core::{InvocationContext, RunConfig, Services, Session};
use adk_tools::{invoke_tool, SharedTool, ToolContext};
use serde_json::{json, Map, Value};
use std::sync::Arc;

use crate::protocol::{
    initialize_result, tool_entry, tool_result, JsonRpcRequest, JsonRpcResponse, INTERNAL_ERROR,
    INVALID_PARAMS, METHOD_NOT_FOUND,
};

/// Serves a set of ADK tools over the Model Context Protocol.
pub struct McpServer {
    name: String,
    version: String,
    tools: Vec<SharedTool>,
    services: Services,
    app_name: String,
}

impl McpServer {
    /// Builds a server exposing `tools`.
    ///
    /// The session service backs the [`ToolContext`] each call runs against, so
    /// tools that read or write state work the same as they do inside an agent.
    pub fn new(name: impl Into<String>, tools: Vec<SharedTool>, services: Services) -> Self {
        Self {
            name: name.into(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            tools,
            services,
            app_name: "mcp".to_string(),
        }
    }

    /// Sets the version reported during `initialize`.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Sets the app name used for the sessions tool calls run against.
    pub fn with_app_name(mut self, app_name: impl Into<String>) -> Self {
        self.app_name = app_name.into();
        self
    }

    /// The server's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The tools this server exposes.
    pub fn tools(&self) -> &[SharedTool] {
        &self.tools
    }

    /// Handles one JSON-RPC request.
    ///
    /// Returns `None` for a notification, which the protocol says must not be
    /// answered.
    pub async fn handle(&self, request: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = request.id.clone();
        let is_notification = request.is_notification();

        let outcome = match request.method.as_str() {
            "initialize" => Ok(initialize_result(&self.name, &self.version)),
            "notifications/initialized" | "notifications/cancelled" => Ok(Value::Null),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": self
                    .tools
                    .iter()
                    .filter_map(|t| t.declaration())
                    .map(|d| tool_entry(&d))
                    .collect::<Vec<_>>(),
            })),
            "tools/call" => self.call_tool(request.params).await,
            other => Err((METHOD_NOT_FOUND, format!("unsupported method '{other}'"))),
        };

        if is_notification {
            return None;
        }

        let id = id.unwrap_or(Value::Null);
        Some(match outcome {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err((code, message)) => JsonRpcResponse::error(id, code, message),
        })
    }

    /// Handles `tools/call`.
    async fn call_tool(&self, params: Option<Value>) -> Result<Value, (i32, String)> {
        let params = params.unwrap_or(Value::Null);
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| (INVALID_PARAMS, "missing tool name".to_string()))?;

        let args: Map<String, Value> = match params.get("arguments") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(map)) => map.clone(),
            Some(_) => {
                return Err((INVALID_PARAMS, "arguments must be an object".to_string()));
            }
        };

        let tool = self
            .tools
            .iter()
            .find(|t| t.name() == name)
            .ok_or_else(|| (METHOD_NOT_FOUND, format!("unknown tool '{name}'")))?;

        let session = Session::new(adk_core::new_id("mcp"), &self.app_name, "mcp-client");
        let ctx = ToolContext::new(InvocationContext::new(
            session,
            self.services.clone(),
            RunConfig::default(),
        ));

        // A tool failure is reported as an MCP tool error, not a JSON-RPC
        // error: the protocol distinguishes "the tool ran and failed" from
        // "the request was malformed", and clients rely on that difference.
        match invoke_tool(tool.as_ref(), args, &ctx).await {
            Ok(value) => Ok(tool_result(&value)),
            Err(err) => Ok(tool_result(&adk_tools::error(err.to_string()))),
        }
    }

    /// Handles a raw JSON body, for transports that deal in bytes.
    ///
    /// Returns the serialized response, or `None` for a notification.
    pub async fn handle_raw(&self, body: &str) -> Option<String> {
        let request: JsonRpcRequest = match serde_json::from_str(body) {
            Ok(request) => request,
            Err(err) => {
                let response = JsonRpcResponse::error(
                    Value::Null,
                    crate::protocol::PARSE_ERROR,
                    format!("invalid JSON-RPC request: {err}"),
                );
                return serde_json::to_string(&response).ok();
            }
        };

        let response = self.handle(request).await?;
        match serde_json::to_string(&response) {
            Ok(text) => Some(text),
            Err(err) => {
                let fallback = JsonRpcResponse::error(
                    Value::Null,
                    INTERNAL_ERROR,
                    format!("failed to encode response: {err}"),
                );
                serde_json::to_string(&fallback).ok()
            }
        }
    }
}

/// Builds a server over a session service with no artifact or memory backend.
pub fn serve_tools(name: impl Into<String>, tools: Vec<SharedTool>) -> McpServer {
    let services = Services::new(Arc::new(adk_core_in_memory()));
    McpServer::new(name, tools, services)
}

/// A minimal in-memory session service, so the crate can build a server without
/// depending on `adk-sessions`.
fn adk_core_in_memory() -> impl adk_core::SessionService {
    EphemeralSessionService
}

/// Sessions that live only as long as the call that created them.
///
/// An MCP server has no conversation of its own: each `tools/call` is
/// independent. Tools that need durable state should be served with a real
/// session service via [`McpServer::new`].
struct EphemeralSessionService;

#[async_trait::async_trait]
impl adk_core::SessionService for EphemeralSessionService {
    async fn create_session(
        &self,
        app_name: &str,
        user_id: &str,
        state: Option<adk_core::State>,
        session_id: Option<String>,
    ) -> adk_core::Result<Session> {
        let mut session = Session::new(
            session_id.unwrap_or_else(|| adk_core::new_id("session")),
            app_name,
            user_id,
        );
        if let Some(state) = state {
            session.state = state;
        }
        Ok(session)
    }

    async fn get_session(
        &self,
        _app_name: &str,
        _user_id: &str,
        _session_id: &str,
    ) -> adk_core::Result<Option<Session>> {
        Ok(None)
    }

    async fn list_sessions(
        &self,
        _app_name: &str,
        _user_id: &str,
    ) -> adk_core::Result<Vec<Session>> {
        Ok(Vec::new())
    }

    async fn delete_session(
        &self,
        _app_name: &str,
        _user_id: &str,
        _session_id: &str,
    ) -> adk_core::Result<()> {
        Ok(())
    }

    async fn append_event(
        &self,
        session: &mut Session,
        event: adk_core::Event,
    ) -> adk_core::Result<()> {
        if event.is_partial() {
            return Ok(());
        }
        session.state.commit(event.actions.state_delta.clone());
        session.events.push(event);
        Ok(())
    }
}
