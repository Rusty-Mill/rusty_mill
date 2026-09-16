//! `sessionmgr tui`: a grid of session panes over the daemon's public
//! socket.
//!
//! Depends on `sessionmgr-protocol` only -- never `sessionmgr-proc` or
//! `sessionmgr-agents` -- per PLAN.md's ports-and-adapters boundary: a UI
//! that cannot name a process type cannot accidentally spawn one. Every
//! byte this crate renders arrives over the socket from `client::Attached`;
//! nothing here ever touches a PTY, a pid, or `git` directly.

pub mod app;
pub mod client;
pub mod error;
pub mod grid;
pub mod panes;
mod terminal;

use std::path::PathBuf;

pub use error::{Error, Result};

/// Runs the TUI against an already-running daemon at `socket`.
///
/// Takes the daemon's socket path directly rather than a state root: the
/// composition root (`sessionmgr-daemon`'s CLI dispatch) already knows
/// how to resolve a root to a socket path and how to auto-start a daemon
/// -- both are `sessionmgr-proc`-adjacent concerns this crate must not
/// depend on to do (see the crate docs).
pub async fn run(socket: PathBuf) -> Result<()> {
    let mut term = terminal::enter()?;
    let (mut app, mut session_rx) = app::App::new(socket);
    let outcome = app.run(&mut term, &mut session_rx).await;
    terminal::leave(term)?;
    outcome
}
