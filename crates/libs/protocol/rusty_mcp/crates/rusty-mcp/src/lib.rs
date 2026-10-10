//! First-party building blocks for MCP servers and gateways: the
//! transport-level layers that sit in front of an MCP endpoint.
//!
//! - [`auth`]: OAuth 2.1 resource-server authorization (RFC 9728 metadata,
//!   `WWW-Authenticate` challenges, audience-bound bearer tokens; JWT
//!   validation with the `jwt` feature).
//! - [`limits`]: bounded concurrency and a response timeout that shed load
//!   with `503` instead of queueing it.
//! - [`trace`]: strict W3C trace-context parsing over MCP `_meta` (SEP-414).
//! - [`otel`] (feature `otel`): an OTLP trace and metrics pipeline and a
//!   per-request metrics layer.
//!
//! The server and client themselves are `rusty_mcp_server` and
//! `rusty-mcp-client`; this crate no longer depends on `rmcp` (ADR-0002,
//! Amendment 1). The earlier `rmcp`-based server scaffold (`run`, `serve`,
//! CLI and config, resources, tasks, subscriptions) was removed once no
//! consumer used it; it is in git history before that change.

#![doc(html_root_url = "https://docs.rs/rusty-mcp/0.6.0")]

pub mod auth;
pub mod limits;
#[cfg(feature = "otel")]
pub mod otel;
pub mod trace;
