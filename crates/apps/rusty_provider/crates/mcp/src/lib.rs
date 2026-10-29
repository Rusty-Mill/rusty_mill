//! MCP (Model Context Protocol) support for rusty_provider, built on
//! [`rusty_mcp_server`] and [`rusty_mcp_client`] -- both directions:
//!
//! - **Server**: rusty_provider's own routing exposed as MCP tools
//!   ([`native`]).
//! - **Gateway**: other MCP servers' tools proxied through the same
//!   endpoint ([`gateway`]).
//!
//! [`build`] merges both into one [`Server`]. See `docs/MCP.md` for
//! configuration and how `rp-server` mounts this.

pub mod gateway;
pub mod native;

use std::sync::Arc;

use rp_router::{McpConfig, Router};
use rusty_mcp_server::Server;

pub use gateway::{GatewayError, GatewaySource, McpGateway};

/// Build the combined MCP server: native tools wrapping `router`, plus
/// every configured upstream connected (best-effort -- a failed connection
/// is logged and simply absent from the tool list, not a startup failure).
/// A connection that later drops is reconnected with backoff in the
/// background -- see `gateway`'s module doc.
///
/// The server's blocking handlers drive the router and the upstream clients
/// on the calling task's rusty_tokio runtime, or, called from another
/// executor (rp-server's tokio `main`), on rusty_tokio's fallback runtime,
/// which this starts if the host has not. Serve the result from threads that
/// are not that runtime's workers (`spawn_blocking`, or the transports' own
/// threads).
///
/// # Errors
/// If the fallback runtime cannot start, or the server description is
/// invalid (a bug: the built-in tools are fixed).
pub async fn build(config: &McpConfig, router: Arc<Router>) -> anyhow::Result<Arc<Server>> {
    let rt = match rusty_tokio::runtime::Handle::try_current() {
        Some(rt) => rt,
        None => rusty_tokio::runtime::install_fallback()?,
    };
    let gateway = Arc::new(McpGateway::connect(&config.upstreams, config).await);
    let builder = Server::builder(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
        .instructions(
            "rusty_provider's own LLM routing (chat_completion/list_models/embeddings), \
             plus any tool proxied from a configured MCP upstream, named \
             \"{upstream}/{tool}\".",
        )
        .tool_source(GatewaySource::new(gateway, rt.clone()));
    let server = native::register(builder, &router, &rt)?.build()?;
    Ok(Arc::new(server))
}
