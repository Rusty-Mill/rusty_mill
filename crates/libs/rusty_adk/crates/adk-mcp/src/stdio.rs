//! Stdio transport: newline-delimited JSON-RPC over stdin/stdout.
//!
//! This is what an ADK agent's `StdioConnectionParams` launches — the server
//! is a subprocess, and the agent speaks JSON-RPC to it over pipes.
//!
//! Anything the server wants to log must go to **stderr**: stdout carries the
//! protocol, and a stray `println!` corrupts the stream.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use adk_core::{AdkError, Result};
use rmcp::service::ServerInitializeError;
use rmcp::ServiceExt;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::line_cap::{cap_error, LineCapped};
use crate::server::McpServer;

/// Serves `server` over stdin and stdout until stdin closes.
pub async fn serve_stdio(server: &McpServer) -> Result<()> {
    serve_stream(server, tokio::io::stdin(), tokio::io::stdout()).await
}

/// Serves `server` over an arbitrary reader and writer, until the reader
/// ends.
///
/// Exposed separately so the transport can be exercised over in-memory pipes.
///
/// # Errors
/// Fails if a line exceeds the 16 MiB cap (the session ends rather than
/// buffering it), or if the session fails for any reason other than the
/// peer going away.
pub async fn serve_stream<R, W>(server: &McpServer, reader: R, writer: W) -> Result<()>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let exceeded = Arc::new(AtomicBool::new(false));
    let reader = LineCapped::new(reader, Arc::clone(&exceeded));
    let outcome = match server.clone().serve((reader, writer)).await {
        Ok(running) => running
            .waiting()
            .await
            .map(drop)
            .map_err(|e| AdkError::Other(format!("MCP server task failed: {e}"))),
        // The peer left before (or while) initializing: nothing to serve.
        Err(ServerInitializeError::ConnectionClosed(_)) => Ok(()),
        Err(err) => Err(AdkError::Other(format!("MCP session failed: {err}"))),
    };
    if exceeded.load(Ordering::Acquire) {
        return Err(AdkError::Other(cap_error()));
    }
    outcome
}
