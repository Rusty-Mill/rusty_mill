//! Streamable HTTP transport.
//!
//! This is what an ADK agent's `StreamableHTTPConnectionParams` connects to:
//! JSON-RPC requests are POSTed to a single endpoint, and the response comes
//! back as JSON. Served by `rusty_mcp_server`'s HTTP handler, mounted in a
//! `rusty_axum` router, stateless (no `Mcp-Session-Id`), so it scales behind
//! a plain load balancer.

use std::sync::Arc;
use std::time::Duration;

use adk_core::{AdkError, Result};
use rusty_axum::routing::any;
use rusty_axum::{BoxFuture, Request, Response, Router, ServeOptions, StatusCode};
use rusty_mcp_server::{HttpConfig, HttpHandler, Mount};

use crate::server::McpServer;

/// The largest request body accepted, by the router itself: an app that
/// mounts it may allow larger bodies on its other routes.
const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;

/// A reply is always plain JSON: this is far past any tool's runtime.
const JSON_ONLY: Duration = Duration::from_secs(24 * 60 * 60);

/// Builds a `rusty_axum` router serving `server` at `path`.
///
/// Mounting a router rather than owning the listener lets the MCP endpoint sit
/// alongside an application's own routes. An empty path or `/` serves every
/// path; any other path is matched exactly.
///
/// Any `Host` header is accepted, as before this crate moved onto
/// `rusty_mcp_server`: the router cannot tell whether it is mounted on a
/// loopback-only listener, and a loopback-only default would reject every
/// remote client of a server bound to a public address. Call it from inside a
/// tokio runtime: tool calls run on the runtime that was current here. Serve
/// the router on a `rusty_tokio` runtime, as [`serve_http`] does.
pub fn router(server: Arc<McpServer>, path: &str) -> Router {
    server.bind_runtime();
    let path = if path.is_empty() { "/" } else { path };
    let wire = match server.wire_server() {
        Ok(wire) => wire,
        // The description is fixed by this crate; every reply is a 500 if it
        // is somehow invalid, rather than a panic at router construction.
        Err(e) => {
            rusty_log::error!(error = %e, "MCP server description is invalid");
            return Router::new().fallback(|| async {
                (StatusCode::INTERNAL_SERVER_ERROR, "MCP server unavailable")
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
    let mcp = Mount::new(handler).at(path);
    if path == "/" {
        Router::new().fallback(move |req| bounded(&mcp, req))
    } else {
        Router::new().route(path, any(move |req| bounded(&mcp, req)))
    }
}

/// Answers `req` with `mcp`, or 413 when its body is over [`MAX_BODY_BYTES`].
fn bounded(mcp: &Mount, req: Request) -> BoxFuture<'static, Response> {
    if req.body().len() as u64 > MAX_BODY_BYTES {
        return Box::pin(async {
            Response::new(StatusCode::PAYLOAD_TOO_LARGE, "request body too large")
        });
    }
    mcp.call(req)
}

/// Serves `server` over HTTP on `addr` at `path`, until the process ends.
///
/// Call it from inside a tokio runtime, which tool calls run on. The server
/// itself runs on a `rusty_tokio` runtime of its own, on a blocking thread of
/// the caller's runtime, so dropping the returned future does not stop it.
pub async fn serve_http(server: Arc<McpServer>, addr: &str, path: &str) -> Result<()> {
    let listener = std::net::TcpListener::bind(addr)
        .and_then(|listener| listener.set_nonblocking(true).map(|()| listener))
        .map_err(|e| AdkError::Config(format!("cannot bind {addr}: {e}")))?;
    let app = router(server, path);

    rusty_log::info!(%addr, %path, "MCP server listening");

    tokio::task::spawn_blocking(move || serve_until_exit(listener, app))
        .await
        .map_err(|e| AdkError::Other(format!("HTTP server task failed: {e}")))?
        .map_err(|e| AdkError::Other(format!("HTTP server failed: {e}")))
}

/// Runs `app` on `listener` on a new `rusty_tokio` runtime, forever.
fn serve_until_exit(listener: std::net::TcpListener, app: Router) -> std::io::Result<()> {
    let runtime = rusty_tokio::Builder::new_multi_thread()
        .thread_name("adk-mcp-http")
        .build()?;
    runtime.block_on(async move {
        let listener = rusty_tokio::io::TcpListener::from_std(listener)?;
        let options = ServeOptions {
            max_body_bytes: MAX_BODY_BYTES,
            ..ServeOptions::default()
        };
        rusty_axum::serve(listener, None, options, app)?
            .shutdown_on(std::future::pending())
            .await;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::Services;
    use adk_sessions::InMemorySessionService;
    use rusty_axum::Method;

    /// The status `router` answers a POST of `len` bytes to `/mcp` with, the
    /// way a host app that allows larger bodies would hand it over.
    fn status_for(len: usize) -> StatusCode {
        let tokio = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .unwrap();
        let services = Services::new(Arc::new(InMemorySessionService::new()));
        let server = Arc::new(McpServer::new("test", Vec::new(), services));
        let app = tokio.block_on(async { router(server, "/mcp") });
        let request = Request::new(Method::Post, "/mcp")
            .with_header("content-type", "application/json")
            .unwrap()
            .with_body(vec![b' '; len]);
        let runtime = rusty_tokio::Builder::new_current_thread().build().unwrap();
        runtime.block_on(app.call(request)).status
    }

    #[test]
    fn a_body_over_the_limit_is_refused_whatever_the_host_allows() {
        let over = usize::try_from(MAX_BODY_BYTES).unwrap() + 1;
        assert_eq!(status_for(over), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn a_body_at_the_limit_reaches_the_handler() {
        let at = usize::try_from(MAX_BODY_BYTES).unwrap();
        let status = status_for(at);
        assert_ne!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_ne!(status, StatusCode::NOT_FOUND);
    }
}
