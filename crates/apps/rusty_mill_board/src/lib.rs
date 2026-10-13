//! Mill Board: a capability-gated kanban board for humans and agents.
//!
//! - [`board`]: the service: roles, clock, commit with retry.
//! - [`domain`]: pure cards, columns, capabilities and the transition table.
//! - [`store`]: the storage port and an in-memory adapter.
//! - [`mmdb`]: the durable `rusty_multimodal_db` adapter.

pub mod board;
pub mod domain;
pub mod mmdb;
pub mod store;
