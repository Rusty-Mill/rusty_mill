//! The merged MCP handler: rusty_provider's own tools (Direction A) plus
//! every proxied upstream tool (Direction B), presented as one `tools/list`.
//!
//! `list_tools`/`call_tool`/`get_tool` are hand-written rather than built
//! with `rusty_mcp::forward_tool_methods!`: that macro forwards a single
//! static `ToolRouter`, but this handler's tool set spans a compile-time
//! native router *and* a runtime-discovered set of upstream tools. It still
//! reuses `rusty_mcp::pagination::page_owned` for correct cursor paging and
//! `rusty_mcp::server_info`/`PROTOCOL_VERSION` for the handshake, same as a
//! server built directly on the scaffold would.

use std::sync::Arc;

use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorData, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use rusty_mcp::pagination::{page_owned, CursorKind, DEFAULT_PAGE_SIZE};

use crate::gateway::McpGateway;
use crate::native::NativeTools;

/// rusty_provider's combined MCP surface: its own routing as tools, plus
/// every tool proxied from a configured `[[mcp.upstreams]]` entry.
#[derive(Clone)]
pub struct RustyMcpServer {
    native: NativeTools,
    gateway: Arc<McpGateway>,
}

impl RustyMcpServer {
    pub fn new(native: NativeTools, gateway: Arc<McpGateway>) -> Self {
        Self { native, gateway }
    }
}

impl ServerHandler for RustyMcpServer {
    fn get_info(&self) -> ServerInfo {
        rusty_mcp::server_info(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_instructions(
            "rusty_provider's own LLM routing (chat_completion/list_models/embeddings), \
             plus any tool proxied from a configured MCP upstream, named \
             \"{upstream}/{tool}\".",
        )
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let cursor = request.as_ref().and_then(|r| r.cursor.as_deref());

        let mut all: Vec<Tool> = self.native.tool_router.list_all();
        all.extend(self.gateway.list_tools().await);
        all.sort_by(|a, b| a.name.cmp(&b.name));

        let (tools, next) = page_owned(
            &all,
            |tool| tool.name.as_ref(),
            CursorKind::Tool,
            cursor,
            DEFAULT_PAGE_SIZE,
        )?;

        let mut result = ListToolsResult::with_all_items(tools);
        result.next_cursor = next;
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if let Some((upstream, tool)) = request.name.split_once('/') {
            return self
                .gateway
                .call_tool(upstream, tool, request.arguments.clone())
                .await
                .map_err(|e| ErrorData::invalid_params(e.to_string(), None));
        }

        let call = ToolCallContext::new(&self.native, request, context);
        self.native.tool_router.call(call).await
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        if let Some((_upstream, _tool)) = name.split_once('/') {
            // Proxied tools are discovered per `list_tools` call, not cached
            // here -- `get_tool` is only consulted for pre-call schema
            // validation, and skipping it for proxied tools just means that
            // validation happens upstream instead of locally.
            return None;
        }
        self.native.tool_router.get(name).cloned()
    }
}
