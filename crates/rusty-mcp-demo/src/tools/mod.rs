//! Tool implementations, one module per topic.
//!
//! Each module adds an `impl DemoServer` block annotated with
//! `#[tool_router(router = ..., vis = "pub(crate)")]`, which generates a router
//! function that [`crate::server::DemoServer::with_state`] merges. Tools stay
//! grouped by topic without a central registry to keep in sync.

pub mod calculator;
pub mod notify;
pub mod slow;
pub mod text;
