//! rusty_mux's server.
//!
//! The server owns one PTY per block and feeds its output to a server-side
//! `rusty_term_core` grid, so a client that attaches gets a snapshot of the
//! terminal (negotiated replay checkpoint), then a ready frame, then the raw PTY bytes. Every attached client
//! parses those bytes itself; the server never re-renders. Input reaches a
//! PTY only from the client holding that block's lease ([`rusty_mux_core`]).
//!
//! Everything runs on one thread (`rusty_tokio`'s `LocalRuntime`), so the
//! model, the grids and the client queues are plain `Rc<RefCell<_>>` state
//! and every handler sees a consistent view between awaits.
//!
//! Phase 1 is Linux-only; [`serve`] reports `Unsupported` elsewhere.

pub mod checkpoint;
#[cfg(target_os = "linux")]
mod conn;
#[cfg(target_os = "linux")]
mod outbox;
#[cfg(target_os = "linux")]
mod pty;
#[cfg(target_os = "linux")]
mod state;

use std::ffi::OsString;
use std::io;

use rusty_tokio::io::UnixListener;

/// How the server starts each block's program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The program run in every new block, normally the user's shell.
    pub program: OsString,
    pub args: Vec<OsString>,
    /// The working directory of every new block.
    pub cwd: OsString,
}

/// Accepts clients on `listener` until accepting fails. Must run inside a
/// `rusty_tokio` `LocalRuntime`, because connection and PTY tasks are
/// spawned with `spawn_local`.
///
/// Phase 1 has no authentication, so only peers running as this process's
/// effective user are served; any other connection is closed at once. This
/// holds whatever the socket file's permissions are.
#[cfg(target_os = "linux")]
pub async fn serve(listener: UnixListener, config: Config) -> io::Result<()> {
    serve_for(listener, config, rusty_libc::process::geteuid()).await
}

#[cfg(target_os = "linux")]
async fn serve_for(listener: UnixListener, config: Config, owner: u32) -> io::Result<()> {
    let state = state::State::shared(config);
    loop {
        let (stream, _) = listener.accept().await?;
        match stream.peer_cred() {
            Ok(peer) if peer.uid() == owner => {
                rusty_tokio::task::spawn_local(conn::run(state.clone(), stream));
            }
            Ok(peer) => eprintln!("rusty-mux-server: refusing a client of uid {}", peer.uid()),
            Err(err) => eprintln!("rusty-mux-server: reading a client's credentials: {err}"),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;

#[cfg(not(target_os = "linux"))]
pub async fn serve(_listener: UnixListener, _config: Config) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "rusty_mux_server hosts PTYs on Linux only in Phase 1",
    ))
}
