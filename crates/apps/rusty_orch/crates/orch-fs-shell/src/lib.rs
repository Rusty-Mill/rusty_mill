//! The sandbox fs/shell MCP server (Let Em Loose #1379, 726-sandbox-design §6).
//!
//! It gives one agent session `read`, `list`, `stat`, `write` (builders only)
//! and `exec` over one directory, its session root. It is for builders that
//! bring no shell of their own (an API-only model); a CLI builder keeps its
//! built-in tools inside the jail.
//!
//! # Containment
//!
//! The gateway runs it as a `per_session` stdio target, inside the same
//! `lel-jail` and `jail.json` the session would get, so there is one
//! containment path: Landlock and the mount namespace are the boundary.
//! This crate adds two checks in front of them, as defence in depth:
//!
//! 1. [`path::SessionPath`] refuses absolute paths, `..` and NUL before any
//!    I/O touches the disk.
//! 2. [`FsWorkspace`] resolves every path and refuses one whose real location
//!    is outside the root, which catches a symlink out (to `/proc`, say).
//!
//! # Shape
//!
//! [`core`] is pure: it parses a call, applies the role and the caps, and
//! shapes the answer. [`Workspace`] is the port to the disk and processes;
//! [`FsWorkspace`] is its adapter. [`server`] wires both into an MCP server.

pub mod core;
mod fs;
pub mod path;
pub mod server;

pub use crate::core::{Entry, ExecOutcome, ExecSpec, Kind, Role, Workspace};
pub use crate::fs::FsWorkspace;
