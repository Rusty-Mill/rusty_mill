//! The transport flags and how to serve on them: stdio, or Streamable HTTP
//! with an optional shared-secret bearer token. These replace the flags,
//! logging setup and authorization the `rusty-mcp` scaffold used to provide.

use std::net::SocketAddr;
use std::sync::Arc;

use clap::{Parser, ValueEnum};
use rusty_crypto_key::constant_time_eq;
use rusty_http::StatusCode;
use rusty_mcp_server::{HttpConfig, HttpHandler, Server, serve_stdio};
use rusty_serve::{Limits, Request, Response, SharedHandler};

/// Which transport to serve on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TransportArg {
    /// stdin/stdout, for locally launched servers.
    Stdio,
    /// Streamable HTTP.
    Http,
}

/// The transport and logging flags.
#[derive(Debug, Clone, Parser)]
pub struct McpArgs {
    /// Transport to serve on.
    #[arg(long, value_enum, default_value = "stdio", env = "MCP_TRANSPORT")]
    pub transport: TransportArg,

    /// Address to bind (HTTP only).
    #[arg(long, default_value = "127.0.0.1:8080", env = "MCP_BIND")]
    pub bind: SocketAddr,

    /// Path to mount the MCP endpoint at (HTTP only).
    #[arg(long, default_value = "/mcp", env = "MCP_PATH")]
    pub path: String,

    /// Accepted `Host` values (HTTP only, repeatable). Defaults to loopback
    /// only; set this for any non-local deployment.
    #[arg(
        long = "allowed-host",
        env = "MCP_ALLOWED_HOSTS",
        value_delimiter = ','
    )]
    pub allowed_hosts: Option<Vec<String>>,

    /// Accepted browser `Origin` values (HTTP only, repeatable).
    #[arg(
        long = "allowed-origin",
        env = "MCP_ALLOWED_ORIGINS",
        value_delimiter = ','
    )]
    pub allowed_origins: Option<Vec<String>>,

    /// Keep sessions for clients that open one with `initialize` (HTTP only).
    /// Off by default; 2026-07-28 clients are served statelessly regardless.
    #[arg(long, env = "MCP_LEGACY_SESSIONS")]
    pub legacy_sessions: bool,

    /// Largest accepted request body, in bytes (HTTP only).
    #[arg(long, default_value_t = 1_048_576, env = "MCP_MAX_BODY_BYTES")]
    pub max_body_bytes: u64,

    /// Event-stream keep-alive interval in seconds (HTTP only).
    #[arg(long, default_value_t = 15, env = "MCP_SSE_KEEP_ALIVE_SECS")]
    pub sse_keep_alive_secs: u64,

    /// Connections served at once (HTTP only); over it, `503`.
    #[arg(long, default_value_t = 256, env = "MCP_MAX_CONCURRENT")]
    pub max_concurrent: usize,

    /// Log filter directive. `RUST_LOG` overrides this when set.
    #[arg(long, default_value = "info", env = "MCP_LOG")]
    pub log: String,
}

/// Send logs to stderr (stdout carries the protocol on stdio).
pub fn init_logging(filter: &str) {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));
    let _ = tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr).with_ansi(false))
        .with(env_filter)
        .try_init();
}

/// Requires `Authorization: Bearer <token>` before handing a request to
/// `inner`; the comparison takes the same time whatever the token.
struct BearerGate {
    token: String,
    inner: HttpHandler,
}

impl SharedHandler for BearerGate {
    fn handle(&self, request: &Request<'_>) -> Response {
        let presented = request
            .authorization
            .and_then(rusty_oauth::bearer::token_from_authorization)
            .unwrap_or("");
        if constant_time_eq(presented.as_bytes(), self.token.as_bytes()) {
            return self.inner.handle(request);
        }
        let body =
            br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"unauthorized"}}"#;
        Response::json(StatusCode::UNAUTHORIZED, body.to_vec())
    }
}

/// Serve `server` on the transport `args` names until it ends. Blocks.
///
/// # Errors
/// A message for a bind failure or a transport failure.
pub fn serve(server: Arc<Server>, args: &McpArgs, token: Option<String>) -> Result<(), String> {
    match args.transport {
        TransportArg::Stdio => serve_stdio(server).map_err(|e| format!("stdio failed: {e}")),
        TransportArg::Http => serve_http(server, args, token),
    }
}

fn serve_http(server: Arc<Server>, args: &McpArgs, token: Option<String>) -> Result<(), String> {
    let http = bind(server, args, token)?;
    tracing::info!(bind = %args.bind, path = %args.path, "MCP server listening");
    http.run().map_err(|e| format!("HTTP server failed: {e}"))
}

/// The HTTP server for `args`, bound but not yet running.
fn bind(
    server: Arc<Server>,
    args: &McpArgs,
    token: Option<String>,
) -> Result<rusty_serve::Server, String> {
    let mut config = HttpConfig {
        path: args.path.clone(),
        keep_alive: std::time::Duration::from_secs(args.sse_keep_alive_secs.max(1)),
        max_sessions: if args.legacy_sessions { 1024 } else { 0 },
        ..HttpConfig::default()
    };
    if let Some(hosts) = &args.allowed_hosts {
        config.allowed_hosts.clone_from(hosts);
    }
    if let Some(origins) = &args.allowed_origins {
        config.allowed_origins.clone_from(origins);
    }
    let limits = Limits {
        max_connections: args.max_concurrent.max(1),
        max_body_bytes: args.max_body_bytes,
        ..Limits::default()
    };
    let handler = HttpHandler::new(server, config);
    let bound = match token {
        Some(token) => rusty_serve::Server::bind_shared(
            args.bind,
            BearerGate {
                token,
                inner: handler,
            },
        ),
        None => rusty_serve::Server::bind_shared(args.bind, handler),
    };
    Ok(bound
        .map_err(|e| format!("cannot bind {}: {e}", args.bind))?
        .with_limits(limits))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn args() -> McpArgs {
        McpArgs::parse_from(["homelab", "--transport", "http", "--bind", "127.0.0.1:0"])
    }

    fn empty_server() -> Arc<Server> {
        Arc::new(Server::builder("t", "1").build().unwrap())
    }

    /// POST a ping with `authorization`, return the status line's code.
    fn status_of(addr: SocketAddr, authorization: Option<&str>) -> u16 {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let mut raw = format!(
            "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        );
        if let Some(a) = authorization {
            raw.push_str(&format!("Authorization: {a}\r\n"));
        }
        raw.push_str("\r\n");
        raw.push_str(body);
        let mut sock = TcpStream::connect(addr).unwrap();
        sock.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = sock.read_to_string(&mut out);
        out.split(' ')
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0)
    }

    #[test]
    fn the_bearer_gate_admits_only_the_right_token() {
        let http = bind(empty_server(), &args(), Some("s3cret".to_owned())).unwrap();
        let addr = http.local_addr().unwrap();
        let stop = http.shutdown_handle().unwrap();
        std::thread::spawn(move || {
            let _ = http.run();
        });
        assert_eq!(status_of(addr, None), 401);
        assert_eq!(status_of(addr, Some("Bearer wrong")), 401);
        assert_eq!(status_of(addr, Some("Basic s3cret")), 401);
        assert_eq!(status_of(addr, Some("Bearer s3cret")), 200);
        assert_eq!(status_of(addr, Some("bearer s3cret")), 200);
        stop.shutdown();
    }

    #[test]
    fn without_a_token_the_endpoint_is_open() {
        let http = bind(empty_server(), &args(), None).unwrap();
        let addr = http.local_addr().unwrap();
        let stop = http.shutdown_handle().unwrap();
        std::thread::spawn(move || {
            let _ = http.run();
        });
        assert_eq!(status_of(addr, None), 200);
        stop.shutdown();
    }
}
