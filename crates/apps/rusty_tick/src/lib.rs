//! A self-hosted task manager on `rusty_multimodal_db_engine`: storage,
//! rules, a JSON HTTP API and a static file server for the web UI in `web/`.
//! `SPIKE-FINDINGS.md` records which engine gaps mattered.

pub mod admin;
pub mod api;
pub mod assistant;
pub mod auth;
pub mod backend;
pub mod docs;
pub mod dto;
pub mod lists;
pub mod pool;
pub mod server;
pub mod service;
pub mod static_files;
pub mod store;
pub mod table;
pub mod tags;
pub mod task;
pub mod users;

pub use store::{TaskStore, TickError};
pub use task::{ChecklistItem, Priority, Status, Task, TaskKind};
