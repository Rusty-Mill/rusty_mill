//! Example MCP server built on the `rusty-mcp` scaffold.
//!
//! Run it over stdio:
//!
//! ```text
//! cargo run -p rusty-mcp-demo
//! ```
//!
//! or over Streamable HTTP:
//!
//! ```text
//! cargo run -p rusty-mcp-demo -- --transport http --bind 127.0.0.1:8080
//! ```
//!
//! `main` is three lines because the scaffold owns argument parsing, transport
//! selection, logging and shutdown. Everything specific to this server lives in
//! [`server`] and [`tools`].

mod server;
mod tools;

use server::DemoServer;

#[tokio::main]
async fn main() -> Result<(), rusty_mcp::ServeError> {
    rusty_mcp::run(|| Ok(DemoServer::new())).await
}
