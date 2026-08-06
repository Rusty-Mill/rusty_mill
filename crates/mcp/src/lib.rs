//! MCP (Model Context Protocol) support for rusty_provider, built on
//! [`rusty_mcp`] -- both directions:
//!
//! - **Server**: rusty_provider's own routing exposed as MCP tools
//!   ([`native`]).
//! - **Gateway**: other MCP servers' tools proxied through the same
//!   endpoint ([`gateway`]).
//!
//! [`server::RustyMcpServer`] merges both into one `ServerHandler`. See
//! `docs/MCP.md` for configuration and how `rp-server` mounts this.

pub mod gateway;
pub mod native;
pub mod server;

use std::sync::Arc;

use rp_router::{McpConfig, Router};

pub use gateway::{GatewayError, McpGateway};
pub use native::NativeTools;
pub use server::RustyMcpServer;

/// Build the combined MCP handler: native tools wrapping `router`, plus
/// every configured upstream connected (best-effort -- a failed connection
/// is logged and simply absent from the tool list, not a startup failure).
pub async fn build(config: &McpConfig, router: Arc<Router>) -> Arc<RustyMcpServer> {
    let native = NativeTools::new(router);
    let gateway = Arc::new(McpGateway::connect(&config.upstreams).await);
    Arc::new(RustyMcpServer::new(native, gateway))
}
