//! MCP server mode (PRD 07 / ADR-0029): `rusty-keys --mcp` exposes a single
//! `chat` tool that maps to [`Session::send`], over `rmcp`'s stdio JSON-RPC
//! transport. `rmcp` owns the wire protocol; the harness layer is not bypassed —
//! a `chat` call runs the same turn cycle, verification, and evidence journal as
//! a CLI turn. Behind the `mcp-server` feature; not exercisable in offline CI.

use std::sync::Arc;

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use anyhow::Context;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler};
use serde_json::json;

use crate::Session;

/// The MCP server handler — owns one [`Session`] and serves `chat`.
struct ChatServer<M> {
    session: Arc<Session<M>>,
}

impl<M> ServerHandler for ChatServer<M>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    fn get_info(&self) -> ServerInfo {
        rusty_mcp::server_info(
            "rusty-keys",
            env!("CARGO_PKG_VERSION"),
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_instructions("Rusty Keys harness exposed over MCP. Call `chat` to run a turn.")
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        let schema = json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" },
                "session_id": { "type": "string", "description": "Resume a named session" }
            },
            "required": ["message"]
        });
        let map = schema.as_object().cloned().unwrap_or_default();
        let tool = Tool::new(
            "chat",
            "Send a message to Rusty Keys and receive a reply.",
            Arc::new(map),
        );
        let mut result = ListToolsResult {
            tools: vec![tool],
            ..Default::default()
        };
        rusty_mcp::apply_cache_hints(&ctx, &mut result.ttl_ms, &mut result.cache_scope);
        std::future::ready(Ok(result))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResponse, ErrorData>> + Send + '_ {
        let session = self.session.clone();
        async move {
            if request.name != "chat" {
                return Err(ErrorData::method_not_found::<
                    rmcp::model::CallToolRequestMethod,
                >());
            }
            let message = request
                .arguments
                .as_ref()
                .and_then(|a| a.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match session.send(message).await {
                Ok(outcome) => Ok(CallToolResponse::from(CallToolResult::success(vec![
                    ContentBlock::text(outcome.reply),
                ]))),
                Err(e) => Ok(CallToolResponse::from(CallToolResult::error(vec![
                    ContentBlock::text(format!("harness error: {e}")),
                ]))),
            }
        }
    }
}

/// Run the MCP server over stdio until the client disconnects (the same turn
/// cycle as the CLI; `Session::send()` is not bypassed).
pub async fn serve<M>(session: Session<M>) -> anyhow::Result<()>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    let session = Arc::new(session);
    rusty_mcp::serve(
        move || {
            Ok(ChatServer {
                session: session.clone(),
            })
        },
        rusty_mcp::ServerConfig::stdio(),
    )
    .await
    .context("MCP server")
}
