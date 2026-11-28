//! The sandbox fs/shell MCP server (Let Em Loose #1379, 726-sandbox-design §6).
//!
//! It gives one agent session `read`, `list`, `stat`, `write` (builders only)
//! and `exec` over one directory, its session root. It is for builders that
//! bring no shell of their own (an API-only model); a CLI builder keeps its
//! built-in tools inside the jail.
//!
//! # Containment
//!
//! This crate confines nothing. Its path checks cover the paths named in
//! tool calls, not what an `exec`'d program does:
//!
//! 1. [`path::SessionPath`] refuses absolute paths, `..` and NUL before any
//!    I/O touches the disk.
//! 2. [`FsWorkspace`] resolves every path and refuses one whose real location
//!    is outside the root, which catches a symlink out (to `/proc`, say).
//!
//! [`Role::Critic`] withholds `write` and nothing else. `exec` runs any
//! program with the server's own rights, so a critic's commands can write
//! files, reach outside the root, and leave descendants running after a
//! timeout. Read-only critic execution, filesystem confinement and cleanup
//! of descendants belong to the jail around the process (`lel-jail`:
//! Landlock and the mount namespace, 726-sandbox-design §6-§7).
//!
//! Nothing launches it yet: gateway registration is a later change. That
//! change calls the feature confined only with evidence that the jail
//! enforces read-only critic execution and owns descendant cleanup.
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
