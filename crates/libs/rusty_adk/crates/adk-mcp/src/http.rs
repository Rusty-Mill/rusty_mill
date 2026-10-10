//! Streamable HTTP transport.
//!
//! This is what an ADK agent's `StreamableHTTPConnectionParams` connects to:
//! JSON-RPC requests are POSTed to a single endpoint, and the response comes
//! back as JSON. Served by `rusty_mcp_server`'s HTTP handler (mounted in axum
//! by `rusty_mcp_axum`), stateless (no `Mcp-Session-Id`), so it scales behind
//! a plain load balancer.

use std::sync::Arc;
use std::time::Duration;

use adk_core::{AdkError, Result};
use axum::Router;
use rusty_mcp_server::{HttpConfig, HttpHandler};

use crate::server::McpServer;

/// The largest request body accepted.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// A reply is always plain JSON: this is far past any tool's runtime.
const JSON_ONLY: Duration = Duration::from_secs(24 * 60 * 60);

/// Builds an Axum router serving `server` at `path`.
///
/// Mounting a router rather than owning the listener lets the MCP endpoint sit
/// alongside an application's own routes.
///
/// Any `Host` header is accepted, as before this crate moved onto
/// `rusty_mcp_server`: the router cannot tell whether it is mounted on a
/// loopback-only listener, and a loopback-only default would reject every
/// remote client of a server bound to a public address. Call it from inside a
/// tokio runtime: tool calls run on the runtime that was current here.
pub fn router(server: Arc<McpServer>, path: &str) -> Router {
    server.bind_runtime();
    let path = if path.is_empty() { "/" } else { path };
    let wire = match server.wire_server() {
        Ok(wire) => wire,
        // The description is fixed by this crate; every reply is a 500 if it
        // is somehow invalid, rather than a panic at router construction.
        Err(e) => {
            tracing::error!(error = %e, "MCP server description is invalid");
            return Router::new().fallback(|| async {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "MCP server unavailable",
                )
            });
        }
    };
    let handler = Arc::new(HttpHandler::new(
        Arc::new(wire),
        HttpConfig {
            path: path.to_owned(),
            allowed_hosts: Vec::new(),
            max_sessions: 0,
            sse_after: JSON_ONLY,
            ..HttpConfig::default()
        },
    ));
    let mounted = rusty_mcp_axum::router(handler, MAX_BODY_BYTES);
    if path == "/" {
        Router::new().fallback_service(mounted)
    } else {
        Router::new().nest_service(path, mounted)
    }
}

/// Serves `server` over HTTP on `addr` at `path`, until the process ends.
pub async fn serve_http(server: Arc<McpServer>, addr: &str, path: &str) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| AdkError::Config(format!("cannot bind {addr}: {e}")))?;

    tracing::info!(%addr, %path, "MCP server listening");

    axum::serve(listener, router(server, path))
        .await
        .map_err(|e| AdkError::Other(format!("HTTP server failed: {e}")))
}
