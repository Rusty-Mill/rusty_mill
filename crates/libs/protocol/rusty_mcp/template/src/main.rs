//! {{description}}
//!
//! Run it over stdio — what a desktop client launches:
//!
//! ```text
//! cargo run
//! ```
//!
//! or over Streamable HTTP:
//!
//! ```text
//! cargo run -- --transport http --bind 127.0.0.1:8080
//! ```
//!
//! `main` is short because the scaffold owns argument parsing, transport
//! selection, logging and graceful shutdown. What is left is yours: the
//! handler in [`server`], and the tools it routes to.

mod server;

use server::Server;

#[tokio::main]
async fn main() -> Result<(), rusty_mcp::ServeError> {
    // State that must survive across calls goes here, cloned into each handler
    // — Streamable HTTP builds a fresh handler per *request*, so anything
    // constructed inside the closure would not be shared.
    rusty_mcp::run(|| Ok(Server::new())).await
}
