//! Mill Board: a capability-gated kanban board for humans and agents.
//!
//! - [`api`]: the operation table shared by MCP and web.
//! - [`board`]: the service: roles, clock, commit with retry.
//! - [`domain`]: pure cards, columns, capabilities and the transition table.
//! - [`store`]: the storage port and an in-memory adapter.
//! - [`mcp`], [`web`]: the two surfaces over [`api`].
//! - [`mmdb`]: the durable `rusty_multimodal_db` adapter.

pub mod api;
pub mod board;
pub mod domain;
pub mod mcp;
pub mod mmdb;
pub mod store;
pub mod web;
