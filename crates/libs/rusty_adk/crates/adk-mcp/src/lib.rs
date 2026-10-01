//! Model Context Protocol transports for the Rust ADK.
//!
//! ADK has no language-neutral wire protocol for tools, but every ADK SDK can
//! consume an **MCP** server. That makes MCP the interoperability path: serve
//! Rust tools with [`McpServer`] and an ADK agent in Python, Go, TypeScript,
//! Java, or Kotlin can call them; consume someone else's server with
//! [`McpToolset`] and a Rust agent gains their tools.
//!
//! # Serving Rust tools
//!
//! ```no_run
//! # use adk_core::{Schema, Services};
//! # use adk_mcp::{serve_stdio, McpServer};
//! # use adk_sessions::InMemorySessionService;
//! # use adk_tools::FunctionTool;
//! # use std::sync::Arc;
//! # #[tokio::main]
//! # async fn main() -> adk_core::Result<()> {
//! let weather = FunctionTool::new(
//!     "get_weather",
//!     "Retrieves the current weather for a city.",
//!     Schema::object().property("city", Schema::string()),
//!     |args, _ctx| {
//!         Box::pin(async move {
//!             let city = args.get("city").and_then(|v| v.as_str()).unwrap_or("?");
//!             Ok(adk_tools::success(serde_json::json!({ "report": format!("Sunny in {city}") })))
//!         })
//!     },
//! );
//!
//! let services = Services::new(Arc::new(InMemorySessionService::new()));
//! let server = McpServer::new("rust-weather", vec![weather.shared()], services);
//! serve_stdio(&server).await
//! # }
//! ```
//!
//! A Python ADK agent then reaches it with the standard `McpToolset`:
//!
//! ```python
//! McpToolset(connection_params=StdioConnectionParams(
//!     server_params=StdioServerParameters(command="./rust-weather-server", args=[]),
//! ))
//! ```

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod protocol;
pub mod server;

#[cfg(any(feature = "stdio", feature = "client"))]
mod line_cap;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "http")]
pub mod http;
#[cfg(feature = "stdio")]
pub mod stdio;

pub use protocol::PROTOCOL_VERSION;
pub use server::{serve_tools, McpServer};

#[cfg(feature = "client")]
pub use client::{BoundMcpToolset, ConnectionParams, McpToolset};
#[cfg(feature = "http")]
pub use http::{router, serve_http};
#[cfg(feature = "stdio")]
pub use stdio::{serve_stdio, serve_stream};
