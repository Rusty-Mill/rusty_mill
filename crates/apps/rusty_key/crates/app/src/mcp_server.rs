//! MCP server mode (PRD 07 / ADR-0029): `rusty-keys --mcp` exposes a single
//! `chat` tool that maps to [`Session::send`], over stdio. `rusty_mcp_server`
//! owns the wire protocol; the harness layer is not bypassed -- a `chat` call
//! runs the same turn cycle, verification, and evidence journal as a CLI turn.
//! Behind the `mcp-server` feature; not exercisable in offline CI.
//!
//! The server is blocking, one request at a time. `Session::send` is async, so
//! the handler runs it on the caller's tokio runtime with `Handle::block_on`
//! from a `spawn_blocking` thread.

use std::sync::Arc;

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use anyhow::Context;
use rusty_json::Value;
use rusty_mcp_proto::schema::Kind;
use rusty_mcp_proto::{CallToolResult, Schema, Tool};
use rusty_mcp_server::stdio::{serve_stdio, Options};
use rusty_mcp_server::{Dispatcher, Server};

use crate::Session;

/// Run the MCP server over stdio until the client disconnects (the same turn
/// cycle as the CLI; `Session::send()` is not bypassed).
pub async fn serve<M>(session: Session<M>) -> anyhow::Result<()>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    let session = Arc::new(session);
    let runtime = tokio::runtime::Handle::current();
    let chat = Tool::new(
        "chat",
        "Send a message to Rusty Keys and receive a reply.",
        Schema::object()
            .field("message", Kind::String, "", true)
            .field("session_id", Kind::String, "Resume a named session", false)
            .build(),
    );
    let server = Server::new("rusty-keys", env!("CARGO_PKG_VERSION"))
        .instructions("Rusty Keys harness exposed over MCP. Call `chat` to run a turn.")
        .tool(chat, move |_ctx, args| {
            let message = args.get("message").and_then(Value::as_str).unwrap_or("");
            Ok(match runtime.block_on(session.send(message)) {
                Ok(outcome) => CallToolResult::text(outcome.reply),
                Err(e) => CallToolResult::error(format!("harness error: {e}")),
            })
        });
    tokio::task::spawn_blocking(move || serve_stdio(Dispatcher::new(server), Options::default()))
        .await
        .context("MCP server task")?
        .context("MCP server")
}
