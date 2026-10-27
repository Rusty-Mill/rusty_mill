//! rusty_mux's server.
//!
//! The server owns one PTY per block and feeds its output to a server-side
//! `rusty_term_core` grid, so a client that attaches gets a snapshot of the
//! terminal (negotiated RTCP v3 checkpoint chunks), then a ready frame and
//! raw PTY bytes. Every attached client
//! parses those bytes itself; the server never re-renders. Input reaches a
//! PTY only from the client holding that block's lease ([`rusty_mux_core`]).
//!
//! Every model change is broadcast as an event. With `EVENT_SEQ` events are
//! numbered, and a client that reconnects with the last number it saw gets
//! the events it missed, if the server still holds them (the newest 4096).
//!
//! Everything runs on one thread (`rusty_tokio`'s `LocalRuntime`), so the
//! model, the grids and the client queues are plain `Rc<RefCell<_>>` state
//! and every handler sees a consistent view between awaits.
//!
//! The server runs on Linux (PTYs) and Windows (ConPTY over AF_UNIX);
//! [`serve`] reports `Unsupported` elsewhere.

pub mod checkpoint;
#[cfg(any(target_os = "linux", windows))]
mod conn;
#[cfg(any(target_os = "linux", windows))]
mod history;
#[cfg(any(target_os = "linux", windows))]
mod outbox;
#[cfg(any(target_os = "linux", windows))]
mod pty;
#[cfg(any(target_os = "linux", windows))]
mod state;
#[cfg(any(target_os = "linux", windows))]
mod terminal;

use std::ffi::OsString;
use std::io;

use rusty_tokio::io::UnixListener;
#[cfg(any(target_os = "linux", windows))]
use rusty_tokio::io::UnixStream;

/// How the server starts each block's program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The program run in a new block, normally the user's shell, unless
    /// the request names one.
    pub program: OsString,
    pub args: Vec<OsString>,
    /// The working directory of a new block, unless the request names one.
    pub cwd: OsString,
}

/// Accepts clients on `listener` until accepting fails. Must run inside a
/// `rusty_tokio` `LocalRuntime`, because connection and PTY tasks are
/// spawned with `spawn_local`.
///
/// The server has no authentication, so only peers running as this
/// process's user are served (the effective uid on Linux, the token's user
/// SID on Windows); any other connection is closed at once. This holds
/// whatever the socket file's permissions are.
#[cfg(any(target_os = "linux", windows))]
pub async fn serve(listener: UnixListener, config: Config) -> io::Result<()> {
    serve_for(listener, config, peer_check).await
}

/// Whether a connection's peer may be served.
#[cfg(any(target_os = "linux", windows))]
pub(crate) enum Peer {
    Allowed,
    /// Who was refused, for the log.
    Refused(String),
}

#[cfg(target_os = "linux")]
fn peer_check(stream: &UnixStream) -> io::Result<Peer> {
    owned_by(stream, rusty_libc::process::geteuid())
}

#[cfg(target_os = "linux")]
fn owned_by(stream: &UnixStream, owner: u32) -> io::Result<Peer> {
    let uid = stream.peer_cred()?.uid();
    Ok(if uid == owner {
        Peer::Allowed
    } else {
        Peer::Refused(format!("uid {uid}"))
    })
}

/// The peer's process, then that process's token user, compared with ours.
#[cfg(windows)]
fn peer_check(stream: &UnixStream) -> io::Result<Peer> {
    let pid = stream.peer_pid()?;
    let mine = platform_windows::is_current_user(pid)
        .map_err(|e| io::Error::other(format!("pid {pid}: {e:?}")))?;
    Ok(if mine {
        Peer::Allowed
    } else {
        Peer::Refused(format!("pid {pid}"))
    })
}

#[cfg(any(target_os = "linux", windows))]
async fn serve_for(
    listener: UnixListener,
    config: Config,
    allowed: impl Fn(&UnixStream) -> io::Result<Peer>,
) -> io::Result<()> {
    let socket = listener
        .local_addr()
        .ok()
        .and_then(|addr| addr.as_pathname().map(std::path::absolute))
        .and_then(Result::ok);
    let state = state::State::shared(config, socket);
    loop {
        let (stream, _) = listener.accept().await?;
        match allowed(&stream) {
            Ok(Peer::Allowed) => {
                rusty_tokio::task::spawn_local(conn::run(state.clone(), stream));
            }
            Ok(Peer::Refused(who)) => eprintln!("rusty-mux-server: refusing a client of {who}"),
            Err(err) => eprintln!("rusty-mux-server: reading a client's credentials: {err}"),
        }
    }
}

#[cfg(all(test, any(target_os = "linux", windows)))]
mod tests;

#[cfg(not(any(target_os = "linux", windows)))]
pub async fn serve(_listener: UnixListener, _config: Config) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "rusty_mux_server hosts PTYs on Linux and Windows only",
    ))
}
