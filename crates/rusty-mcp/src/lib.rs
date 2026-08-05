//! A reusable scaffold for building Model Context Protocol servers in Rust.
//!
//! This crate owns the parts every MCP server needs and nobody wants to write
//! twice: argument parsing, transport selection, stderr logging, graceful
//! shutdown. You supply the tools.
//!
//! It targets MCP [spec 2026-07-28] on top of [`rmcp`] 3.x. That revision made
//! the protocol stateless — no `initialize` handshake, no `Mcp-Session-Id`, no
//! stream resumption — so an HTTP server built here scales horizontally behind
//! a plain load balancer with no session affinity.
//!
//! # Example
//!
//! ```no_run
//! use rmcp::{ServerHandler, handler::server::{router::tool::ToolRouter, wrapper::Parameters}};
//! use rmcp::{tool, tool_handler, tool_router};
//! use schemars::JsonSchema;
//! use serde::Deserialize;
//!
//! #[derive(Debug, Deserialize, JsonSchema)]
//! struct AddArgs {
//!     /// Left operand.
//!     a: i64,
//!     /// Right operand.
//!     b: i64,
//! }
//!
//! #[derive(Clone)]
//! struct Calculator {
//!     tool_router: ToolRouter<Self>,
//! }
//!
//! #[tool_router(router = tool_router)]
//! impl Calculator {
//!     fn new() -> Self {
//!         Self { tool_router: Self::tool_router() }
//!     }
//!
//!     #[tool(description = "Add two integers.")]
//!     async fn add(&self, Parameters(AddArgs { a, b }): Parameters<AddArgs>) -> String {
//!         (a + b).to_string()
//!     }
//! }
//!
//! #[tool_handler(router = self.tool_router)]
//! impl ServerHandler for Calculator {}
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     rusty_mcp::run(|| Ok(Calculator::new())).await?;
//!     Ok(())
//! }
//! ```
//!
//! That binary speaks stdio by default and Streamable HTTP with
//! `--transport http`.
//!
//! [spec 2026-07-28]: https://modelcontextprotocol.io/specification/2026-07-28

#![doc(html_root_url = "https://docs.rs/rusty-mcp/0.1.0")]

pub mod cli;
pub mod config;
pub mod error;
pub mod runtime;
pub mod shutdown;
pub mod telemetry;

pub use cli::{Cli, TransportArg};
pub use config::{HttpConfig, ServerConfig, Transport};
pub use error::{ServeError, ToolError};
pub use runtime::{HandlerFactory, serve};

pub use rmcp::model::{ProtocolVersion, ServerCapabilities, ServerInfo};

/// The MCP revision this scaffold targets.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2026_07_28;

/// Build a [`ServerInfo`] pinned to [`PROTOCOL_VERSION`].
///
/// Worth using rather than [`ServerInfo::new`]: as of `rmcp` 3.1,
/// `ProtocolVersion::LATEST` still points at `2025-11-25`, so a server that
/// takes the default advertises the *older* revision even though it can speak
/// 2026-07-28 and will happily negotiate it. Pinning the version here is what
/// gets you the stateless semantics, `resultType`, and the `ttlMs`/`cacheScope`
/// cache hints that 2026-07-28 requires on list results.
///
/// Clients that ask for an older revision still negotiate down as usual.
///
/// ```
/// use rmcp::model::ServerCapabilities;
///
/// let info = rusty_mcp::server_info(
///     "my-server",
///     "1.0.0",
///     ServerCapabilities::builder().enable_tools().build(),
/// );
/// assert_eq!(info.protocol_version, rusty_mcp::PROTOCOL_VERSION);
/// ```
pub fn server_info(
    name: impl Into<String>,
    version: impl Into<String>,
    capabilities: ServerCapabilities,
) -> ServerInfo {
    ServerInfo::new(capabilities)
        .with_server_info(rmcp::model::Implementation::new(
            name.into(),
            version.into(),
        ))
        .with_protocol_version(PROTOCOL_VERSION)
}

/// Parse the standard CLI, start logging, and serve — the whole `main` body.
///
/// Use [`serve`] directly if you parse your own arguments.
pub async fn run<S, F>(factory: F) -> Result<(), ServeError>
where
    S: rmcp::ServerHandler + Send + 'static,
    F: HandlerFactory<S>,
{
    use clap::Parser as _;

    let config: ServerConfig = Cli::parse().into();
    telemetry::init(&config.log_filter);
    serve(factory, config).await
}
