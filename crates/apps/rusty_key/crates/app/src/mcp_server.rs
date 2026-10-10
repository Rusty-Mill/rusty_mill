//! MCP server mode (PRD 07 / ADR-0029): `rusty-keys --mcp` exposes a single
//! `chat` tool that maps to [`Session::send`], over `rusty_mcp_server`'s stdio
//! transport. The server owns the wire protocol; the harness layer is not
//! bypassed -- a `chat` call runs the same turn cycle, verification, and
//! evidence journal as a CLI turn. Behind the `mcp-server` feature.
//!
//! The server runs its tools on plain threads, while a turn is async: each
//! `chat` call drives its turn on the caller's tokio runtime with
//! [`Handle::block_on`], so the runtime must be multi-threaded (the binary's
//! is).

use std::sync::Arc;

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use anyhow::Context;
use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{CallToolResult, ContentBlock, Tool};
use rusty_mcp_server::Server;
use tokio::runtime::Handle;

use crate::Session;

/// The `chat` tool's input schema.
const CHAT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "message": { "type": "string" },
        "session_id": { "type": "string", "description": "Resume a named session" }
    },
    "required": ["message"]
}"#;

fn text_result(text: String, is_error: bool) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text(text)],
        is_error: Some(is_error),
        ..CallToolResult::default()
    }
}

/// The server for `session`: one tool, `chat`. A turn that fails is a tool
/// result with `isError`, not a protocol error. Turns are driven on `rt`.
pub fn build_server<M>(session: Arc<Session<M>>, rt: Handle) -> anyhow::Result<Server>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    let schema = Value::from_json_str(CHAT_SCHEMA).context("the chat tool's schema")?;
    let mut chat = Tool::new("chat", schema);
    chat.description = Some("Send a message to Rusty Keys and receive a reply.".to_owned());
    Server::builder("rusty-keys", env!("CARGO_PKG_VERSION"))
        .instructions("Rusty Keys harness exposed over MCP. Call `chat` to run a turn.")
        .tool(chat, move |_ctx, call| {
            let message = call
                .arguments
                .as_ref()
                .and_then(|a| a.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            // A client that cancels still gets the turn run to completion (its
            // answer is dropped): aborting `send` half way could leave the
            // evidence journal and session state inconsistent.
            Ok(match rt.block_on(session.send(&message)) {
                Ok(outcome) => text_result(outcome.reply, false),
                Err(e) => text_result(format!("harness error: {e}"), true),
            })
        })
        .build()
        .context("building the MCP server")
}

/// Resolves on ctrl-c or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

/// Run the MCP server over stdio until the client disconnects or a shutdown
/// signal arrives (the same turn cycle as the CLI; `Session::send()` is not
/// bypassed).
pub async fn serve<M>(session: Session<M>) -> anyhow::Result<()>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    let server = Arc::new(build_server(Arc::new(session), Handle::current())?);
    // A plain thread, not `spawn_blocking`: reading stdin cannot be
    // interrupted, and the runtime would wait for a blocking task forever.
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("mcp-stdio".into())
        .spawn(move || {
            let _ = done_tx.send(rusty_mcp_server::serve_stdio(server));
        })
        .context("starting the MCP stdio thread")?;
    tokio::select! {
        done = done_rx => done
            .context("the MCP stdio thread ended without a result")?
            .context("MCP server"),
        () = shutdown_signal() => Ok(()),
    }
}
