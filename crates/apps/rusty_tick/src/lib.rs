//! Spike: a task manager's storage on `rusty_multimodal_db_engine`.
//!
//! Its purpose is to find out which of the gaps listed in
//! Rusty-Mill/rusty_mill#382 actually block a task manager. See
//! `SPIKE-FINDINGS.md`.

pub mod api;
pub mod auth;
pub mod backend;
pub mod dto;
pub mod lists;
pub mod pool;
pub mod server;
pub mod service;
pub mod store;
pub mod task;
pub mod users;

pub use store::{TaskStore, TickError};
pub use task::{Priority, Status, Task};
