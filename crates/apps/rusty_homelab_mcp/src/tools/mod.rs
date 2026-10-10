//! Tool modules, one per backend.
//!
//! Each is an `impl HomelabServer` block of plain async methods, plus a
//! `register_tools` function that offers them on a server (name, description
//! and argument type; the schema comes from the argument type). Adding a new
//! backend (Home Assistant, UniFi, ...) means adding a module here and one
//! more `register_tools` call in [`crate::server::HomelabServer::wire_server`].

pub mod fedora;
pub mod opnsense;
pub mod proxmox;
