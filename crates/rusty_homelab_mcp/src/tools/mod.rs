//! Tool modules, one per backend.
//!
//! Each contributes its own [`rmcp::handler::server::router::tool::ToolRouter`]
//! via `#[tool_router(router = ..., vis = "pub(crate)")]` on an `impl
//! HomelabServer` block; [`crate::server::HomelabServer::new`] composes them
//! with `+`. Adding a new backend (Home Assistant, UniFi, ...) means adding a
//! module here plus one more `+` term there -- nothing else changes.

pub mod opnsense;
pub mod proxmox;
