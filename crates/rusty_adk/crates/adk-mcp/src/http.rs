//! Streamable HTTP transport.
//!
//! This is what an ADK agent's `StreamableHTTPConnectionParams` connects to:
//! JSON-RPC requests are POSTed to a single endpoint, and the response comes
//! back as JSON.

use adk_core::{AdkError, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use std::sync::Arc;

use crate::server::McpServer;

/// Builds an Axum router serving `server` at `path`.
///
/// Mounting a router rather than owning the listener lets the MCP endpoint sit
/// alongside an application's own routes.
pub fn router(server: Arc<McpServer>, path: &str) -> Router {
    Router::new().route(path, post(handle)).with_state(server)
}

async fn handle(State(server): State<Arc<McpServer>>, body: String) -> Response {
    match server.handle_raw(&body).await {
        Some(response) => (
            StatusCode::OK,
            [("content-type", "application/json")],
            response,
        )
            .into_response(),
        // A notification is answered with 202 and no body, per JSON-RPC.
        None => StatusCode::ACCEPTED.into_response(),
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
