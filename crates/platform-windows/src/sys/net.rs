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

#![allow(unsafe_code)]

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use crate::ffi::win32_surface as w;
use crate::util::wide::to_wide_nul;

fn ensure_wsa_started() {
    static START: Once = Once::new();
    START.call_once(|| {
        // SAFETY: `WSADATA` is a plain-old-data struct for which
        // all-zeroes is a valid (if meaningless) value; `WSAStartup`
        // overwrites it on success.
        let mut data: w::WSADATA = unsafe { std::mem::zeroed() };
        // SAFETY: `data` is a valid out-pointer; `0x0202` requests
        // Winsock 2.2, the only version this module's calls target.
        let r = unsafe { w::WSAStartup(0x0202, &mut data) };
        // A `WSAStartup` failure here is unrecoverable for this whole
        // module (every socket call needs it); the same "this really
        // shouldn't happen, and there is no sane fallback" territory
        // `platform-windows` treats as a panic elsewhere it can't
        // thread a `Result` through initialization state.
        assert_eq!(r, 0, "WSAStartup failed with error {r}");
    });
}

fn wsa_err(op: &'static str) -> PlatformError {
    // SAFETY: `WSAGetLastError` takes no arguments and has no
    // preconditions.
    let code = unsafe { w::WSAGetLastError() };
    let kind = match code {
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
    };
    PlatformError::new(kind, OsCode::Win32(code as u32), op)
}

/// An owned Winsock `SOCKET`, closed on drop.
pub struct OwnedSocket(w::SOCKET);

impl Drop for OwnedSocket {
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

fn new_tcp_socket(addr: SocketAddr) -> Result<OwnedSocket> {
    ensure_wsa_started();
    let family = match addr {
        SocketAddr::V4(_) => w::AF_INET,
        SocketAddr::V6(_) => w::AF_INET6,
    };
    // SAFETY: plain integer arguments, no memory referenced.
    let sock = unsafe { w::socket(i32::from(family), w::SOCK_STREAM, w::IPPROTO_TCP) };
    if sock == w::INVALID_SOCKET {
        return Err(wsa_err("socket"));
    }
    Ok(OwnedSocket(sock))
}

/// `socket` + `connect`, blocking until the connection completes or
/// fails.
pub fn tcp_connect(addr: SocketAddr) -> Result<OwnedSocket> {
    let sock = new_tcp_socket(addr)?;
    let (buf, len) = to_sockaddr(addr);
    // SAFETY: `buf` holds a valid `SOCKADDR_IN`/`SOCKADDR_IN6` for
    // exactly the first `len` bytes (`to_sockaddr`'s contract); `sock`
    // is a freshly created, valid socket.
    let r = unsafe { w::connect(sock.raw(), buf.as_ptr().cast::<w::SOCKADDR>(), len) };
    if r != 0 {
        return Err(wsa_err("connect"));
    }
    Ok(sock)
}

/// `socket` + `SO_REUSEADDR` + `bind` + `listen(SOMAXCONN)`.
pub fn tcp_listen(addr: SocketAddr) -> Result<OwnedSocket> {
    let sock = new_tcp_socket(addr)?;
    let reuse: i32 = 1;
    // SAFETY: `&reuse` is a valid `i32`-sized buffer outliving the call;
    // `sock` is a valid, freshly created socket.
    let r = unsafe {
        w::setsockopt(
            sock.raw(),
            w::SOL_SOCKET,
            w::SO_REUSEADDR,
            (&reuse as *const i32).cast(),
            std::mem::size_of::<i32>() as i32,
        )
    };
    if r != 0 {
        return Err(wsa_err("setsockopt(SO_REUSEADDR)"));
    }

    let (buf, len) = to_sockaddr(addr);
    // SAFETY: see `tcp_connect`.
    let r = unsafe { w::bind(sock.raw(), buf.as_ptr().cast::<w::SOCKADDR>(), len) };
    if r != 0 {
        return Err(wsa_err("bind"));
    }
    // SAFETY: `sock` is a valid, bound socket.
    let r = unsafe { w::listen(sock.raw(), w::SOMAXCONN as i32) };
    if r != 0 {
        return Err(wsa_err("listen"));
    }
    Ok(sock)
}

/// `accept`, returning the accepted connection and the peer's address.
pub fn tcp_accept(listen_sock: &OwnedSocket) -> Result<(OwnedSocket, SocketAddr)> {
    let mut buf = [0u8; 28];
    let mut len = buf.len() as i32;
    // SAFETY: `buf`/`len` are valid, exclusively borrowed out-params
    // Winsock fills; `listen_sock` is a valid, listening socket.
    let sock = unsafe {
        w::accept(
            listen_sock.raw(),
            buf.as_mut_ptr().cast::<w::SOCKADDR>(),
            &mut len,
        )
    };
    if sock == w::INVALID_SOCKET {
        return Err(wsa_err("accept"));
    }
    let peer = from_sockaddr(&buf)?;
    Ok((OwnedSocket(sock), peer))
}

/// `ioctlsocket(FIONBIO, ...)` (rustils#59) — Winsock's equivalent of
/// `fcntl(F_SETFL, O_NONBLOCK)`. Additive: existing blocking callers
/// are unaffected unless they opt in.
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

/// `setsockopt(IPPROTO_TCP, TCP_NODELAY, ...)`.
pub fn set_nodelay(sock: &OwnedSocket, nodelay: bool) -> Result<()> {
    let value: i32 = i32::from(nodelay);
    // SAFETY: `&value` is a valid `i32`-sized buffer outliving the call;
    // `sock` is caller-owned.
    let r = unsafe {
        w::setsockopt(
            sock.raw(),
            w::IPPROTO_TCP,
            w::TCP_NODELAY,
            (&value as *const i32).cast(),
            std::mem::size_of::<i32>() as i32,
        )
    };
    if r != 0 {
        return Err(wsa_err("setsockopt(TCP_NODELAY)"));
    }
    Ok(())
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
    // SAFETY: `&millis` is a valid `u32`-sized buffer outliving the
    // call; `sock` is caller-owned.
    let r = unsafe {
        w::setsockopt(
            sock.raw(),
            w::SOL_SOCKET,
            w::SO_RCVTIMEO,
            (&millis as *const u32).cast(),
            std::mem::size_of::<u32>() as i32,
        )
    };
    if r != 0 {
        return Err(wsa_err("setsockopt(SO_RCVTIMEO)"));
    }
    Ok(())
}

/// `getpeername`.
pub fn peer_addr(sock: &OwnedSocket) -> Result<SocketAddr> {
    let mut buf = [0u8; 28];
    let mut len = buf.len() as i32;
    // SAFETY: `buf`/`len` are valid, exclusively borrowed out-params
    // Winsock fills; `sock` is a valid, connected socket.
    let r = unsafe { w::getpeername(sock.raw(), buf.as_mut_ptr().cast::<w::SOCKADDR>(), &mut len) };
    if r != 0 {
        return Err(wsa_err("getpeername"));
    }
    from_sockaddr(&buf)
}

/// `getsockname`.
pub fn local_addr(sock: &OwnedSocket) -> Result<SocketAddr> {
    let mut buf = [0u8; 28];
    let mut len = buf.len() as i32;
    // SAFETY: see `peer_addr`.
    let r = unsafe { w::getsockname(sock.raw(), buf.as_mut_ptr().cast::<w::SOCKADDR>(), &mut len) };
    if r != 0 {
        return Err(wsa_err("getsockname"));
    }
    from_sockaddr(&buf)
}

/// `recv`.
pub fn read(sock: &OwnedSocket, buf: &mut [u8]) -> Result<usize> {
    let len = i32::try_from(buf.len()).unwrap_or(i32::MAX);
    // SAFETY: `buf` is a valid writable region of at least `len` bytes
    // outliving the call; `sock` is caller-owned.
    let n = unsafe { w::recv(sock.raw(), buf.as_mut_ptr().cast(), len, 0) };
    if n < 0 {
        return Err(wsa_err("recv"));
    }
    Ok(n as usize)
}

/// `send`.
pub fn write(sock: &OwnedSocket, buf: &[u8]) -> Result<usize> {
    let len = i32::try_from(buf.len()).unwrap_or(i32::MAX);
    // SAFETY: `buf` is a valid readable region of at least `len` bytes
    // outliving the call; `sock` is caller-owned.
    let n = unsafe { w::send(sock.raw(), buf.as_ptr().cast(), len, 0) };
    if n < 0 {
        return Err(wsa_err("send"));
    }
    Ok(n as usize)
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

fn new_unix_socket() -> Result<OwnedSocket> {
    ensure_wsa_started();
    // SAFETY: plain integer arguments, no memory referenced.
    let sock = unsafe { w::socket(i32::from(w::AF_UNIX), w::SOCK_STREAM, 0) };
    if sock == w::INVALID_SOCKET {
        return Err(wsa_err("socket"));
    }
    Ok(OwnedSocket(sock))
}

/// `socket` + `connect`, blocking until the connection completes or
/// fails.
pub fn unix_connect(path: &Path) -> Result<OwnedSocket> {
    let sock = new_unix_socket()?;
    let buf = to_sockaddr_un(path)?;
    // SAFETY: `buf` holds a valid `SOCKADDR_UN` for its entire length
    // (`to_sockaddr_un`'s contract); `sock` is a freshly created, valid
    // socket.
    let r = unsafe {
        w::connect(
            sock.raw(),
            buf.as_ptr().cast::<w::SOCKADDR>(),
            buf.len() as i32,
        )
    };
    if r != 0 {
        return Err(wsa_err("connect"));
    }
    Ok(sock)
}

/// Probe whether the `AF_UNIX` path at `path` is a stale leftover file
/// (no live listener) or genuinely held by one — see this module's doc
/// comment for why a throwaway `connect` is the only way to tell.
fn is_stale_socket(path: &Path) -> bool {
    let Ok(probe) = new_unix_socket() else {
        return false;
    };
    let Ok(buf) = to_sockaddr_un(path) else {
        return false;
    };
    // SAFETY: `buf` holds a valid `SOCKADDR_UN` for its entire length
    // (`to_sockaddr_un`'s contract); `probe` is a freshly created,
    // valid, otherwise-unused socket.
    let r = unsafe {
        w::connect(
            probe.raw(),
            buf.as_ptr().cast::<w::SOCKADDR>(),
            buf.len() as i32,
        )
    };
    if r == 0 {
        // A live listener accepted the probe; `probe`'s `Drop` closes
        // it, ending the connection without disturbing the listener.
        return false;
    }
    // SAFETY: `WSAGetLastError` takes no arguments and has no
    // preconditions.
    (unsafe { w::WSAGetLastError() }) == w::WSAECONNREFUSED
}

/// `socket` + `bind` (stale-cleanup retried once — see this module's doc
/// comment) + `listen(SOMAXCONN)`.
pub fn unix_listen(path: &Path) -> Result<OwnedSocket> {
    let sock = new_unix_socket()?;
    let buf = to_sockaddr_un(path)?;
    // SAFETY: see `unix_connect`.
    let mut r = unsafe {
        w::bind(
            sock.raw(),
            buf.as_ptr().cast::<w::SOCKADDR>(),
            buf.len() as i32,
        )
    };
    // SAFETY: `WSAGetLastError` takes no arguments and has no
    // preconditions.
    if r != 0 && unsafe { w::WSAGetLastError() } == w::WSAEADDRINUSE && is_stale_socket(path) {
        let wide = to_wide_nul(path.as_os_str());
        // SAFETY: `wide` is a valid, NUL-terminated UTF-16 buffer
        // outliving the call.
        unsafe { w::DeleteFileW(wide.as_ptr()) };
        // SAFETY: identical call to the one above; retried at most once.
        r = unsafe {
            w::bind(
                sock.raw(),
                buf.as_ptr().cast::<w::SOCKADDR>(),
                buf.len() as i32,
            )
        };
    }
    if r != 0 {
        return Err(wsa_err("bind"));
    }
    // SAFETY: `sock` is a valid, bound socket.
    let r = unsafe { w::listen(sock.raw(), w::SOMAXCONN as i32) };
    if r != 0 {
        return Err(wsa_err("listen"));
    }
    Ok(sock)
}

/// `accept`, returning the accepted connection and the peer's path, if
/// it bound to one.
pub fn unix_accept(listen_sock: &OwnedSocket) -> Result<(OwnedSocket, Option<PathBuf>)> {
    let mut buf = [0u8; std::mem::size_of::<w::SOCKADDR_UN>()];
    let mut len = buf.len() as i32;
    // SAFETY: `buf`/`len` are valid, exclusively borrowed out-params
    // Winsock fills; `listen_sock` is a valid, listening socket.
    let sock = unsafe {
        w::accept(
            listen_sock.raw(),
            buf.as_mut_ptr().cast::<w::SOCKADDR>(),
            &mut len,
        )
    };
    if sock == w::INVALID_SOCKET {
        return Err(wsa_err("accept"));
    }
    let peer = from_sockaddr_un(&buf, len)?;
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
    let mut buf = [0u8; std::mem::size_of::<w::SOCKADDR_UN>()];
    let mut len = buf.len() as i32;
    // SAFETY: see `unix_peer_addr`.
    let r = unsafe { w::getsockname(sock.raw(), buf.as_mut_ptr().cast::<w::SOCKADDR>(), &mut len) };
    if r != 0 {
        return Err(wsa_err("getsockname"));
    }
    from_sockaddr_un(&buf, len)
}

// --- UDP datagram sockets (D16 final slice) --------------------------
//
// Connectionless: one socket both sends and receives, addressed per
// call via `sendto`/`recvfrom` rather than a fixed peer from
// `connect`/`accept`. `local_addr` (above, TCP's) is already a plain
// `getsockname` with nothing TCP-specific about it — reused as-is for
// UDP rather than duplicated.

fn new_udp_socket(addr: SocketAddr) -> Result<OwnedSocket> {
    ensure_wsa_started();
    let family = match addr {
        SocketAddr::V4(_) => w::AF_INET,
        SocketAddr::V6(_) => w::AF_INET6,
    };
    // SAFETY: plain integer arguments, no memory referenced.
    let sock = unsafe { w::socket(i32::from(family), w::SOCK_DGRAM, 0) };
    if sock == w::INVALID_SOCKET {
        return Err(wsa_err("socket"));
    }
    Ok(OwnedSocket(sock))
}

/// `socket` + `bind`. No `listen`/`accept` — UDP has neither.
pub fn udp_bind(addr: SocketAddr) -> Result<OwnedSocket> {
    let sock = new_udp_socket(addr)?;
    let (buf, len) = to_sockaddr(addr);
    // SAFETY: `buf` holds a valid `SOCKADDR_IN`/`SOCKADDR_IN6` for
    // exactly the first `len` bytes (`to_sockaddr`'s contract); `sock`
    // is a freshly created, valid socket.
    let r = unsafe { w::bind(sock.raw(), buf.as_ptr().cast::<w::SOCKADDR>(), len) };
    if r != 0 {
        return Err(wsa_err("bind"));
    }
    Ok(sock)
}

/// `sendto`, one datagram per call — fire-and-forget, no handshake to
/// fail if nothing is listening at `addr`.
pub fn udp_send_to(sock: &OwnedSocket, buf: &[u8], addr: SocketAddr) -> Result<usize> {
    let len = i32::try_from(buf.len()).unwrap_or(i32::MAX);
    let (addr_buf, addr_len) = to_sockaddr(addr);
    // SAFETY: `buf` is valid for `len` bytes for the call's duration;
    // `addr_buf` holds a valid sockaddr for exactly `addr_len` bytes;
    // `sock` is caller-owned.
    let n = unsafe {
        w::sendto(
            sock.raw(),
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

/// `recvfrom`, blocking until one datagram arrives. A datagram larger
/// than `buf` is truncated to `buf`'s length, matching `WSARecvFrom`'s
/// own truncation behavior for `SOCK_DGRAM` — not detected or reported
/// here, since Winsock gives no signal distinguishing "exactly
/// `buf.len()` bytes arrived" from "more arrived and got truncated".
pub fn udp_recv_from(sock: &OwnedSocket, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
    let len = i32::try_from(buf.len()).unwrap_or(i32::MAX);
    let mut addr_buf = [0u8; 28];
    let mut addr_len = addr_buf.len() as i32;
    // SAFETY: `buf` is valid for `len` bytes for the call's duration;
    // `addr_buf`/`addr_len` are valid, exclusively borrowed out-params
    // Winsock fills; `sock` is caller-owned.
    let n = unsafe {
        w::recvfrom(
            sock.raw(),
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
    let peer = from_sockaddr(&addr_buf)?;
    Ok((n as usize, peer))
}
