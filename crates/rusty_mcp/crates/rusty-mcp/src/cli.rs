//! Ready-made command line interface.
//!
//! A server built on this scaffold gets `--transport stdio|http`, `--bind`,
//! `--path`, host/origin allow-lists and log control without writing any
//! argument parsing. Flatten [`Cli`] into your own `clap` struct if you need
//! extra flags alongside these.

use std::{net::SocketAddr, time::Duration};

use clap::{Parser, ValueEnum};

use crate::{
    config::{
        DEFAULT_BIND, DEFAULT_MAX_REQUEST_BODY_BYTES, DEFAULT_PATH, HttpConfig, ServerConfig,
        Transport,
    },
    limits::LimitsLayer,
};

/// Which transport to serve on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TransportArg {
    /// stdin/stdout, for locally launched servers.
    Stdio,
    /// Streamable HTTP.
    Http,
}

/// Standard MCP server arguments.
#[derive(Debug, Clone, Parser)]
#[command(about = "A Model Context Protocol server", version)]
pub struct Cli {
    /// Transport to serve on.
    #[arg(long, value_enum, default_value = "stdio", env = "MCP_TRANSPORT")]
    pub transport: TransportArg,

    /// Address to bind (HTTP only).
    #[arg(long, default_value = DEFAULT_BIND, env = "MCP_BIND")]
    pub bind: SocketAddr,

    /// Path to mount the MCP endpoint at (HTTP only).
    #[arg(long, default_value = DEFAULT_PATH, env = "MCP_PATH")]
    pub path: String,

    /// Accepted `Host` values (HTTP only, repeatable).
    ///
    /// Defaults to loopback only. Set this for any non-local deployment.
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

    /// Reply with SSE rather than preferring plain JSON (HTTP only).
    #[arg(long, env = "MCP_SSE_RESPONSE")]
    pub sse_response: bool,

    /// Keep sessions alive for pre-2026-07-28 clients (HTTP only).
    ///
    /// Off by default. 2026-07-28 clients are served statelessly regardless.
    #[arg(long, env = "MCP_LEGACY_SESSIONS")]
    pub legacy_sessions: bool,

    /// Maximum accepted request body, in bytes (HTTP only).
    #[arg(long, default_value_t = DEFAULT_MAX_REQUEST_BODY_BYTES, env = "MCP_MAX_BODY_BYTES")]
    pub max_body_bytes: usize,

    /// SSE keep-alive interval in seconds; `0` disables it (HTTP only).
    #[arg(long, default_value_t = 15, env = "MCP_SSE_KEEP_ALIVE_SECS")]
    pub sse_keep_alive_secs: u64,

    /// Maximum requests in flight; `0` means unlimited (HTTP only).
    ///
    /// Over the limit, requests are shed with `503` rather than queued.
    #[arg(long, default_value_t = 0, env = "MCP_MAX_CONCURRENT")]
    pub max_concurrent: usize,

    /// Give up on a request after this many seconds; `0` disables it (HTTP
    /// only).
    ///
    /// Bounds time to produce a response, not time to stream one, so a
    /// long-lived `subscriptions/listen` is unaffected.
    #[arg(long, default_value_t = 0, env = "MCP_REQUEST_TIMEOUT_SECS")]
    pub request_timeout_secs: u64,

    /// Log filter directive. `RUST_LOG` overrides this when set.
    #[arg(long, default_value = "info", env = "MCP_LOG")]
    pub log: String,
}

impl Cli {
    /// Turn parsed arguments into a [`ServerConfig`].
    pub fn into_config(self) -> ServerConfig {
        let transport = match self.transport {
            TransportArg::Stdio => Transport::Stdio,
            TransportArg::Http => Transport::Http(HttpConfig {
                bind: self.bind,
                path: self.path,
                allowed_hosts: self.allowed_hosts,
                allowed_origins: self.allowed_origins,
                json_response: !self.sse_response,
                legacy_sessions: self.legacy_sessions,
                max_request_body_bytes: self.max_body_bytes,
                sse_keep_alive: (self.sse_keep_alive_secs > 0)
                    .then(|| Duration::from_secs(self.sse_keep_alive_secs)),
                // Authorization needs a `TokenValidator`, which cannot come
                // from a flag. Set it on the `HttpConfig` in code.
                auth: None,
                // Metrics need instruments from a live pipeline, so this is
                // set in code too — see `otel::OtelGuard::instruments`.
                #[cfg(feature = "otel")]
                metrics: None,
                limits: (self.max_concurrent > 0 || self.request_timeout_secs > 0).then(|| {
                    let mut limits = LimitsLayer::new();
                    if self.max_concurrent > 0 {
                        limits = limits.with_max_concurrent(self.max_concurrent);
                    }
                    if self.request_timeout_secs > 0 {
                        limits =
                            limits.with_timeout(Duration::from_secs(self.request_timeout_secs));
                    }
                    limits
                }),
            }),
        };

        ServerConfig {
            transport,
            log_filter: self.log,
            // Cleanup needs owned handles, so it is attached in code rather
            // than derived from flags.
            shutdown_hook: None,
        }
    }
}

impl From<Cli> for ServerConfig {
    fn from(cli: Cli) -> Self {
        cli.into_config()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn defaults_to_stdio() {
        let cli = Cli::try_parse_from(["server"]).expect("parses");
        assert!(matches!(cli.into_config().transport, Transport::Stdio));
    }

    #[test]
    fn http_config_maps_flags() {
        let cli = Cli::try_parse_from([
            "server",
            "--transport",
            "http",
            "--bind",
            "0.0.0.0:9000",
            "--path",
            "/api/mcp",
            "--allowed-host",
            "example.com,example.com:9000",
            "--sse-keep-alive-secs",
            "0",
        ])
        .expect("parses");

        let Transport::Http(http) = cli.into_config().transport else {
            panic!("expected http transport");
        };
        assert_eq!(http.bind.to_string(), "0.0.0.0:9000");
        assert_eq!(http.path, "/api/mcp");
        assert_eq!(
            http.allowed_hosts.as_deref(),
            Some(["example.com".to_string(), "example.com:9000".to_string()].as_slice())
        );
        // json_response is the default; --sse-response flips it.
        assert!(http.json_response);
        assert!(!http.legacy_sessions);
        assert_eq!(http.sse_keep_alive, None);
    }
}
