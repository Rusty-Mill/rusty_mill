//! The MCP server: exposes ADK tools to any MCP client.
//!
//! [`McpServer`] holds the ADK tools; the transports in this crate turn it
//! into a `rusty_mcp_server` server, which owns the wire protocol. This module
//! owns only what is ADK-specific, running ADK tools against a session.
//!
//! `rusty_mcp_server`'s handlers block, while ADK tools are async, so each
//! call runs the tool on the tokio runtime the transport was started on
//! (stdio, HTTP and [`router`](crate::router) record it).

use std::sync::{Arc, OnceLock};

use adk_core::{InvocationContext, RunConfig, Services, Session};
use adk_tools::{invoke_tool, SharedTool, ToolContext};
use rusty_mcp_server::proto::{
    CallToolParams, CallToolResult, ErrorCode, ErrorData, Tool as McpTool,
};
use rusty_mcp_server::{BuildError, CallContext, Server, ToolSource};
use serde_json::{Map, Value};
use tokio::runtime::Handle;

use crate::protocol::{from_wire, supported_versions, tool_entry, tool_result};

/// Serves a set of ADK tools over the Model Context Protocol.
///
/// Cloning is cheap (the tools are shared), and each connection serves its
/// own clone.
#[derive(Clone)]
pub struct McpServer {
    name: String,
    version: String,
    tools: Vec<SharedTool>,
    services: Services,
    app_name: String,
    /// The runtime tool calls run on; set by the first transport started.
    runtime: Arc<OnceLock<Handle>>,
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
            runtime: Arc::new(OnceLock::new()),
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

    /// Records the current tokio runtime as the one tool calls run on (the
    /// first one recorded wins). Does nothing outside a runtime.
    #[cfg_attr(not(any(feature = "stdio", feature = "http")), allow(dead_code))]
    pub(crate) fn bind_runtime(&self) {
        if let Ok(handle) = Handle::try_current() {
            let _ = self.runtime.set(handle);
        }
    }

    /// The `rusty_mcp_server` description of this server, speaking the
    /// revisions up to [`PROTOCOL_VERSION`](crate::PROTOCOL_VERSION).
    #[cfg_attr(not(any(feature = "stdio", feature = "http")), allow(dead_code))]
    pub(crate) fn wire_server(&self) -> Result<Server, BuildError> {
        Server::builder(self.name.clone(), self.version.clone())
            .versions(supported_versions())
            .tool_source(self.clone())
            .build()
    }

    /// Runs `tools/call` for the named tool.
    async fn call(
        &self,
        name: &str,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, ErrorData> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name() == name)
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::METHOD_NOT_FOUND,
                    format!("unknown tool '{name}'"),
                )
            })?;

        // This transport has no channel to carry a confirmation answer back
        // to the tool: `tools/call` is a single request/response round trip
        // with no pause-and-resume step, so `ToolContext::tool_confirmation`
        // is always `None` here. Left unchecked, `invoke_tool` would call
        // `ctx.request_confirmation` and the resulting
        // `AdkError::ConfirmationRequired` would fall into the generic tool
        // error branch below, indistinguishable from a real tool failure.
        // Fail fast instead, with a message that names the actual limitation.
        if let Some(hint) = tool.confirmation_hint(&args) {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "tool '{name}' requires user confirmation ('{hint}') before it can run, \
                     but the MCP stdio/http transport does not support confirmation-gated \
                     tools: there is no channel to carry an approval back to a suspended call"
                ),
            ));
        }

        let session = Session::new(adk_core::new_id("mcp"), &self.app_name, "mcp-client");
        let ctx = ToolContext::new(InvocationContext::new(
            session,
            self.services.clone(),
            RunConfig::default(),
        ));

        // A tool failure is reported as an MCP tool error, not a JSON-RPC
        // error: the protocol distinguishes "the tool ran and failed" from
        // "the request was malformed", and clients rely on that difference.
        let value = match invoke_tool(tool.as_ref(), args, &ctx).await {
            Ok(value) => value,
            Err(err) => adk_tools::error(err.to_string()),
        };
        Ok(tool_result(&value))
    }
}

impl ToolSource for McpServer {
    fn tools(&self) -> Vec<McpTool> {
        self.tools
            .iter()
            .filter_map(|t| t.declaration())
            .map(|d| tool_entry(&d))
            .collect()
    }

    fn call(
        &self,
        _ctx: &CallContext,
        call: &CallToolParams,
    ) -> Option<Result<CallToolResult, ErrorData>> {
        // Every name is this source's to answer (an unknown one with an
        // error of its own), so the server's generic one is never used.
        let args = match call.arguments.as_ref().map(from_wire) {
            None => Map::new(),
            Some(Value::Object(map)) => map,
            Some(_) => {
                return Some(Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "tool arguments must be a JSON object",
                )))
            }
        };
        let Some(handle) = self
            .runtime
            .get()
            .cloned()
            .or_else(|| Handle::try_current().ok())
        else {
            return Some(Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "no tokio runtime to run the tool on",
            )));
        };
        Some(handle.block_on(self.call(&call.name, args)))
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
