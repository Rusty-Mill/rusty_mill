//! rusty_mux's server.
//!
//! The server owns one PTY per block and feeds its output to a server-side
//! `rusty_term_core` grid, so a client that attaches gets a snapshot of the
//! terminal (negotiated RTCP v6 checkpoint chunks), then a ready frame and
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
mod datagram;
#[cfg(any(target_os = "linux", windows))]
mod echo;
#[cfg(any(target_os = "linux", windows))]
mod history;
#[cfg(any(target_os = "linux", windows))]
mod keys;
#[cfg(any(target_os = "linux", windows))]
mod outbox;
#[cfg(any(target_os = "linux", windows))]
mod process;
#[cfg(any(target_os = "linux", windows))]
mod pty;
#[cfg(any(target_os = "linux", windows))]
mod state;
pub mod style;
#[cfg(any(target_os = "linux", windows))]
mod terminal;

use std::ffi::{OsStr, OsString};
use std::io;
use std::time::Duration;

#[cfg(any(target_os = "linux", windows))]
use rusty_mux_core::{Principal, Transport};
#[cfg(any(target_os = "linux", windows))]
use rusty_tokio::io::UnixStream;
use rusty_tokio::io::{UdpSocket, UnixListener};

/// How the server starts each block's program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The program run in a new block, normally the user's shell, unless
    /// the request names one.
    pub program: OsString,
    pub args: Vec<OsString>,
    /// The working directory of a new block, unless the request names one.
    pub cwd: OsString,
    /// Which console host runs a block's program on Windows; other
    /// platforms ignore it.
    pub console_host: ConsoleHost,
}

/// The Windows console host behind each block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConsoleHost {
    /// Windows' ConPTY.
    #[default]
    ConPty,
    /// Our own host, `rusty_conhost`.
    Own,
}

impl ConsoleHost {
    /// The variable that picks the host: `conpty` (the default when it is
    /// unset) or `own`.
    pub const ENV: &str = "RUSTY_CONSOLE_HOST";

    /// The host [`Self::ENV`] names. Any other value is an error, never a
    /// silent fallback to ConPTY.
    pub fn from_env() -> Result<Self, String> {
        Self::parse(std::env::var_os(Self::ENV).as_deref())
    }

    /// The host `value` names, in any case; `None` is the default.
    pub fn parse(value: Option<&OsStr>) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        match value.to_str().map(str::to_ascii_lowercase).as_deref() {
            Some("conpty") => Ok(Self::ConPty),
            Some("own") => Ok(Self::Own),
            _ => Err(format!(
                "{}={}: not a console host (conpty or own)",
                Self::ENV,
                value.display()
            )),
        }
    }
}

/// The server's UDP side (`--udp`): one socket for every association.
pub struct Udp {
    pub socket: UdpSocket,
    /// How long an association, and the clients on it, outlive silence.
    pub idle: Duration,
}

/// Accepts clients on `listener` until accepting fails, then stops as
/// [`serve_until`] does. Must run inside a `rusty_tokio` `LocalRuntime`,
/// because connection and PTY tasks are spawned with `spawn_local`.
///
/// The server has no authentication, so only peers running as this
/// process's user are served (the effective uid on Linux, the token's user
/// SID on Windows); any other connection is closed at once. This holds
/// whatever the socket file's permissions are.
#[cfg(any(target_os = "linux", windows))]
pub async fn serve(listener: UnixListener, config: Config) -> io::Result<()> {
    serve_for(listener, config, None, peer_check, std::future::pending()).await
}

/// [`serve`], and offers clients `DATAGRAM`: each may then move its
/// connections onto an association on `udp`. Ends when accepting or
/// receiving fails.
#[cfg(any(target_os = "linux", windows))]
pub async fn serve_udp(listener: UnixListener, config: Config, udp: Udp) -> io::Result<()> {
    serve_for(
        listener,
        config,
        Some(udp),
        peer_check,
        std::future::pending(),
    )
    .await
}

/// [`serve`], or [`serve_udp`] when `udp` is given, until `stop` resolves:
/// a clean stop. Either way it ends, every block's program is ended, and
/// every console we host ([`ConsoleHost::Own`]) is hung up; the call
/// returns once each has shut down or 6 s have passed, whichever is first,
/// logging if one had not. A console that has shut down has closed and
/// tried to end each client still attached, 5 s after CTRL_CLOSE; one it
/// could not open or end may still run. Dropping the
/// future instead stops nothing: the tasks it spawned on the runtime keep
/// serving connected clients and their blocks until the runtime itself is
/// dropped, which ends the programs without that wait.
#[cfg(any(target_os = "linux", windows))]
pub async fn serve_until(
    listener: UnixListener,
    config: Config,
    udp: Option<Udp>,
    stop: impl Future<Output = ()>,
) -> io::Result<()> {
    serve_for(listener, config, udp, peer_check, stop).await
}

/// The principal every client of this server has: its own user, the only
/// peer [`peer_check`] lets in.
#[cfg(target_os = "linux")]
fn owner() -> io::Result<Principal> {
    let uid = rusty_libc::process::geteuid();
    Principal::unix(uid, platform_linux::user_name(uid))
        .map_err(|e| io::Error::other(format!("uid {uid}: {e}")))
}

/// The principal every client of this server has: its own user's SID, the
/// only peer [`peer_check`] lets in.
#[cfg(windows)]
fn owner() -> io::Result<Principal> {
    let sid = platform_windows::sys::security::UserSid::current()
        .and_then(|sid| sid.to_sid_string())
        .map_err(|e| io::Error::other(format!("our own SID: {e:?}")))?;
    Principal::new(format!("sid:{sid}"), "")
        .map_err(|e| io::Error::other(format!("SID {sid}: {e}")))
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
    udp: Option<Udp>,
    allowed: impl Fn(&UnixStream) -> io::Result<Peer>,
    stop: impl Future<Output = ()>,
) -> io::Result<()> {
    let socket = listener
        .local_addr()
        .ok()
        .and_then(|addr| addr.as_pathname().map(std::path::absolute))
        .and_then(Result::ok);
    let owner = owner()?;
    let state = state::State::shared(config, socket);
    rusty_tokio::task::spawn_local(state::watch_processes(std::rc::Rc::downgrade(&state)));
    let served = rusty_tokio::select! {
        result = serve_clients(listener, state.clone(), udp, allowed, owner) => result,
        () = stop => Ok(()),
    };
    let hosts = state.borrow_mut().stop();
    if !hosts.shut_down(pty::SHUTDOWN).await {
        eprintln!(
            "rusty-mux-server: a console was still shutting down after {:?}",
            pty::SHUTDOWN
        );
    }
    served
}

/// Serves clients on `listener`, and on `udp` if given, until either fails.
/// Every peer `allowed` lets in is `owner`.
#[cfg(any(target_os = "linux", windows))]
async fn serve_clients(
    listener: UnixListener,
    state: state::Shared,
    udp: Option<Udp>,
    allowed: impl Fn(&UnixStream) -> io::Result<Peer>,
    owner: Principal,
) -> io::Result<()> {
    let Some(udp) = udp else {
        return accept(listener, state, allowed, owner).await;
    };
    let registry = datagram::Registry::new(udp.socket.local_addr()?.port());
    let lifetimes = state::Lifetimes {
        idle: udp.idle,
        lease: datagram::LEASE_SILENCE,
    };
    state
        .borrow_mut()
        .offer_datagrams(registry.clone(), lifetimes);
    let socket = std::rc::Rc::new(udp.socket);
    rusty_tokio::select! {
        result = accept(listener, state.clone(), allowed, owner) => result,
        result = datagram::serve(socket, registry, state, udp.idle, datagram::LEASE_SILENCE) => result,
    }
}

/// Accepts socket clients, each of `owner`: a personal server lets in only
/// its own user.
#[cfg(any(target_os = "linux", windows))]
async fn accept(
    listener: UnixListener,
    state: state::Shared,
    allowed: impl Fn(&UnixStream) -> io::Result<Peer>,
    owner: Principal,
) -> io::Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        match allowed(&stream) {
            Ok(Peer::Allowed) => {
                let (rd, wr) = stream.into_split();
                rusty_tokio::task::spawn_local(conn::run(
                    state.clone(),
                    rd,
                    wr,
                    owner.clone(),
                    Transport::Socket,
                    |_, _| {},
                ));
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
    Err(unsupported())
}

#[cfg(not(any(target_os = "linux", windows)))]
pub async fn serve_udp(_listener: UnixListener, _config: Config, _udp: Udp) -> io::Result<()> {
    Err(unsupported())
}

#[cfg(not(any(target_os = "linux", windows)))]
pub async fn serve_until(
    _listener: UnixListener,
    _config: Config,
    _udp: Option<Udp>,
    _stop: impl Future<Output = ()>,
) -> io::Result<()> {
    Err(unsupported())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "rusty_mux_server hosts PTYs on Linux and Windows only",
    )
}
