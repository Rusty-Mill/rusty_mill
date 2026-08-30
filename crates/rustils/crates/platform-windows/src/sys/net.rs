//! Raw TCP and Unix domain socket primitives over Winsock (RFC v2 R5+,
//! D16; the Unix domain slice is a D16 follow-on riding the same
//! Winsock plumbing).
//!
//! Winsock needs one-time process-lifetime initialization
//! (`WSAStartup`) before any other call in this module — `ensure_wsa_started`
//! does that lazily, once, via [`std::sync::Once`]. There is no matching
//! `WSACleanup`: the OS tears down every socket and the Winsock DLL's
//! state at process exit regardless, the same pragmatic choice std's own
//! networking and the wider Windows-Rust ecosystem (mio, tokio) make —
//! a `WSACleanup` racing in-flight sockets on other threads at shutdown
//! is a real hazard `WSAStartup`-once-and-never-clean is not.
//!
//! `AF_UNIX` `bind`'s `AddrInUse` doesn't distinguish a path a live
//! listener holds from one a dead listener left behind — Winsock's
//! `bind` can't tell the two apart any more than `bind(2)` can on Unix.
//! `unix_listen` resolves that itself with a throwaway probe `connect`
//! (`is_stale_socket`, below): `WSAECONNREFUSED` means nothing is
//! listening (stale), so the leftover file is deleted and the bind
//! retried exactly once; a successful connect means a live listener
//! owns the path, left untouched. Mirrors the Linux backend's
//! `sys::net::is_stale_socket` — same reasoning, same one-probe/
//! one-retry shape, `DeleteFileW` in place of `unlinkat`.
//!
//! `unix_listen`'s stale-cleanup retry is also reached by one failure
//! *other* than literal `WSAEADDRINUSE` — see [`is_stale_bind_candidate`]
//! for the specific dead-listener race that motivates it, and
//! `docs/decision-request-af-unix-stale-reclaim-race.md` for the full
//! writeup.

#![allow(unsafe_code)]

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use crate::ffi::win32_surface as w;
#[cfg(feature = "track-w")]
use crate::sys::errmap;
use crate::util::wide::to_wide_nul;

fn ensure_wsa_started() {
    static START: Once = Once::new();
    START.call_once(|| {
        // SAFETY: `WSADATA` is a plain-old-data struct for which
        // all-zeroes is a valid (if meaningless) value; `WSAStartup`
        // overwrites it on success.
        let mut data: w::WSADATA = unsafe { std::mem::zeroed() };
        // Track W (D-15): `rusty_win32::net::startup`, which requests the
        // same Winsock 2.2 and owns its own `WSADATA` — this arm's
        // `data` local is therefore unused there, kept declared rather
        // than cfg-split because zeroing a POD struct costs nothing and
        // splitting a `Once` body twice for it would obscure the one
        // thing this function is about.
        #[cfg(feature = "track-w")]
        let r = {
            let _ = &mut data;
            match rusty_win32::net::startup() {
                Ok(()) => 0,
                Err(e) => e.code() as i32,
            }
        };
        // SAFETY: `data` is a valid out-pointer; `0x0202` requests
        // Winsock 2.2, the only version this module's calls target.
        #[cfg(not(feature = "track-w"))]
        let r = unsafe { w::WSAStartup(0x0202, &mut data) };
        // A `WSAStartup` failure here is unrecoverable for this whole
        // module (every socket call needs it); the same "this really
        // shouldn't happen, and there is no sane fallback" territory
        // `platform-windows` treats as a panic elsewhere it can't
        // thread a `Result` through initialization state.
        assert_eq!(r, 0, "WSAStartup failed with error {r}");
    });
}

/// Winsock's own error space classified into a portable [`ErrorKind`].
///
/// Shared by both Track W arms (D-15): the windows-sys arm reads the code
/// from `WSAGetLastError`, the track-w arm takes it from the `Win32Error`
/// the donor already captured — but the *classification* is this one
/// table either way, which is what keeps `PlatformError` bit-identical
/// across the two configurations.
fn wsa_kind(code: i32) -> ErrorKind {
    match code {
        w::WSAECONNREFUSED => ErrorKind::ConnectionRefused,
        w::WSAECONNRESET => ErrorKind::ConnectionReset,
        w::WSAECONNABORTED => ErrorKind::ConnectionAborted,
        w::WSAENOTCONN => ErrorKind::NotConnected,
        w::WSAEADDRINUSE => ErrorKind::AddrInUse,
        w::WSAEADDRNOTAVAIL => ErrorKind::AddrNotAvailable,
        w::WSAETIMEDOUT => ErrorKind::TimedOut,
        w::WSAEACCES => ErrorKind::PermissionDenied,
        w::WSAEINVAL => ErrorKind::InvalidInput,
        w::WSAEWOULDBLOCK => ErrorKind::WouldBlock,
        w::WSAEINTR => ErrorKind::Interrupted,
        _ => ErrorKind::Other,
    }
}

/// Error from the calling thread's last Winsock error code.
fn wsa_err(op: &'static str) -> PlatformError {
    // SAFETY: `WSAGetLastError` takes no arguments and has no
    // preconditions.
    let code = unsafe { w::WSAGetLastError() };
    PlatformError::new(wsa_kind(code), OsCode::Win32(code as u32), op)
}

/// Track W error path for Winsock (D-15). The donor's socket wrappers
/// call `WSAGetLastError` themselves at the only instant it is valid and
/// hand the code back inside a `Win32Error` — the same lesson note 003
/// records for `GetLastError`, applied to Winsock's parallel slot. Note
/// this cannot use `errmap::trackw_err`: that classifies through the
/// *Win32* table, and a Winsock code (10035, 10061, …) means nothing
/// there. Same number-space discipline `errmap`'s own module doc insists
/// on, one space further out.
#[cfg(feature = "track-w")]
fn trackw_wsa_err(op: &'static str, e: rusty_win32::error::Win32Error) -> PlatformError {
    let code = e.code() as i32;
    PlatformError::new(wsa_kind(code), OsCode::Win32(code as u32), op)
}

/// An owned Winsock `SOCKET`, closed on drop.
pub struct OwnedSocket(w::SOCKET);

impl Drop for OwnedSocket {
    /// Track W (D-15): `rusty_win32::net::close_socket`, whose `Result`
    /// is discarded for the same reason the windows-sys arm ignores
    /// `closesocket`'s return — a destructor has nowhere to report to.
    #[cfg(feature = "track-w")]
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid, owned socket not used again after
        // this call — `close_socket`'s whole contract.
        let _ = unsafe { rusty_win32::net::close_socket(self.0) };
    }

    #[cfg(not(feature = "track-w"))]
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid, owned socket not used again after
        // this call.
        unsafe {
            w::closesocket(self.0);
        }
    }
}

impl OwnedSocket {
    /// The raw Winsock `SOCKET` value. `pub(crate)` (rustils#59):
    /// `net.rs`'s `AsRawSocket` impls delegate here, the same "expose
    /// the raw handle, keep ownership private" shape `AsRawFd` gives
    /// the fd backends.
    pub(crate) fn raw(&self) -> w::SOCKET {
        self.0
    }
}

/// Pack a [`SocketAddr`] into a `SOCKADDR_IN`/`SOCKADDR_IN6`-shaped
/// byte buffer and its length — the pair `connect`/`bind` want. A plain
/// byte buffer (not a `sockaddr_storage`-equivalent union type — Winsock
/// has no single admitted one here) sized to the larger variant.
// Under `track-w` the donor owns the sockaddr encoding entirely
// (`to_donor_addr`/`from_donor_addr` above), so this arm's hand-rolled
// conversion is genuinely unreachable rather than merely unused.
#[cfg(not(feature = "track-w"))]
fn to_sockaddr(addr: SocketAddr) -> ([u8; 28], i32) {
    let mut buf = [0u8; 28];
    let len = match addr {
        SocketAddr::V4(v4) => {
            let sin = w::SOCKADDR_IN {
                sin_family: w::AF_INET,
                sin_port: v4.port().to_be(),
                sin_addr: w::IN_ADDR {
                    S_un: w::IN_ADDR_0 {
                        // Same reasoning as the Linux backend's
                        // `to_sockaddr`: `from_ne_bytes` reproduces the
                        // exact in-memory byte pattern the octets are,
                        // on any host — not a byte-order conversion.
                        S_addr: u32::from_ne_bytes(v4.ip().octets()),
                    },
                },
                sin_zero: [0; 8],
            };
            // SAFETY: `buf` is at least `size_of::<SOCKADDR_IN>()` bytes
            // (28 covers it, checked by the `debug_assert!` below);
            // writing a `SOCKADDR_IN` into its start and later reading
            // it back that way is exactly how the API pair on either
            // side of this buffer interprets it.
            unsafe {
                debug_assert!(buf.len() >= std::mem::size_of::<w::SOCKADDR_IN>());
                std::ptr::write(buf.as_mut_ptr().cast::<w::SOCKADDR_IN>(), sin);
            }
            std::mem::size_of::<w::SOCKADDR_IN>()
        }
        SocketAddr::V6(v6) => {
            let sin6 = w::SOCKADDR_IN6 {
                sin6_family: w::AF_INET6,
                sin6_port: v6.port().to_be(),
                sin6_flowinfo: v6.flowinfo(),
                sin6_addr: w::IN6_ADDR {
                    u: w::IN6_ADDR_0 {
                        Byte: v6.ip().octets(),
                    },
                },
                // `Anonymous.sin6_scope_id` is the only member this
                // backend ever writes or reads back (`from_sockaddr`);
                // the union's other view (`sin6_scope_struct`) is never
                // touched, so writing this one is fully initializing.
                Anonymous: w::SOCKADDR_IN6_0 {
                    sin6_scope_id: v6.scope_id(),
                },
            };
            // SAFETY: see the V4 arm above; `SOCKADDR_IN6` is also
            // within `buf`'s 28 bytes.
            unsafe {
                debug_assert!(buf.len() >= std::mem::size_of::<w::SOCKADDR_IN6>());
                std::ptr::write(buf.as_mut_ptr().cast::<w::SOCKADDR_IN6>(), sin6);
            }
            std::mem::size_of::<w::SOCKADDR_IN6>()
        }
    };
    (buf, len as i32)
}

/// Unpack a Winsock-filled address buffer (from `accept`/`getpeername`/
/// `getsockname`) back into a [`SocketAddr`].
// Under `track-w` the donor owns the sockaddr encoding entirely
// (`to_donor_addr`/`from_donor_addr` above), so this arm's hand-rolled
// conversion is genuinely unreachable rather than merely unused.
#[cfg(not(feature = "track-w"))]
fn from_sockaddr(buf: &[u8; 28]) -> Result<SocketAddr> {
    // SAFETY: every variant of the address family union starts with the
    // same `sa_family`/`sin_family`-shaped `u16` at offset 0 — reading
    // it through any one of the pointer types before deciding which
    // variant the rest of `buf` holds is standard sockaddr practice.
    let family = unsafe { *buf.as_ptr().cast::<u16>() };
    match family {
        w::AF_INET => {
            // SAFETY: `family == AF_INET` means Winsock filled this
            // buffer as a `SOCKADDR_IN`, which fits within `buf`'s 28
            // bytes (the same layout `to_sockaddr`'s V4 arm writes).
            let sin = unsafe { &*buf.as_ptr().cast::<w::SOCKADDR_IN>() };
            // SAFETY: reading the union's `S_addr` field — the only one
            // any of this module's code ever writes into it.
            let s_addr = unsafe { sin.sin_addr.S_un.S_addr };
            let ip = Ipv4Addr::from(s_addr.to_ne_bytes());
            Ok(SocketAddr::V4(SocketAddrV4::new(
                ip,
                u16::from_be(sin.sin_port),
            )))
        }
        w::AF_INET6 => {
            // SAFETY: see the V4 arm above, for `SOCKADDR_IN6`.
            let sin6 = unsafe { &*buf.as_ptr().cast::<w::SOCKADDR_IN6>() };
            // SAFETY: reading the union's `Byte` field — the only one
            // any of this module's code ever writes into it.
            let octets = unsafe { sin6.sin6_addr.u.Byte };
            let ip = Ipv6Addr::from(octets);
            Ok(SocketAddr::V6(SocketAddrV6::new(
                ip,
                u16::from_be(sin6.sin6_port),
                sin6.sin6_flowinfo,
                // SAFETY: reading the union's scope-id-bearing member —
                // this backend never writes the alternate view.
                unsafe { sin6.Anonymous.sin6_scope_id },
            )))
        }
        _ => Err(PlatformError::new(
            ErrorKind::Other,
            OsCode::None,
            "unrecognized address family",
        )),
    }
}

// --- Track W primitive layer (D-15) -----------------------------------
//
// Every foreign socket call this module makes goes through one of the
// two-armed helpers below, so the public functions further down keep a
// single body each. Same shape `sys::console`'s `get_mode`/`set_mode`
// pair took in slice 3, for the same reason: two-arming fifteen public
// functions would have doubled the file to say one thing.
//
// The address boundary is what makes this slice its own rather than a
// tail on an earlier one. This crate speaks `std::net::SocketAddr` and
// raw `SOCKADDR_IN`/`SOCKADDR_IN6` byte buffers; the donor speaks its
// own `SocketAddr` enum. Every address-taking call crosses that, so the
// conversion lives here, once, in one direction each way.

/// `std::net::SocketAddr` -> the donor's own address enum.
///
/// Note the octets go across verbatim and only the port is byte-order
/// converted — by the donor, internally. That matches this crate's own
/// `to_sockaddr` (whose comment makes the same point about
/// `from_ne_bytes` reproducing the in-memory pattern rather than
/// converting), so neither side double-swaps.
#[cfg(feature = "track-w")]
fn to_donor_addr(addr: SocketAddr) -> rusty_win32::net::SocketAddr {
    match addr {
        SocketAddr::V4(v4) => rusty_win32::net::SocketAddr::V4 {
            ip: v4.ip().octets(),
            port: v4.port(),
        },
        SocketAddr::V6(v6) => rusty_win32::net::SocketAddr::V6 {
            ip: v6.ip().octets(),
            port: v6.port(),
            flow_info: v6.flowinfo(),
            scope_id: v6.scope_id(),
        },
    }
}

/// The donor's address enum -> `std::net::SocketAddr`.
#[cfg(feature = "track-w")]
fn from_donor_addr(addr: rusty_win32::net::SocketAddr) -> SocketAddr {
    match addr {
        rusty_win32::net::SocketAddr::V4 { ip, port } => {
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(ip), port))
        }
        rusty_win32::net::SocketAddr::V6 {
            ip,
            port,
            flow_info,
            scope_id,
        } => SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::from(ip),
            port,
            flow_info,
            scope_id,
        )),
    }
}

/// `socket(family, type, protocol)`.
#[cfg(feature = "track-w")]
fn raw_socket(v6: bool, stream: bool) -> Result<w::SOCKET> {
    use rusty_win32::net::{AddressFamily, Protocol, SocketKind};
    let family = if v6 {
        AddressFamily::Inet6
    } else {
        AddressFamily::Inet
    };
    let (kind, proto) = if stream {
        (SocketKind::Stream, Protocol::Tcp)
    } else {
        (SocketKind::Dgram, Protocol::Udp)
    };
    rusty_win32::net::socket(family, kind, proto).map_err(|e| trackw_wsa_err("socket", e))
}

#[cfg(not(feature = "track-w"))]
fn raw_socket(v6: bool, stream: bool) -> Result<w::SOCKET> {
    let family = if v6 { w::AF_INET6 } else { w::AF_INET };
    let (kind, proto) = if stream {
        (w::SOCK_STREAM, w::IPPROTO_TCP)
    } else {
        (w::SOCK_DGRAM, 0)
    };
    // SAFETY: plain integer arguments, no memory referenced.
    let sock = unsafe { w::socket(i32::from(family), kind, proto) };
    if sock == w::INVALID_SOCKET {
        return Err(wsa_err("socket"));
    }
    Ok(sock)
}

/// `connect` to an IP address.
#[cfg(feature = "track-w")]
fn raw_connect(sock: w::SOCKET, addr: SocketAddr) -> Result<()> {
    // SAFETY: `sock` is a valid, freshly created socket owned by the
    // caller for the duration of this call.
    unsafe { rusty_win32::net::connect(sock, &to_donor_addr(addr)) }
        .map_err(|e| trackw_wsa_err("connect", e))
}

#[cfg(not(feature = "track-w"))]
fn raw_connect(sock: w::SOCKET, addr: SocketAddr) -> Result<()> {
    let (buf, len) = to_sockaddr(addr);
    // SAFETY: `buf` holds a valid `SOCKADDR_IN`/`SOCKADDR_IN6` for
    // exactly the first `len` bytes (`to_sockaddr`'s contract); `sock`
    // is a freshly created, valid socket.
    let r = unsafe { w::connect(sock, buf.as_ptr().cast::<w::SOCKADDR>(), len) };
    if r != 0 {
        return Err(wsa_err("connect"));
    }
    Ok(())
}

/// `bind` to an IP address.
#[cfg(feature = "track-w")]
fn raw_bind(sock: w::SOCKET, addr: SocketAddr) -> Result<()> {
    // SAFETY: as in `raw_connect`.
    unsafe { rusty_win32::net::bind(sock, &to_donor_addr(addr)) }
        .map_err(|e| trackw_wsa_err("bind", e))
}

#[cfg(not(feature = "track-w"))]
fn raw_bind(sock: w::SOCKET, addr: SocketAddr) -> Result<()> {
    let (buf, len) = to_sockaddr(addr);
    // SAFETY: see `raw_connect`.
    let r = unsafe { w::bind(sock, buf.as_ptr().cast::<w::SOCKADDR>(), len) };
    if r != 0 {
        return Err(wsa_err("bind"));
    }
    Ok(())
}

/// `listen(SOMAXCONN)`.
#[cfg(feature = "track-w")]
fn raw_listen(sock: w::SOCKET) -> Result<()> {
    // SAFETY: `sock` is a valid, bound socket owned by the caller.
    unsafe { rusty_win32::net::listen(sock, w::SOMAXCONN as i32) }
        .map_err(|e| trackw_wsa_err("listen", e))
}

#[cfg(not(feature = "track-w"))]
fn raw_listen(sock: w::SOCKET) -> Result<()> {
    // SAFETY: `sock` is a valid, bound socket.
    let r = unsafe { w::listen(sock, w::SOMAXCONN as i32) };
    if r != 0 {
        return Err(wsa_err("listen"));
    }
    Ok(())
}

/// `accept` on an IP listener.
#[cfg(feature = "track-w")]
fn raw_accept(sock: w::SOCKET) -> Result<(w::SOCKET, SocketAddr)> {
    // SAFETY: `sock` is a valid, listening socket owned by the caller.
    let (new_sock, peer) =
        unsafe { rusty_win32::net::accept(sock) }.map_err(|e| trackw_wsa_err("accept", e))?;
    Ok((new_sock, from_donor_addr(peer)))
}

#[cfg(not(feature = "track-w"))]
fn raw_accept(sock: w::SOCKET) -> Result<(w::SOCKET, SocketAddr)> {
    let mut buf = [0u8; 28];
    let mut len = buf.len() as i32;
    // SAFETY: `buf`/`len` are valid, exclusively borrowed out-params
    // Winsock fills; `sock` is a valid, listening socket.
    let new_sock = unsafe { w::accept(sock, buf.as_mut_ptr().cast::<w::SOCKADDR>(), &mut len) };
    if new_sock == w::INVALID_SOCKET {
        return Err(wsa_err("accept"));
    }
    Ok((new_sock, from_sockaddr(&buf)?))
}

/// `getsockname`/`getpeername` over an IP socket. `peer` selects which.
#[cfg(feature = "track-w")]
fn raw_sock_addr(sock: w::SOCKET, peer: bool) -> Result<SocketAddr> {
    let addr = if peer {
        // SAFETY: `sock` is a valid socket owned by the caller.
        unsafe { rusty_win32::net::peer_addr(sock) }
            .map_err(|e| trackw_wsa_err("getpeername", e))?
    } else {
        // SAFETY: same as the `peer_addr` arm above.
        unsafe { rusty_win32::net::local_addr(sock) }
            .map_err(|e| trackw_wsa_err("getsockname", e))?
    };
    Ok(from_donor_addr(addr))
}

#[cfg(not(feature = "track-w"))]
fn raw_sock_addr(sock: w::SOCKET, peer: bool) -> Result<SocketAddr> {
    let mut buf = [0u8; 28];
    let mut len = buf.len() as i32;
    let ptr = buf.as_mut_ptr().cast::<w::SOCKADDR>();
    let (r, op) = if peer {
        // SAFETY: `buf`/`len` are valid, exclusively borrowed out-params
        // Winsock fills; `sock` is a valid socket.
        let r = unsafe { w::getpeername(sock, ptr, &mut len) };
        (r, "getpeername")
    } else {
        // SAFETY: same as the `getpeername` arm above.
        let r = unsafe { w::getsockname(sock, ptr, &mut len) };
        (r, "getsockname")
    };
    if r != 0 {
        return Err(wsa_err(op));
    }
    from_sockaddr(&buf)
}

/// `recv`.
#[cfg(feature = "track-w")]
fn raw_recv(sock: w::SOCKET, buf: &mut [u8]) -> Result<usize> {
    // SAFETY: `sock` is a valid socket owned by the caller; the buffer's
    // pointer/length pair is derived by the donor from `buf` itself.
    unsafe { rusty_win32::net::recv(sock, buf) }.map_err(|e| trackw_wsa_err("recv", e))
}

#[cfg(not(feature = "track-w"))]
fn raw_recv(sock: w::SOCKET, buf: &mut [u8]) -> Result<usize> {
    let len = i32::try_from(buf.len()).unwrap_or(i32::MAX);
    // SAFETY: `buf` is a valid writable region of at least `len` bytes
    // outliving the call; `sock` is caller-owned.
    let n = unsafe { w::recv(sock, buf.as_mut_ptr().cast(), len, 0) };
    if n < 0 {
        return Err(wsa_err("recv"));
    }
    Ok(n as usize)
}

/// `send`.
#[cfg(feature = "track-w")]
fn raw_send(sock: w::SOCKET, buf: &[u8]) -> Result<usize> {
    // SAFETY: as in `raw_recv`.
    unsafe { rusty_win32::net::send(sock, buf) }.map_err(|e| trackw_wsa_err("send", e))
}

#[cfg(not(feature = "track-w"))]
fn raw_send(sock: w::SOCKET, buf: &[u8]) -> Result<usize> {
    let len = i32::try_from(buf.len()).unwrap_or(i32::MAX);
    // SAFETY: `buf` is a valid readable region of at least `len` bytes
    // outliving the call; `sock` is caller-owned.
    let n = unsafe { w::send(sock, buf.as_ptr().cast(), len, 0) };
    if n < 0 {
        return Err(wsa_err("send"));
    }
    Ok(n as usize)
}

fn new_tcp_socket(addr: SocketAddr) -> Result<OwnedSocket> {
    ensure_wsa_started();
    Ok(OwnedSocket(raw_socket(
        matches!(addr, SocketAddr::V6(_)),
        true,
    )?))
}

/// `socket` + `connect`, blocking until the connection completes or
/// fails.
pub fn tcp_connect(addr: SocketAddr) -> Result<OwnedSocket> {
    let sock = new_tcp_socket(addr)?;
    raw_connect(sock.raw(), addr)?;
    Ok(sock)
}

/// `socket` + `SO_REUSEADDR` + `bind` + `listen(SOMAXCONN)`.
pub fn tcp_listen(addr: SocketAddr) -> Result<OwnedSocket> {
    let sock = new_tcp_socket(addr)?;
    set_sockopt(&sock, SockOpt::ReuseAddr)?;
    raw_bind(sock.raw(), addr)?;
    raw_listen(sock.raw())?;
    Ok(sock)
}

/// `accept`, returning the accepted connection and the peer's address.
pub fn tcp_accept(listen_sock: &OwnedSocket) -> Result<(OwnedSocket, SocketAddr)> {
    let (sock, peer) = raw_accept(listen_sock.raw())?;
    Ok((OwnedSocket(sock), peer))
}

/// `ioctlsocket(FIONBIO, ...)` (rustils#59) — Winsock's equivalent of
/// `fcntl(F_SETFL, O_NONBLOCK)`. Additive: existing blocking callers
/// are unaffected unless they opt in.
///
/// Track W (D-15): `rusty_win32::net::set_nonblocking`, added upstream
/// for this migration — the donor had the whole TCP/UDP socket surface
/// but no blocking-mode control, since its own consumer never needed
/// one. Same `ioctlsocket(FIONBIO)` call, same set-only limitation
/// (Winsock offers no way to *read* the current mode, which the donor
/// documents rather than leaving as an apparent omission).
#[cfg(feature = "track-w")]
pub fn set_nonblocking(sock: &OwnedSocket, nonblocking: bool) -> Result<()> {
    // SAFETY: `sock` is caller-owned and valid for the life of the `&`
    // borrow — `set_nonblocking`'s whole safety contract.
    unsafe { rusty_win32::net::set_nonblocking(sock.raw(), nonblocking) }
        .map_err(|e| errmap::trackw_err("ioctlsocket(FIONBIO)", e))
}

#[cfg(not(feature = "track-w"))]
pub fn set_nonblocking(sock: &OwnedSocket, nonblocking: bool) -> Result<()> {
    let mut mode: u32 = u32::from(nonblocking);
    // SAFETY: `sock` is caller-owned and valid; `&mut mode` is a valid
    // `u32` out-param `ioctlsocket` both reads (the requested mode) and
    // is documented to only read for `FIONBIO`, outliving the call.
    let r = unsafe { w::ioctlsocket(sock.raw(), w::FIONBIO, &mut mode) };
    if r != 0 {
        return Err(wsa_err("ioctlsocket(FIONBIO)"));
    }
    Ok(())
}

/// The three socket options this backend sets. A closed enum rather
/// than a `(level, name, value)` triple because that is the shape the
/// donor's own `set_sockopt` takes, and mirroring it keeps the track-w
/// arm a one-line map instead of a translation table.
enum SockOpt {
    ReuseAddr,
    NoDelay(bool),
    RecvTimeoutMillis(u32),
}

/// `setsockopt` — Track W (D-15): `rusty_win32::net::set_sockopt`.
#[cfg(feature = "track-w")]
fn set_sockopt(sock: &OwnedSocket, opt: SockOpt) -> Result<()> {
    use rusty_win32::net::SockOpt as D;
    let (donor, op) = match opt {
        SockOpt::ReuseAddr => (D::ReuseAddr(true), "setsockopt(SO_REUSEADDR)"),
        SockOpt::NoDelay(on) => (D::TcpNoDelay(on), "setsockopt(TCP_NODELAY)"),
        SockOpt::RecvTimeoutMillis(ms) => (D::RecvTimeout(ms), "setsockopt(SO_RCVTIMEO)"),
    };
    // SAFETY: `sock` is caller-owned and valid for the life of the `&`
    // borrow; the option carries a plain value, not a pointer.
    unsafe { rusty_win32::net::set_sockopt(sock.raw(), donor) }.map_err(|e| trackw_wsa_err(op, e))
}

#[cfg(not(feature = "track-w"))]
fn set_sockopt(sock: &OwnedSocket, opt: SockOpt) -> Result<()> {
    let (level, name, value, op) = match opt {
        SockOpt::ReuseAddr => (
            w::SOL_SOCKET,
            w::SO_REUSEADDR,
            1u32,
            "setsockopt(SO_REUSEADDR)",
        ),
        SockOpt::NoDelay(on) => (
            w::IPPROTO_TCP,
            w::TCP_NODELAY,
            u32::from(on),
            "setsockopt(TCP_NODELAY)",
        ),
        SockOpt::RecvTimeoutMillis(ms) => {
            (w::SOL_SOCKET, w::SO_RCVTIMEO, ms, "setsockopt(SO_RCVTIMEO)")
        }
    };
    // SAFETY: `&value` is a valid 4-byte buffer outliving the call — the
    // width every one of these three options takes (`SO_REUSEADDR` and
    // `TCP_NODELAY` as a boolean-valued `int`, `SO_RCVTIMEO` as a
    // millisecond `DWORD` on Winsock, not a `timeval`); `sock` is
    // caller-owned.
    let r = unsafe {
        w::setsockopt(
            sock.raw(),
            level,
            name,
            (&value as *const u32).cast(),
            std::mem::size_of::<u32>() as i32,
        )
    };
    if r != 0 {
        return Err(wsa_err(op));
    }
    Ok(())
}

/// `setsockopt(IPPROTO_TCP, TCP_NODELAY, ...)`.
pub fn set_nodelay(sock: &OwnedSocket, nodelay: bool) -> Result<()> {
    set_sockopt(sock, SockOpt::NoDelay(nodelay))
}

/// `setsockopt(SOL_SOCKET, SO_RCVTIMEO, ...)`. Winsock's `SO_RCVTIMEO`
/// is a plain millisecond count, not a `timeval` struct — `None` and
/// any `Duration` under 1ms both become `0`, which Winsock treats as
/// "no timeout", the same sentinel `set_read_timeout` documents at the
/// trait level.
pub fn set_read_timeout(sock: &OwnedSocket, timeout: Option<Duration>) -> Result<()> {
    let millis: u32 = timeout
        .map(|d| u32::try_from(d.as_millis()).unwrap_or(u32::MAX))
        .unwrap_or(0);
    set_sockopt(sock, SockOpt::RecvTimeoutMillis(millis))
}

/// `getpeername`.
pub fn peer_addr(sock: &OwnedSocket) -> Result<SocketAddr> {
    raw_sock_addr(sock.raw(), true)
}

/// `getsockname`.
pub fn local_addr(sock: &OwnedSocket) -> Result<SocketAddr> {
    raw_sock_addr(sock.raw(), false)
}

/// `recv`.
pub fn read(sock: &OwnedSocket, buf: &mut [u8]) -> Result<usize> {
    raw_recv(sock.raw(), buf)
}

/// `send`.
pub fn write(sock: &OwnedSocket, buf: &[u8]) -> Result<usize> {
    raw_send(sock.raw(), buf)
}

// --- Unix domain sockets (RFC v2 R5+, D16 follow-on) -----------------
//
// `read`/`write`/`recv`/`send` above are already family-agnostic (a
// connected socket's fd/`SOCKET` is just bytes in and out regardless of
// `AF_INET`/`AF_INET6`/`AF_UNIX`), so this section only adds what is
// actually `AF_UNIX`-specific: the `SOCKADDR_UN` <-> `Path` conversion,
// and `connect`/`bind`+`listen`/`accept`/`getpeername`/`getsockname`
// wired to that address type instead of `SocketAddr`'s.

/// `sun_path`'s capacity in `SOCKADDR_UN` (`windows-sys`'s binding of
/// `afunix.h`, the same 108 bytes every BSD-derived `sockaddr_un` uses).
/// One byte of that is reserved for the NUL terminator this module
/// always writes, so `107` is the longest path actually representable.
// Under `track-w` the donor owns `sun_path`'s capacity
// (`rusty_win32::net::UNIX_PATH_CAPACITY`, the same 108) and this
// constant has no remaining reader.
#[cfg(not(feature = "track-w"))]
const UNIX_PATH_CAP: usize = 108;

/// Pack a filesystem [`Path`] into a `SOCKADDR_UN`-shaped byte buffer —
/// the pointer `connect`/`bind` want.
///
/// Unlike [`to_sockaddr`], which carries any losslessly-representable
/// `OsStr` through WTF-16, `AF_UNIX` paths travel through `sun_path`'s
/// narrow (non-UTF-16) `i8` bytes — a real, OS-level narrowing this
/// backend cannot route around, not an implementation shortcut. A path
/// that is not valid UTF-8, or that does not fit `sun_path` alongside
/// its NUL terminator, is rejected here, before any socket call.
// Under `track-w` the donor owns the sockaddr encoding entirely
// (`to_donor_addr`/`from_donor_addr` above), so this arm's hand-rolled
// conversion is genuinely unreachable rather than merely unused.
#[cfg(not(feature = "track-w"))]
fn to_sockaddr_un(path: &Path) -> Result<[u8; std::mem::size_of::<w::SOCKADDR_UN>()]> {
    let s = path.to_str().ok_or_else(|| {
        PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "AF_UNIX path is not valid UTF-8",
        )
    })?;
    let bytes = s.as_bytes();
    if bytes.len() > UNIX_PATH_CAP - 1 {
        return Err(PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "AF_UNIX path exceeds sun_path's 107-byte usable capacity",
        ));
    }

    let mut sun_path = [0i8; UNIX_PATH_CAP];
    for (dst, &b) in sun_path.iter_mut().zip(bytes) {
        *dst = b as i8;
    }
    let sun = w::SOCKADDR_UN {
        sun_family: w::AF_UNIX,
        sun_path,
    };
    let mut buf = [0u8; std::mem::size_of::<w::SOCKADDR_UN>()];
    // SAFETY: `buf` is exactly `size_of::<SOCKADDR_UN>()` bytes;
    // writing a `SOCKADDR_UN` into its start and later reading it back
    // that way is exactly how the API pair on either side of this
    // buffer interprets it.
    unsafe {
        std::ptr::write(buf.as_mut_ptr().cast::<w::SOCKADDR_UN>(), sun);
    }
    Ok(buf)
}

/// Unpack a Winsock-filled `SOCKADDR_UN` buffer (from `accept`/
/// `getpeername`/`getsockname`) back into a path, or `None` for an
/// anonymous (unbound) peer — `len` is at most `sun_family`'s two bytes
/// in that case, mirroring `platform::net::UnixStream::peer_addr`'s
/// documented `Ok(None)` case.
fn from_sockaddr_un(
    buf: &[u8; std::mem::size_of::<w::SOCKADDR_UN>()],
    len: i32,
) -> Result<Option<PathBuf>> {
    let family_size = std::mem::size_of::<u16>();
    let len = usize::try_from(len).unwrap_or(0);
    if len <= family_size {
        return Ok(None);
    }
    // SAFETY: every variant of the address family union starts with the
    // same `sa_family`/`sun_family`-shaped `u16` at offset 0 — reading
    // it before trusting the rest of `buf` is standard sockaddr
    // practice, the same `from_sockaddr` above does for `AF_INET`/
    // `AF_INET6`.
    let family = unsafe { *buf.as_ptr().cast::<u16>() };
    if family != w::AF_UNIX {
        return Err(PlatformError::new(
            ErrorKind::Other,
            OsCode::None,
            "unrecognized address family",
        ));
    }
    let path_end = len.min(buf.len());
    let mut path_bytes = &buf[family_size..path_end];
    // Winsock's `sun_path` is NUL-terminated; trim the terminator (and
    // anything Winsock left past it, though `len` should already stop
    // there) rather than embedding it in the returned `PathBuf`.
    if let Some(nul_pos) = path_bytes.iter().position(|&b| b == 0) {
        path_bytes = &path_bytes[..nul_pos];
    }
    if path_bytes.is_empty() {
        return Ok(None);
    }
    let s = std::str::from_utf8(path_bytes).map_err(|_| {
        PlatformError::new(
            ErrorKind::Other,
            OsCode::None,
            "AF_UNIX peer path is not valid UTF-8",
        )
    })?;
    Ok(Some(PathBuf::from(s)))
}

/// A `Path` as the donor's own Unix-domain address type.
///
/// Both sides reject a non-UTF-8 path: this crate's `to_sockaddr_un`
/// does (an `OsStr` reaches `sun_path` as bytes only if it is valid
/// UTF-8 — Windows `OsStr` is WTF-16 underneath), and the donor's
/// `UnixSocketAddr::new` takes bytes but is fed them from here. Same
/// rejection, same `InvalidInput`, arrived at from the two directions.
#[cfg(feature = "track-w")]
fn to_donor_unix_addr(path: &Path) -> Result<rusty_win32::net::UnixSocketAddr> {
    let s = path.to_str().ok_or_else(|| {
        PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "AF_UNIX path is not valid UTF-8",
        )
    })?;
    rusty_win32::net::UnixSocketAddr::new(s.as_bytes())
        .map_err(|e| trackw_wsa_err("sockaddr_un", e))
}

/// The donor's Unix address as a path, or `None` for an unnamed socket.
#[cfg(feature = "track-w")]
fn from_donor_unix_addr(addr: &rusty_win32::net::UnixSocketAddr) -> Option<PathBuf> {
    let bytes = addr.path_bytes();
    if bytes.is_empty() {
        return None;
    }
    core::str::from_utf8(bytes).ok().map(PathBuf::from)
}

/// `socket(AF_UNIX, SOCK_STREAM, 0)`.
#[cfg(feature = "track-w")]
fn raw_unix_socket() -> Result<w::SOCKET> {
    use rusty_win32::net::{AddressFamily, Protocol, SocketKind};
    rusty_win32::net::socket(
        AddressFamily::Unix,
        SocketKind::Stream,
        Protocol::Unspecified,
    )
    .map_err(|e| trackw_wsa_err("socket", e))
}

#[cfg(not(feature = "track-w"))]
fn raw_unix_socket() -> Result<w::SOCKET> {
    // SAFETY: plain integer arguments, no memory referenced.
    let sock = unsafe { w::socket(i32::from(w::AF_UNIX), w::SOCK_STREAM, 0) };
    if sock == w::INVALID_SOCKET {
        return Err(wsa_err("socket"));
    }
    Ok(sock)
}

/// `connect` to a Unix-domain path.
#[cfg(feature = "track-w")]
fn raw_connect_unix(sock: w::SOCKET, path: &Path) -> Result<()> {
    let addr = to_donor_unix_addr(path)?;
    // SAFETY: `sock` is a freshly created, valid AF_UNIX socket owned by
    // the caller for the duration of this call.
    unsafe { rusty_win32::net::connect_unix(sock, &addr) }.map_err(|e| trackw_wsa_err("connect", e))
}

#[cfg(not(feature = "track-w"))]
fn raw_connect_unix(sock: w::SOCKET, path: &Path) -> Result<()> {
    let buf = to_sockaddr_un(path)?;
    // SAFETY: `buf` holds a valid `SOCKADDR_UN` for its entire length
    // (`to_sockaddr_un`'s contract); `sock` is a freshly created, valid
    // socket.
    let r = unsafe { w::connect(sock, buf.as_ptr().cast::<w::SOCKADDR>(), buf.len() as i32) };
    if r != 0 {
        return Err(wsa_err("connect"));
    }
    Ok(())
}

/// `bind` to a Unix-domain path. Reports the raw failure rather than
/// mapping it, because `unix_listen`'s stale-socket retry has to
/// distinguish `WSAEADDRINUSE` from everything else.
#[cfg(feature = "track-w")]
fn raw_bind_unix(sock: w::SOCKET, path: &Path) -> Result<()> {
    let addr = to_donor_unix_addr(path)?;
    // SAFETY: as in `raw_connect_unix`.
    unsafe { rusty_win32::net::bind_unix(sock, &addr) }.map_err(|e| trackw_wsa_err("bind", e))
}

#[cfg(not(feature = "track-w"))]
fn raw_bind_unix(sock: w::SOCKET, path: &Path) -> Result<()> {
    let buf = to_sockaddr_un(path)?;
    // SAFETY: see `raw_connect_unix`.
    let r = unsafe { w::bind(sock, buf.as_ptr().cast::<w::SOCKADDR>(), buf.len() as i32) };
    if r != 0 {
        return Err(wsa_err("bind"));
    }
    Ok(())
}

/// `accept` on a Unix-domain listener.
#[cfg(feature = "track-w")]
fn raw_accept_unix(sock: w::SOCKET) -> Result<(w::SOCKET, Option<PathBuf>)> {
    // SAFETY: `sock` is a valid, listening AF_UNIX socket owned by the
    // caller.
    let (new_sock, peer) =
        unsafe { rusty_win32::net::accept_unix(sock) }.map_err(|e| trackw_wsa_err("accept", e))?;
    Ok((new_sock, from_donor_unix_addr(&peer)))
}

#[cfg(not(feature = "track-w"))]
fn raw_accept_unix(sock: w::SOCKET) -> Result<(w::SOCKET, Option<PathBuf>)> {
    let mut buf = [0u8; std::mem::size_of::<w::SOCKADDR_UN>()];
    let mut len = buf.len() as i32;
    // SAFETY: `buf`/`len` are valid, exclusively borrowed out-params
    // Winsock fills; `sock` is a valid, listening socket.
    let new_sock = unsafe { w::accept(sock, buf.as_mut_ptr().cast::<w::SOCKADDR>(), &mut len) };
    if new_sock == w::INVALID_SOCKET {
        return Err(wsa_err("accept"));
    }
    Ok((new_sock, from_sockaddr_un(&buf, len)?))
}

/// `getsockname` on a Unix-domain socket.
#[cfg(feature = "track-w")]
fn raw_local_addr_unix(sock: w::SOCKET) -> Result<Option<PathBuf>> {
    // SAFETY: `sock` is a valid AF_UNIX socket owned by the caller.
    let addr = unsafe { rusty_win32::net::local_addr_unix(sock) }
        .map_err(|e| trackw_wsa_err("getsockname", e))?;
    Ok(from_donor_unix_addr(&addr))
}

#[cfg(not(feature = "track-w"))]
fn raw_local_addr_unix(sock: w::SOCKET) -> Result<Option<PathBuf>> {
    let mut buf = [0u8; std::mem::size_of::<w::SOCKADDR_UN>()];
    let mut len = buf.len() as i32;
    // SAFETY: `buf`/`len` are valid out-params Winsock fills; `sock` is
    // a valid socket.
    let r = unsafe { w::getsockname(sock, buf.as_mut_ptr().cast::<w::SOCKADDR>(), &mut len) };
    if r != 0 {
        return Err(wsa_err("getsockname"));
    }
    from_sockaddr_un(&buf, len)
}

fn new_unix_socket() -> Result<OwnedSocket> {
    ensure_wsa_started();
    Ok(OwnedSocket(raw_unix_socket()?))
}

/// `socket` + `connect`, blocking until the connection completes or
/// fails.
pub fn unix_connect(path: &Path) -> Result<OwnedSocket> {
    let sock = new_unix_socket()?;
    raw_connect_unix(sock.raw(), path)?;
    Ok(sock)
}

/// How many times [`is_stale_socket`] retries its probe connect while it
/// keeps seeing `WSAENOBUFS` — see that function's own doc comment for
/// why this exists at all.
const STALE_PROBE_ENOBUFS_RETRIES: u32 = 20;
const STALE_PROBE_ENOBUFS_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(25);

/// Probe whether the `AF_UNIX` path at `path` is a stale leftover file
/// (no live listener) or genuinely held by one — see this module's doc
/// comment for why a throwaway `connect` is the only way to tell.
///
/// Real `windows-latest` CI evidence (`rusty_prime_agent`'s own
/// integration test) showed the probe connect reliably returning
/// `WSAENOBUFS` (10055, "No buffer space available") — not
/// `WSAECONNREFUSED` — in exactly the race this module's docs already
/// describe (rebind right after force-killing a process that also held
/// a second `AF_UNIX` connection). `WSAENOBUFS` is Winsock's generic
/// resource-transient code, not a considered "yes/no" answer the way
/// `WSAECONNREFUSED`/a successful connect are — treating it as "not
/// stale" outright (the previous behavior) means this scenario can
/// never reclaim, no matter how long the caller retries, since a fresh
/// probe keeps reproducing the identical code. Retrying the probe
/// itself specifically on this one code narrows the window (and is the
/// right *shape* of fix, not a wrong one — the alternative of folding
/// `WSAENOBUFS` straight into "definitely stale" risks deleting a path
/// a live listener still legitimately holds if this code is ever
/// genuinely resource-exhaustion-related, not just this race).
///
/// **This bounded retry is not sufficient on its own**, per further
/// real `windows-latest` evidence from the same integration test: in
/// its fuller, more realistic scenario (a real supervisor with a longer
/// connection history than this crate's own dedicated regression test),
/// `WSAENOBUFS` was observed persisting solidly for a full 20+ continuous
/// seconds, identically on every fresh probe — well past what any bounded
/// in-process retry here can afford to wait out. `rusty_prime_agent`
/// closed the remaining gap with its own defense in depth
/// (`transport::Listener::bind_with_retry`'s `probe()`-based fallback,
/// which checks liveness via a real request/response round trip instead
/// of trusting any raw OS error code) rather than waiting on a fix at
/// this layer alone. See `docs/decision-request-af-unix-stale-reclaim-
/// race.md` for the full trace across both layers.
fn is_stale_socket(path: &Path) -> bool {
    for attempt in 0..=STALE_PROBE_ENOBUFS_RETRIES {
        let Ok(probe) = new_unix_socket() else {
            return false;
        };
        let connect_result = raw_connect_unix(probe.raw(), path);
        let r = match connect_result {
            Ok(()) => 0,
            // The probe only needs to distinguish "refused" (nothing
            // listening — stale) from "connected" (a live owner). Any
            // other failure is neither, and is reported as a nonzero
            // code the caller's `WSAECONNREFUSED` check below will not
            // match.
            Err(e) if e.kind == ErrorKind::ConnectionRefused => w::WSAECONNREFUSED,
            Err(e)
                if matches!(e.os, OsCode::Win32(code) if code == w::WSAENOBUFS as u32)
                    && attempt < STALE_PROBE_ENOBUFS_RETRIES =>
            {
                std::thread::sleep(STALE_PROBE_ENOBUFS_RETRY_DELAY);
                continue;
            }
            Err(_) => -1,
        };
        // A live listener accepting the probe means not stale; `probe`'s
        // `Drop` closes it, ending the connection without disturbing the
        // listener. Otherwise stale iff the refusal was
        // `WSAECONNREFUSED`.
        //
        // This no longer re-reads `WSAGetLastError` after the fact. That
        // was already a latent race (any intervening Winsock call
        // overwrites the slot), and under `track-w` the code has to
        // come from the returned value anyway — note 003's lesson
        // landing somewhere it changes behavior rather than merely
        // restating it.
        return r == w::WSAECONNREFUSED;
    }
    false
}

/// Whether a failed `bind` is worth probing [`is_stale_socket`] over, not
/// just the textbook `WSAEADDRINUSE` case.
///
/// A dead listener's leftover path is documented (this module's own doc
/// comment) to always come back `WSAEADDRINUSE` — true on an idle system.
/// It stops being true in one specific race: the previous owner is force-
/// killed (`TerminateProcess`, not a graceful close) while it also holds
/// a *second*, unrelated live `AF_UNIX` connection open (an outbound
/// connection to some other path). Windows tears down a terminated
/// process's whole socket table as one batch, and afunix.sys's bookkeeping
/// for the path this function is trying to rebind can apparently still be
/// mid-teardown when a fresh `bind` on it lands — Winsock reports the call
/// as failed (`SOCKET_ERROR`) but leaves `WSAGetLastError` at `0`
/// (success), which decodes to `OsCode::Win32(0)` / `ErrorKind::Other`
/// here, not `AddrInUse`. See
/// `docs/decision-request-af-unix-stale-reclaim-race.md` for the full
/// writeup and the harness repro this was traced from — confirmed on
/// real `windows-latest` CI, and promoted to `docs/divergences.md`
/// **016**.
///
/// A fresh, just-created socket's `bind` genuinely cannot fail with
/// success — treating that specific nonsensical combination as an
/// `AddrInUse`-equivalent candidate is safe rather than permissive:
/// [`is_stale_socket`]'s own probe-connect is still the only thing that
/// actually authorizes deleting the path, so a *real* live listener is
/// never at risk of being reclaimed out from under it just because this
/// gate widened.
fn is_stale_bind_candidate(e: &PlatformError) -> bool {
    e.kind == ErrorKind::AddrInUse || matches!(e.os, OsCode::Win32(0))
}

/// `socket` + `bind` (stale-cleanup retried once — see this module's doc
/// comment) + `listen(SOMAXCONN)`.
pub fn unix_listen(path: &Path) -> Result<OwnedSocket> {
    let sock = new_unix_socket()?;
    // The stale-socket retry now branches on the mapped `ErrorKind`
    // rather than a fresh `WSAGetLastError` read. That is not merely
    // equivalent, it is *more* correct: re-reading the thread-local
    // after `bind` returned was already a latent race (any intervening
    // Winsock call overwrites it), and under `track-w` the code has to
    // come from the returned value anyway — the donor captured it at the
    // only instant it was valid. Note 003's lesson, arriving where it
    // actually changes something.
    let first = raw_bind_unix(sock.raw(), path);
    let bound = match first {
        Ok(()) => Ok(()),
        Err(e) if is_stale_bind_candidate(&e) && is_stale_socket(path) => {
            let wide = to_wide_nul(path.as_os_str());
            // Stays on windows-sys in both configurations: this is a
            // *filesystem* unlink sitting in a net module, and the
            // donor's `fs::delete_file` takes `&str` where this has an
            // `OsStr`. `sys::fileio` is where a future slice should own
            // it, not here.
            // SAFETY: `wide` is a valid, NUL-terminated UTF-16 buffer
            // outliving the call.
            unsafe { w::DeleteFileW(wide.as_ptr()) };
            raw_bind_unix(sock.raw(), path)
        }
        Err(e) => Err(e),
    };
    bound?;
    raw_listen(sock.raw())?;
    Ok(sock)
}

/// `accept`, returning the accepted connection and the peer's path, if
/// it bound to one.
pub fn unix_accept(listen_sock: &OwnedSocket) -> Result<(OwnedSocket, Option<PathBuf>)> {
    let (sock, peer) = raw_accept_unix(listen_sock.raw())?;
    Ok((OwnedSocket(sock), peer))
}

/// `getpeername`. `Ok(None)` when the peer connected from an unnamed
/// (anonymous) `AF_UNIX` socket.
pub fn unix_peer_addr(sock: &OwnedSocket) -> Result<Option<PathBuf>> {
    let mut buf = [0u8; std::mem::size_of::<w::SOCKADDR_UN>()];
    let mut len = buf.len() as i32;
    // SAFETY: `buf`/`len` are valid, exclusively borrowed out-params
    // Winsock fills; `sock` is a valid, connected socket.
    let r = unsafe { w::getpeername(sock.raw(), buf.as_mut_ptr().cast::<w::SOCKADDR>(), &mut len) };
    if r != 0 {
        return Err(wsa_err("getpeername"));
    }
    from_sockaddr_un(&buf, len)
}

/// `getsockname`. `Ok(None)` when the socket is not bound to a path.
pub fn unix_local_addr(sock: &OwnedSocket) -> Result<Option<PathBuf>> {
    raw_local_addr_unix(sock.raw())
}

// --- UDP datagram sockets (D16 final slice) --------------------------
//
// Connectionless: one socket both sends and receives, addressed per
// call via `sendto`/`recvfrom` rather than a fixed peer from
// `connect`/`accept`. `local_addr` (above, TCP's) is already a plain
// `getsockname` with nothing TCP-specific about it — reused as-is for
// UDP rather than duplicated.

/// `sendto`.
#[cfg(feature = "track-w")]
fn raw_sendto(sock: w::SOCKET, buf: &[u8], addr: SocketAddr) -> Result<usize> {
    // SAFETY: `sock` is a valid socket owned by the caller; the buffer's
    // pointer/length pair is derived by the donor from `buf` itself.
    unsafe { rusty_win32::net::sendto(sock, buf, &to_donor_addr(addr)) }
        .map_err(|e| trackw_wsa_err("sendto", e))
}

#[cfg(not(feature = "track-w"))]
fn raw_sendto(sock: w::SOCKET, buf: &[u8], addr: SocketAddr) -> Result<usize> {
    let len = i32::try_from(buf.len()).unwrap_or(i32::MAX);
    let (addr_buf, addr_len) = to_sockaddr(addr);
    // SAFETY: `buf` is valid for `len` bytes for the call's duration;
    // `addr_buf` holds a valid sockaddr for exactly `addr_len` bytes;
    // `sock` is caller-owned.
    let n = unsafe {
        w::sendto(
            sock,
            buf.as_ptr(),
            len,
            0,
            addr_buf.as_ptr().cast::<w::SOCKADDR>(),
            addr_len,
        )
    };
    if n < 0 {
        return Err(wsa_err("sendto"));
    }
    Ok(n as usize)
}

/// `recvfrom`.
#[cfg(feature = "track-w")]
fn raw_recvfrom(sock: w::SOCKET, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
    // SAFETY: as in `raw_sendto`.
    let (n, peer) = unsafe { rusty_win32::net::recvfrom(sock, buf) }
        .map_err(|e| trackw_wsa_err("recvfrom", e))?;
    Ok((n, from_donor_addr(peer)))
}

#[cfg(not(feature = "track-w"))]
fn raw_recvfrom(sock: w::SOCKET, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
    let len = i32::try_from(buf.len()).unwrap_or(i32::MAX);
    let mut addr_buf = [0u8; 28];
    let mut addr_len = addr_buf.len() as i32;
    // SAFETY: `buf` is valid for `len` bytes for the call's duration;
    // `addr_buf`/`addr_len` are valid, exclusively borrowed out-params
    // Winsock fills; `sock` is caller-owned.
    let n = unsafe {
        w::recvfrom(
            sock,
            buf.as_mut_ptr(),
            len,
            0,
            addr_buf.as_mut_ptr().cast::<w::SOCKADDR>(),
            &mut addr_len,
        )
    };
    if n < 0 {
        return Err(wsa_err("recvfrom"));
    }
    Ok((n as usize, from_sockaddr(&addr_buf)?))
}

fn new_udp_socket(addr: SocketAddr) -> Result<OwnedSocket> {
    ensure_wsa_started();
    Ok(OwnedSocket(raw_socket(
        matches!(addr, SocketAddr::V6(_)),
        false,
    )?))
}

/// `socket` + `bind`. No `listen`/`accept` — UDP has neither.
pub fn udp_bind(addr: SocketAddr) -> Result<OwnedSocket> {
    let sock = new_udp_socket(addr)?;
    raw_bind(sock.raw(), addr)?;
    Ok(sock)
}

/// `sendto`, one datagram per call — fire-and-forget, no handshake to
/// fail if nothing is listening at `addr`.
pub fn udp_send_to(sock: &OwnedSocket, buf: &[u8], addr: SocketAddr) -> Result<usize> {
    raw_sendto(sock.raw(), buf, addr)
}

/// `recvfrom`, blocking until one datagram arrives. A datagram larger
/// than `buf` is truncated to `buf`'s length, matching `WSARecvFrom`'s
/// own truncation behavior for `SOCK_DGRAM` — not detected or reported
/// here, since Winsock gives no signal distinguishing "exactly
/// `buf.len()` bytes arrived" from "more arrived and got truncated".
pub fn udp_recv_from(sock: &OwnedSocket, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
    raw_recvfrom(sock.raw(), buf)
}
