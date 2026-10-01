//! The MCP server: exposes ADK tools to any MCP client.
//!
//! [`McpServer`] is an `rmcp` server handler: the stdio and HTTP transports
//! hand it to `rmcp`, which owns the wire protocol. This module owns only
//! what is ADK-specific, running ADK tools against a session.

use std::borrow::Cow;

use adk_core::{InvocationContext, RunConfig, Services, Session};
use adk_tools::{invoke_tool, SharedTool, ToolContext};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorCode, Implementation, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use std::sync::Arc;

use crate::protocol::{protocol_version, tool_entry, tool_result, SUPPORTED_VERSIONS};

/// Serves a set of ADK tools over the Model Context Protocol.
///
/// An [`rmcp`] server handler: the transports in this crate hand it to
/// `rmcp`, which owns the wire protocol. Cloning is cheap (the tools are
/// shared), and each connection serves its own clone.
#[derive(Clone)]
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

    /// Runs `tools/call` for the named tool.
    async fn call(&self, request: CallToolRequestParams) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.as_ref();
        let args = request.arguments.unwrap_or_default();
        let tool = self
            .tools
            .iter()
            .find(|t| t.name() == name)
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::METHOD_NOT_FOUND,
                    format!("unknown tool '{name}'"),
                    None,
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
            return Err(ErrorData::internal_error(
                format!(
                    "tool '{name}' requires user confirmation ('{hint}') before it can run, \
                     but the MCP stdio/http transport does not support confirmation-gated \
                     tools: there is no channel to carry an approval back to a suspended call"
                ),
                None,
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
        Ok(tool_result(&value).into())
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(self.name.clone(), self.version.clone()))
            .with_protocol_version(protocol_version())
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_VERSIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self
            .tools
            .iter()
            .filter_map(|t| t.declaration())
            .map(|d| tool_entry(&d))
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.call(request).await
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
