//! Server configuration: which transport to run and how to tune it.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use crate::auth::AuthConfig;

/// Default HTTP listen address. Loopback-only, matching the transport's
/// default `Host` allow-list.
pub const DEFAULT_BIND: &str = "127.0.0.1:8080";

/// Default HTTP path the MCP endpoint is mounted at.
pub const DEFAULT_PATH: &str = "/mcp";

/// Default maximum accepted POST body, in bytes (4 MiB).
pub const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Which transport the server listens on.
///
/// Both are defined by the 2026-07-28 specification. `Stdio` is the local
/// process transport; `Http` is Streamable HTTP.
#[derive(Debug, Clone)]
pub enum Transport {
    /// Serve over stdin/stdout. The protocol owns stdout, so all diagnostics
    /// must go to stderr — [`crate::telemetry`] enforces this.
    Stdio,
    /// Serve over Streamable HTTP.
    Http(HttpConfig),
}

/// Tuning for the Streamable HTTP transport.
///
/// Under spec 2026-07-28 the protocol is stateless: no `Mcp-Session-Id`, no
/// standalone GET stream, no `Last-Event-ID` resumption. The transport applies
/// that automatically for clients that negotiate 2026-07-28 or newer, so a
/// server built on this scaffold can sit behind a plain round-robin load
/// balancer with no session affinity and no shared session store.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Address to bind the listener to.
    pub bind: SocketAddr,
    /// Path the MCP endpoint is mounted at, e.g. `/mcp`.
    pub path: String,
    /// Accepted `Host` values, guarding against DNS rebinding.
    ///
    /// `None` keeps the transport's default (loopback only), which is the right
    /// choice for local servers. A public deployment must set its own
    /// hostnames. `Some(vec![])` disables the check entirely — only safe when
    /// something in front of the server already validates `Host`.
    pub allowed_hosts: Option<Vec<String>>,
    /// Accepted browser `Origin` values, per RFC 6454 `(scheme, host, port)`.
    ///
    /// `None` keeps the default of no `Origin` validation. Set this for any
    /// server reachable from a browser.
    pub allowed_origins: Option<Vec<String>>,
    /// Prefer `application/json` over `text/event-stream` for plain
    /// request/response tools. The transport still falls back to SSE when a
    /// handler emits notifications before its final result, so no message is
    /// lost.
    pub json_response: bool,
    /// Keep per-connection sessions alive for pre-2026-07-28 clients.
    ///
    /// Off by default: sessions are what SEP-2567 removed, and leaving them off
    /// keeps every client stateless. Turn it on only to support older clients
    /// that need resumable streams.
    pub legacy_sessions: bool,
    /// Maximum accepted POST body, in bytes. Oversized payloads get a `413`.
    pub max_request_body_bytes: usize,
    /// Keep-alive ping interval for SSE responses. `None` disables pings.
    pub sse_keep_alive: Option<Duration>,
    /// OAuth 2.1 resource-server authorization.
    ///
    /// `None` leaves the endpoint open, which is fine behind a gateway that
    /// already authenticates callers. When set, the runtime guards the MCP
    /// endpoint with [`crate::auth::RequireAuthLayer`] and publishes the
    /// Protected Resource Metadata document unauthenticated alongside it.
    pub auth: Option<Arc<AuthConfig>>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind: DEFAULT_BIND.parse().expect("DEFAULT_BIND is a valid addr"),
            path: DEFAULT_PATH.to_string(),
            allowed_hosts: None,
            allowed_origins: None,
            json_response: true,
            legacy_sessions: false,
            max_request_body_bytes: DEFAULT_MAX_REQUEST_BODY_BYTES,
            sse_keep_alive: Some(Duration::from_secs(15)),
            auth: None,
        }
    }
}

/// Everything the runtime needs to start a server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Transport to listen on.
    pub transport: Transport,
    /// `tracing-subscriber` filter directive, e.g. `info` or `rusty_mcp=debug`.
    pub log_filter: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            transport: Transport::Stdio,
            log_filter: "info".to_string(),
        }
    }
}

impl ServerConfig {
    /// Config for a stdio server.
    pub fn stdio() -> Self {
        Self {
            transport: Transport::Stdio,
            ..Default::default()
        }
    }

    /// Config for a Streamable HTTP server on `bind`, using defaults elsewhere.
    pub fn http(bind: SocketAddr) -> Self {
        Self {
            transport: Transport::Http(HttpConfig {
                bind,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Override the log filter directive.
    pub fn with_log_filter(mut self, filter: impl Into<String>) -> Self {
        self.log_filter = filter.into();
        self
    }
}
