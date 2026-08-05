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

mod prompts;
mod resources;
mod server;
mod tools;

use std::{sync::Arc, time::Duration};

use clap::Parser as _;
use rusty_mcp::{Cli, ServerConfig};
use server::{DemoServer, DemoState, default_task_support};

/// How long in-flight tasks get to finish before they are aborted.
const DRAIN_GRACE: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() -> Result<(), rusty_mcp::ServeError> {
    // State and tasks are built once and cloned into each handler: Streamable
    // HTTP constructs a fresh handler per request, but tasks must outlive the
    // call that created them.
    let state = Arc::new(DemoState::default());
    let tasks = default_task_support();

    let config: ServerConfig = Cli::parse().into();
    let config = config.with_shutdown_hook({
        let tasks = tasks.clone();
        move || {
            let tasks = tasks.clone();
            Box::pin(async move {
                let abandoned = tasks.drain(DRAIN_GRACE).await;
                if abandoned > 0 {
                    tracing::warn!(
                        abandoned,
                        "aborted tasks that were still running at shutdown"
                    );
                }
            })
        }
    });

    rusty_mcp::telemetry::init(&config.log_filter);

    rusty_mcp::serve(
        move || {
            Ok(DemoServer::with_state_and_tasks(
                Arc::clone(&state),
                tasks.clone(),
            ))
        },
        config,
    )
    .await
}
