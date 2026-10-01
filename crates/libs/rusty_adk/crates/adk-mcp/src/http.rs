//! Streamable HTTP transport.
//!
//! This is what an ADK agent's `StreamableHTTPConnectionParams` connects to:
//! JSON-RPC requests are POSTed to a single endpoint, and the response comes
//! back as JSON. Served by `rmcp`'s Streamable HTTP service, stateless (no
//! `Mcp-Session-Id`), so it scales behind a plain load balancer.

use adk_core::{AdkError, Result};
use axum::Router;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use std::sync::Arc;

use crate::server::McpServer;

/// Builds an Axum router serving `server` at `path`.
///
/// Mounting a router rather than owning the listener lets the MCP endpoint sit
/// alongside an application's own routes.
///
/// Any `Host` header is accepted, as before this crate moved onto `rmcp`:
/// the router cannot tell whether it is mounted on a loopback-only listener,
/// and `rmcp`'s loopback-only default would reject every remote client of a
/// server bound to a public address.
pub fn router(server: Arc<McpServer>, path: &str) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .disable_allowed_hosts();
    let service = StreamableHttpService::new(
        move || Ok(McpServer::clone(&server)),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new().route_service(path, service)
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
