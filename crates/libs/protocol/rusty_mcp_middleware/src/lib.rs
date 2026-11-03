//! First-party building blocks for MCP servers and gateways: the
//! transport-level checks that sit in front of an MCP endpoint.
//!
//! - [`auth`]: OAuth 2.1 resource-server authorization (RFC 9728 metadata,
//!   `WWW-Authenticate` challenges, audience-bound bearer tokens; JWT
//!   validation with the `jwt` feature).
//! - [`trace`]: strict W3C trace-context parsing over MCP `_meta` (SEP-414).
//! - [`otel`] (feature `otel`): an OTLP trace and metrics pipeline and a
//!   per-request metrics recorder.
//! - [`serve`] (feature `serve`): auth and load shedding in front of a
//!   blocking `rusty_serve` handler.
//!
//! Nothing here depends on a server framework. The tower layers were removed
//! in #886, once the gateway served on `rusty_axum`, which sheds load itself
//! (`rusty_axum::Admission`).
//!
//! The server and client themselves are `rusty_mcp_server` and
//! `rusty-mcp-client`; this crate no longer depends on `rmcp` (ADR-0002,
//! Amendment 1). The earlier `rmcp`-based server scaffold (`run`, `serve`,
//! CLI and config, resources, tasks, subscriptions) was removed once no
//! consumer used it; it is in git history before that change.

#![doc(html_root_url = "https://docs.rs/rusty_mcp_middleware/0.13.0")]

#[cfg(feature = "serve")]
pub(crate) mod admission;
pub mod auth;
#[cfg(feature = "otel")]
pub mod otel;
#[cfg(feature = "serve")]
pub mod serve;
pub mod trace;
