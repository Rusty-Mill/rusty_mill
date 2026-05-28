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
    CallToolRequestParam, CallToolResult, Content, ListToolsResult, PaginatedRequestParam,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt};
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
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "Rusty Keys harness exposed over MCP. Call `chat` to run a turn.".into(),
            ),
            ..Default::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _ctx: RequestContext<RoleServer>,
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
        std::future::ready(Ok(ListToolsResult {
            tools: vec![tool],
            next_cursor: None,
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _ctx: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, ErrorData>> + Send + '_ {
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
                Ok(outcome) => Ok(CallToolResult::success(vec![Content::text(outcome.reply)])),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "harness error: {e}"
                ))])),
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
    let handler = ChatServer {
        session: Arc::new(session),
    };
    let running = handler
        .serve(rmcp::transport::stdio())
        .await
        .context("starting MCP server")?;
    running.waiting().await.context("MCP server")?;
    Ok(())
}
