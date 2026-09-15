//! Raw TCP socket primitives (RFC v2 R5+, D16). Not track-p-gated — see
//! `ffi::libc_surface`'s doc comment for why (sockets were never in
//! rush's required surface, so `rusty_libc` has nothing to route
//! through here yet).

#![allow(unsafe_code)]

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use crate::ffi::libc_surface as c;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn kind_of(errno: i32) -> ErrorKind {
    match errno {
        libc::ECONNREFUSED => ErrorKind::ConnectionRefused,
        libc::ECONNRESET => ErrorKind::ConnectionReset,
        libc::ECONNABORTED => ErrorKind::ConnectionAborted,
        libc::ENOTCONN => ErrorKind::NotConnected,
        libc::EADDRINUSE => ErrorKind::AddrInUse,
        libc::EADDRNOTAVAIL => ErrorKind::AddrNotAvailable,
        libc::ETIMEDOUT => ErrorKind::TimedOut,
        libc::EACCES | libc::EPERM => ErrorKind::PermissionDenied,
        libc::EINVAL => ErrorKind::InvalidInput,
        libc::EAGAIN => ErrorKind::WouldBlock,
        libc::EINTR => ErrorKind::Interrupted,
        // `AF_UNIX` connect/bind additions: `ENOENT` is what a `connect`
        // to a path with no socket bound there reports (the Unix-socket
        // counterpart of TCP's `ConnectionRefused` for "nothing there",
        // but the kernel distinguishes "no file at all" from "a socket
        // file exists but nothing is listening", the latter still
        // surfacing as `ECONNREFUSED` above).
        libc::ENOENT => ErrorKind::NotFound,
        _ => ErrorKind::Other,
    }
}

fn net_err(op: &'static str) -> PlatformError {
    let e = errno();
    PlatformError::new(kind_of(e), OsCode::Errno(e), op)
}

/// Pack a [`SocketAddr`] into a kernel-layout `sockaddr_storage` and the
/// length of the variant actually filled in (`sockaddr_in` or
/// `sockaddr_in6`) — the pair `connect`/`bind` want.
fn to_sockaddr(addr: SocketAddr) -> (c::sockaddr_storage, c::socklen_t) {
    // SAFETY: not actually unsafe to construct — `sockaddr_storage` is
    // plain-old-data for which all-zeroes is a valid (if meaningless)
    // value; only the variant `ss_family` selects is ever read back.
    let mut storage: c::sockaddr_storage = unsafe { std::mem::zeroed() };
    let len = match addr {
        SocketAddr::V4(v4) => {
            let sin = c::sockaddr_in {
                sin_family: c::AF_INET as u16,
                sin_port: v4.port().to_be(),
                sin_addr: libc::in_addr {
                    // `s_addr` is a raw 32-bit blob whose in-memory bytes
                    // must equal the address octets in order —
                    // `from_ne_bytes` reproduces exactly that byte
                    // pattern on any host, not a byte-order conversion.
                    s_addr: u32::from_ne_bytes(v4.ip().octets()),
                },
                sin_zero: [0; 8],
            };
            // SAFETY: `storage` is a valid, large-enough, suitably
            // aligned buffer for any sockaddr variant on this platform
            // (that is `sockaddr_storage`'s documented purpose); writing
            // a `sockaddr_in` into its start and reading it back as one
            // is exactly how the kernel itself interprets the buffer
            // once `ss_family` says `AF_INET`.
            unsafe {
                std::ptr::write(
                    (&mut storage as *mut c::sockaddr_storage).cast::<c::sockaddr_in>(),
                    sin,
                );
            }
            std::mem::size_of::<c::sockaddr_in>()
        }
        SocketAddr::V6(v6) => {
            let sin6 = c::sockaddr_in6 {
                sin6_family: c::AF_INET6 as u16,
                sin6_port: v6.port().to_be(),
                sin6_flowinfo: v6.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                },
                sin6_scope_id: v6.scope_id(),
            };
            // SAFETY: see the V4 arm above; the same reasoning applies
            // to the `sockaddr_in6` variant.
            unsafe {
                std::ptr::write(
                    (&mut storage as *mut c::sockaddr_storage).cast::<c::sockaddr_in6>(),
                    sin6,
                );
            }
            std::mem::size_of::<c::sockaddr_in6>()
        }
    };
    (storage, len as c::socklen_t)
}

/// Unpack a kernel-filled `sockaddr_storage` (from `accept`/
/// `getpeername`/`getsockname`) back into a [`SocketAddr`].
fn from_sockaddr(storage: &c::sockaddr_storage) -> Result<SocketAddr> {
    match i32::from(storage.ss_family) {
        c::AF_INET => {
            // SAFETY: `ss_family == AF_INET` means the kernel filled
            // this buffer as a `sockaddr_in`, which is no larger than
            // `sockaddr_storage`; reading it back as that type is the
            // same reinterpretation `to_sockaddr` writes as.
            let sin = unsafe { &*(storage as *const c::sockaddr_storage).cast::<c::sockaddr_in>() };
            let ip = Ipv4Addr::from(sin.sin_addr.s_addr.to_ne_bytes());
            Ok(SocketAddr::V4(SocketAddrV4::new(
                ip,
                u16::from_be(sin.sin_port),
            )))
        }
        c::AF_INET6 => {
            // SAFETY: see the V4 arm above, for `sockaddr_in6`.
            let sin6 =
                unsafe { &*(storage as *const c::sockaddr_storage).cast::<c::sockaddr_in6>() };
            let ip = Ipv6Addr::from(sin6.sin6_addr.s6_addr);
            Ok(SocketAddr::V6(SocketAddrV6::new(
                ip,
                u16::from_be(sin6.sin6_port),
                sin6.sin6_flowinfo,
                sin6.sin6_scope_id,
            )))
        }
        _ => Err(PlatformError::new(
            ErrorKind::Other,
            OsCode::None,
            "unrecognized address family",
        )),
    }
}

fn new_tcp_socket(addr: SocketAddr) -> Result<OwnedFd> {
    let family = match addr {
        SocketAddr::V4(_) => c::AF_INET,
        SocketAddr::V6(_) => c::AF_INET6,
    };
    // SAFETY: plain integer arguments, no memory referenced.
    let fd = unsafe { c::socket(family, c::SOCK_STREAM | c::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(net_err("socket"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `socket` + `connect`, blocking until the connection completes or
/// fails.
pub fn tcp_connect(addr: SocketAddr) -> Result<OwnedFd> {
    use std::os::fd::AsRawFd;
    let fd = new_tcp_socket(addr)?;
    let (storage, len) = to_sockaddr(addr);
    // SAFETY: `storage` holds a valid `sockaddr_in`/`sockaddr_in6` for
    // exactly the first `len` bytes (`to_sockaddr`'s contract); `fd` is
    // a freshly created, valid socket.
    let r = unsafe {
        c::connect(
            fd.as_raw_fd(),
            (&storage as *const c::sockaddr_storage).cast::<c::sockaddr>(),
            len,
        )
    };
    if r < 0 {
        return Err(net_err("connect"));
    }
    Ok(fd)
}

/// `socket` + `SO_REUSEADDR` + `bind` + `listen(SOMAXCONN)`.
pub fn tcp_listen(addr: SocketAddr) -> Result<OwnedFd> {
    use std::os::fd::AsRawFd;
    let fd = new_tcp_socket(addr)?;
    let reuse: c::c_int = 1;
    // SAFETY: `&reuse` is a valid `c_int`-sized buffer outliving the
    // call; `fd` is a valid, freshly created socket.
    let r = unsafe {
        c::setsockopt(
            fd.as_raw_fd(),
            c::SOL_SOCKET,
            c::SO_REUSEADDR,
            (&reuse as *const c::c_int).cast(),
            std::mem::size_of::<c::c_int>() as c::socklen_t,
        )
    };
    if r < 0 {
        return Err(net_err("setsockopt(SO_REUSEADDR)"));
    }

    let (storage, len) = to_sockaddr(addr);
    // SAFETY: see `tcp_connect`.
    let r = unsafe {
        c::bind(
            fd.as_raw_fd(),
            (&storage as *const c::sockaddr_storage).cast::<c::sockaddr>(),
            len,
        )
    };
    if r < 0 {
        return Err(net_err("bind"));
    }
    // SAFETY: `fd` is a valid, bound socket.
    let r = unsafe { c::listen(fd.as_raw_fd(), c::SOMAXCONN) };
    if r < 0 {
        return Err(net_err("listen"));
    }
    Ok(fd)
}

/// `accept4` with `SOCK_CLOEXEC`, returning the accepted connection and
/// the peer's address.
pub fn tcp_accept(listen_fd: &OwnedFd) -> Result<(OwnedFd, SocketAddr)> {
    use std::os::fd::AsRawFd;
    // SAFETY: `storage`/`len` are valid, exclusively borrowed out-params
    // the kernel fills; `listen_fd` is a valid, listening socket.
    let (fd, storage) = unsafe {
        let mut storage: c::sockaddr_storage = std::mem::zeroed();
        let mut len = std::mem::size_of::<c::sockaddr_storage>() as c::socklen_t;
        let fd = c::accept4(
            listen_fd.as_raw_fd(),
            (&mut storage as *mut c::sockaddr_storage).cast::<c::sockaddr>(),
            &mut len,
            c::SOCK_CLOEXEC,
        );
        (fd, storage)
    };
    if fd < 0 {
        return Err(net_err("accept4"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let peer = from_sockaddr(&storage)?;
    Ok((owned, peer))
}

/// `setsockopt(IPPROTO_TCP, TCP_NODELAY, ...)`.
pub fn set_nodelay(fd: &OwnedFd, nodelay: bool) -> Result<()> {
    use std::os::fd::AsRawFd;
    let value: c::c_int = c::c_int::from(nodelay);
    // SAFETY: `&value` is a valid `c_int`-sized buffer outliving the
    // call; `fd` is caller-owned.
    let r = unsafe {
        c::setsockopt(
            fd.as_raw_fd(),
            c::IPPROTO_TCP,
            c::TCP_NODELAY,
            (&value as *const c::c_int).cast(),
            std::mem::size_of::<c::c_int>() as c::socklen_t,
        )
    };
    if r < 0 {
        return Err(net_err("setsockopt(TCP_NODELAY)"));
    }
    Ok(())
}

/// `setsockopt(SOL_SOCKET, SO_RCVTIMEO, ...)`. `None` sets an
/// all-zero `timeval`, which `SO_RCVTIMEO` treats as "no timeout" —
/// the same sentinel the kernel itself uses, not a special case this
/// wrapper invents.
pub fn set_read_timeout(fd: &OwnedFd, timeout: Option<Duration>) -> Result<()> {
    use std::os::fd::AsRawFd;
    let tv = match timeout {
        Some(d) => c::timeval {
            tv_sec: d.as_secs() as c::time_t,
            tv_usec: c::suseconds_t::from(d.subsec_micros()),
        },
        None => c::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
    };
    // SAFETY: `&tv` is a valid `timeval`-sized buffer outliving the
    // call; `fd` is caller-owned.
    let r = unsafe {
        c::setsockopt(
            fd.as_raw_fd(),
            c::SOL_SOCKET,
            c::SO_RCVTIMEO,
            (&tv as *const c::timeval).cast(),
            std::mem::size_of::<c::timeval>() as c::socklen_t,
        )
    };
    if r < 0 {
        return Err(net_err("setsockopt(SO_RCVTIMEO)"));
    }
    Ok(())
}

/// `fcntl(F_GETFL)` + `fcntl(F_SETFL)` to toggle `O_NONBLOCK` on an
/// already-open socket (rustils#41: rusty_tail's `rusty_tokio` reactor
/// needs a non-blocking fd to register with its own poll loop).
pub fn set_nonblocking(fd: &OwnedFd, nonblocking: bool) -> Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: `fd` is caller-owned and valid; `fcntl(F_GETFL)` takes no
    // variadic argument.
    let flags = unsafe { c::fcntl(fd.as_raw_fd(), c::F_GETFL) };
    if flags < 0 {
        return Err(net_err("fcntl(F_GETFL)"));
    }
    let new_flags = if nonblocking {
        flags | c::O_NONBLOCK
    } else {
        flags & !c::O_NONBLOCK
    };
    // SAFETY: `fd` is caller-owned and valid; `new_flags` is a plain
    // integer, the sole variadic argument `F_SETFL` expects.
    let r = unsafe { c::fcntl(fd.as_raw_fd(), c::F_SETFL, new_flags) };
    if r < 0 {
        return Err(net_err("fcntl(F_SETFL)"));
    }
    Ok(())
}

/// `getpeername`.
pub fn peer_addr(fd: &OwnedFd) -> Result<SocketAddr> {
    use std::os::fd::AsRawFd;
    // SAFETY: `storage`/`len` are valid, exclusively borrowed out-params
    // the kernel fills; `fd` is a valid, connected socket.
    let storage = unsafe {
        let mut storage: c::sockaddr_storage = std::mem::zeroed();
        let mut len = std::mem::size_of::<c::sockaddr_storage>() as c::socklen_t;
        let r = c::getpeername(
            fd.as_raw_fd(),
            (&mut storage as *mut c::sockaddr_storage).cast::<c::sockaddr>(),
            &mut len,
        );
        if r < 0 {
            return Err(net_err("getpeername"));
        }
        storage
    };
    from_sockaddr(&storage)
}

/// `getsockname`.
pub fn local_addr(fd: &OwnedFd) -> Result<SocketAddr> {
    use std::os::fd::AsRawFd;
    // SAFETY: see `peer_addr`.
    let storage = unsafe {
        let mut storage: c::sockaddr_storage = std::mem::zeroed();
        let mut len = std::mem::size_of::<c::sockaddr_storage>() as c::socklen_t;
        let r = c::getsockname(
            fd.as_raw_fd(),
            (&mut storage as *mut c::sockaddr_storage).cast::<c::sockaddr>(),
            &mut len,
        );
        if r < 0 {
            return Err(net_err("getsockname"));
        }
        storage
    };
    from_sockaddr(&storage)
}

// --- Unix domain sockets (D16 follow-on) -----------------------------
//
// Mirrors the TCP block above: `sockaddr_un` in place of
// `sockaddr_in{,6}`, `PathBuf` in place of `SocketAddr`. Two design
// points from the agreed shape (extraction-map D16: "Unix sockets
// incl. mode + stale-cleanup bind", the LocalAPI/agent consumers
// rusty_tail and shh actually asked for):
//
// 1. Mode: `unix_listen` narrows the bound socket file to `0600`
//    (owner read/write only) right after `bind`, since a freshly
//    `bind`ed `AF_UNIX` path otherwise inherits whatever the process
//    umask leaves it at.
// 2. Stale sockets: `bind` alone can't tell a path a live listener
//    holds apart from one a dead listener left behind — both report
//    `EADDRINUSE` identically. `unix_listen` resolves that ambiguity
//    itself with a throwaway probe connect (`is_stale_socket`, below):
//    `ECONNREFUSED` means nothing is listening (stale — the socket
//    *file* outlived its listener), so the stale path is unlinked and
//    the bind retried exactly once; a successful probe connect means a
//    live listener owns the path, so it's left untouched and
//    `AddrInUse` surfaces same as ever. No unbounded retry loop: one
//    probe, one retry, matching the daemon pattern this mirrors
//    (nginx/docker's own stale-socket handling does the same single
//    probe-then-retry, not a loop).

/// The byte offset of `sockaddr_un::sun_path` within `sockaddr_un` —
/// the `AF_UNIX` counterpart of `sockaddr_in`'s fixed layout, but
/// `sun_path` is a trailing array whose own start isn't at a portable
/// constant offset the way `sin_port` is, so it's measured once here
/// instead of hard-coded.
fn sun_path_offset() -> usize {
    // SAFETY: all-zeroes is a valid (if meaningless) `sockaddr_un`, the
    // same reasoning `to_sockaddr`'s `sockaddr_storage` zeroing relies
    // on; nothing here is ever read before being written, only its
    // fields' addresses are taken.
    let addr: c::sockaddr_un = unsafe { std::mem::zeroed() };
    let base = std::ptr::addr_of!(addr) as usize;
    let path = std::ptr::addr_of!(addr.sun_path) as usize;
    path - base
}

/// Pack a filesystem `path` into a kernel-layout `sockaddr_un` and the
/// length of the filled-in prefix — the `AF_UNIX` counterpart of
/// `to_sockaddr`. Includes the trailing NUL in `len`, matching what a C
/// program passes to `bind`/`connect` for a pathname socket (Linux
/// doesn't require it, but every other consumer of this address does
/// expect it, `from_sockaddr_un` included).
fn to_sockaddr_un(path: &Path) -> Result<(c::sockaddr_un, c::socklen_t)> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.contains(&0) {
        return Err(PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "AF_UNIX path must not contain a NUL byte",
        ));
    }
    // SAFETY: see `sun_path_offset`.
    let mut addr: c::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = c::AF_UNIX as _;
    if bytes.len() >= addr.sun_path.len() {
        return Err(PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "AF_UNIX path too long (must fit in sockaddr_un::sun_path)",
        ));
    }
    for (slot, byte) in addr.sun_path.iter_mut().zip(bytes.iter()) {
        *slot = *byte as c::c_char;
    }
    let len = sun_path_offset() + bytes.len() + 1;
    Ok((addr, len as c::socklen_t))
}

/// Unpack a kernel-filled `sockaddr_un` (from `accept`/`getpeername`/
/// `getsockname`) back into a path, or `None` for an unnamed (anonymous)
/// `AF_UNIX` endpoint — the `AF_UNIX` counterpart of `from_sockaddr`,
/// but with a third legal outcome TCP's address family never has.
fn from_sockaddr_un(addr: &c::sockaddr_un, len: c::socklen_t) -> Result<Option<PathBuf>> {
    let offset = sun_path_offset();
    let len = len as usize;
    if len <= offset {
        // No path at all: an unnamed socket (e.g. a `connect`ing client
        // that never called `bind` itself).
        return Ok(None);
    }
    if i32::from(addr.sun_family) != c::AF_UNIX {
        return Err(PlatformError::new(
            ErrorKind::Other,
            OsCode::None,
            "unrecognized address family",
        ));
    }
    let path_len = (len - offset).min(addr.sun_path.len());
    let mut bytes: Vec<u8> = addr.sun_path[..path_len].iter().map(|&b| b as u8).collect();
    // Trim the trailing NUL `to_sockaddr_un` includes in `len` (and the
    // kernel preserves) — the path itself never contains it.
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(std::ffi::OsStr::from_bytes(&bytes))))
}

fn new_unix_socket() -> Result<OwnedFd> {
    // SAFETY: plain integer arguments, no memory referenced.
    let fd = unsafe { c::socket(c::AF_UNIX, c::SOCK_STREAM | c::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(net_err("socket"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `socket` + `connect`, blocking until the connection completes or
/// fails.
pub fn unix_connect(path: &Path) -> Result<OwnedFd> {
    use std::os::fd::AsRawFd;
    let fd = new_unix_socket()?;
    let (addr, len) = to_sockaddr_un(path)?;
    // SAFETY: `addr` holds a valid `sockaddr_un` for exactly the first
    // `len` bytes (`to_sockaddr_un`'s contract); `fd` is a freshly
    // created, valid socket.
    let r = unsafe {
        c::connect(
            fd.as_raw_fd(),
            (&addr as *const c::sockaddr_un).cast::<c::sockaddr>(),
            len,
        )
    };
    if r < 0 {
        return Err(net_err("connect"));
    }
    Ok(fd)
}

fn path_to_cstring(path: &Path) -> Result<std::ffi::CString> {
    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "AF_UNIX path must not contain a NUL byte",
        )
    })
}

/// Probe whether the `AF_UNIX` path at `path` is a stale leftover (no
/// live listener, just the socket file) or genuinely held by a live
/// one, via a throwaway `connect` — the only way to tell, since
/// `bind`'s `EADDRINUSE` doesn't distinguish the two. `ECONNREFUSED`
/// means the kernel routed the connect and nothing accepted it: stale.
/// A successful connect (immediately dropped, ending it cleanly) or
/// any other outcome is treated as "not stale" — `unix_listen` must
/// never unlink a path it isn't certain is dead, since guessing wrong
/// would hijack a live listener's socket out from under it.
fn is_stale_socket(path: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(probe) = new_unix_socket() else {
        return false;
    };
    let Ok((addr, len)) = to_sockaddr_un(path) else {
        return false;
    };
    // SAFETY: `addr` holds a valid `sockaddr_un` for exactly `len`
    // bytes (`to_sockaddr_un`'s contract); `probe` is a freshly
    // created, valid, otherwise-unused socket.
    let r = unsafe {
        c::connect(
            probe.as_raw_fd(),
            (&addr as *const c::sockaddr_un).cast::<c::sockaddr>(),
            len,
        )
    };
    if r == 0 {
        // A live listener accepted the probe; `probe`'s `Drop` closes
        // it, ending the connection without disturbing the listener.
        return false;
    }
    errno() == libc::ECONNREFUSED
}

/// `socket` + `bind` (stale-cleanup retried once — see the module-level
/// comment) + mode-`0600` `chmod` + `listen(SOMAXCONN)`. No
/// `SO_REUSEADDR` equivalent: `AF_UNIX` has no such option, and the
/// probe-then-unlink dance above is this address family's version of
/// it.
pub fn unix_listen(path: &Path) -> Result<OwnedFd> {
    use std::os::fd::AsRawFd;
    let fd = new_unix_socket()?;
    let (addr, len) = to_sockaddr_un(path)?;
    // SAFETY: see `unix_connect`.
    let mut r = unsafe {
        c::bind(
            fd.as_raw_fd(),
            (&addr as *const c::sockaddr_un).cast::<c::sockaddr>(),
            len,
        )
    };
    if r < 0 && errno() == libc::EADDRINUSE && is_stale_socket(path) {
        let c_path = path_to_cstring(path)?;
        // SAFETY: `c_path` is a valid, NUL-terminated C string
        // outliving the call; `AT_FDCWD` is the well-known sentinel
        // for "resolve relative to the process cwd", the same ambient
        // resolution `bind`/`connect` above already use for `path`.
        unsafe { c::unlinkat(c::AT_FDCWD, c_path.as_ptr(), 0) };
        // SAFETY: identical call to the one above; retried at most once.
        r = unsafe {
            c::bind(
                fd.as_raw_fd(),
                (&addr as *const c::sockaddr_un).cast::<c::sockaddr>(),
                len,
            )
        };
    }
    if r < 0 {
        return Err(net_err("bind"));
    }

    // Narrow the just-created socket file to owner-only, the
    // mode-0600-bind half of D16's agreed shape (rusty_tail's LocalAPI,
    // shh's agent socket) — `bind` alone leaves the file at whatever the
    // process umask allows.
    let c_path = path_to_cstring(path)?;
    // SAFETY: `c_path` is a valid, NUL-terminated C string outliving the
    // call; `bind` above has just created a regular file at this exact
    // path for us to narrow.
    let r = unsafe { c::chmod(c_path.as_ptr(), 0o600 as c::mode_t) };
    if r < 0 {
        return Err(net_err("chmod"));
    }

    // SAFETY: `fd` is a valid, bound socket.
    let r = unsafe { c::listen(fd.as_raw_fd(), c::SOMAXCONN) };
    if r < 0 {
        return Err(net_err("listen"));
    }
    Ok(fd)
}

/// `accept4` with `SOCK_CLOEXEC`, returning the accepted connection and
/// the peer's path (`None` for a peer that connected without binding
/// itself to one — the common case for a plain `unix_connect` client).
pub fn unix_accept(listen_fd: &OwnedFd) -> Result<(OwnedFd, Option<PathBuf>)> {
    use std::os::fd::AsRawFd;
    // SAFETY: `addr`/`len` are valid, exclusively borrowed out-params
    // the kernel fills; `listen_fd` is a valid, listening socket.
    let (fd, addr, len) = unsafe {
        let mut addr: c::sockaddr_un = std::mem::zeroed();
        let mut len = std::mem::size_of::<c::sockaddr_un>() as c::socklen_t;
        let fd = c::accept4(
            listen_fd.as_raw_fd(),
            (&mut addr as *mut c::sockaddr_un).cast::<c::sockaddr>(),
            &mut len,
            c::SOCK_CLOEXEC,
        );
        (fd, addr, len)
    };
    if fd < 0 {
        return Err(net_err("accept4"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let peer = from_sockaddr_un(&addr, len)?;
    Ok((owned, peer))
}

/// `getpeername`.
pub fn unix_peer_addr(fd: &OwnedFd) -> Result<Option<PathBuf>> {
    use std::os::fd::AsRawFd;
    // SAFETY: `addr`/`len` are valid, exclusively borrowed out-params
    // the kernel fills; `fd` is a valid, connected socket.
    let (addr, len) = unsafe {
        let mut addr: c::sockaddr_un = std::mem::zeroed();
        let mut len = std::mem::size_of::<c::sockaddr_un>() as c::socklen_t;
        let r = c::getpeername(
            fd.as_raw_fd(),
            (&mut addr as *mut c::sockaddr_un).cast::<c::sockaddr>(),
            &mut len,
        );
        if r < 0 {
            return Err(net_err("getpeername"));
        }
        (addr, len)
    };
    from_sockaddr_un(&addr, len)
}

/// `getsockname`.
pub fn unix_local_addr(fd: &OwnedFd) -> Result<Option<PathBuf>> {
    use std::os::fd::AsRawFd;
    // SAFETY: see `unix_peer_addr`.
    let (addr, len) = unsafe {
        let mut addr: c::sockaddr_un = std::mem::zeroed();
        let mut len = std::mem::size_of::<c::sockaddr_un>() as c::socklen_t;
        let r = c::getsockname(
            fd.as_raw_fd(),
            (&mut addr as *mut c::sockaddr_un).cast::<c::sockaddr>(),
            &mut len,
        );
        if r < 0 {
            return Err(net_err("getsockname"));
        }
        (addr, len)
    };
    from_sockaddr_un(&addr, len)
}

// --- UDP datagram sockets (D16 final slice) --------------------------
//
// Connectionless: one socket both sends and receives, addressed per
// call via `sendto`/`recvfrom` rather than a fixed peer from
// `connect`/`accept`. `local_addr` (above, TCP's) is already a plain
// `getsockname` with nothing TCP-specific about it — reused as-is for
// UDP rather than duplicated.

fn new_udp_socket(addr: SocketAddr) -> Result<OwnedFd> {
    let family = match addr {
        SocketAddr::V4(_) => c::AF_INET,
        SocketAddr::V6(_) => c::AF_INET6,
    };
    // SAFETY: plain integer arguments, no memory referenced.
    let fd = unsafe { c::socket(family, c::SOCK_DGRAM | c::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(net_err("socket"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `socket` + `bind`. No `listen`/`accept` — UDP has neither.
pub fn udp_bind(addr: SocketAddr) -> Result<OwnedFd> {
    use std::os::fd::AsRawFd;
    let fd = new_udp_socket(addr)?;
    let (storage, len) = to_sockaddr(addr);
    // SAFETY: `storage` holds a valid `sockaddr_in`/`sockaddr_in6` for
    // exactly the first `len` bytes (`to_sockaddr`'s contract); `fd` is
    // a freshly created, valid socket.
    let r = unsafe {
        c::bind(
            fd.as_raw_fd(),
            (&storage as *const c::sockaddr_storage).cast::<c::sockaddr>(),
            len,
        )
    };
    if r < 0 {
        return Err(net_err("bind"));
    }
    Ok(fd)
}

/// `sendto`, one datagram per call — fire-and-forget, no handshake to
/// fail if nothing is listening at `addr`.
pub fn udp_send_to(fd: &OwnedFd, buf: &[u8], addr: SocketAddr) -> Result<usize> {
    use std::os::fd::AsRawFd;
    let (storage, len) = to_sockaddr(addr);
    // SAFETY: `buf` is valid for `buf.len()` bytes for the call's
    // duration; `storage` holds a valid sockaddr for exactly `len`
    // bytes; `fd` is caller-owned.
    let n = unsafe {
        c::sendto(
            fd.as_raw_fd(),
            buf.as_ptr().cast(),
            buf.len(),
            0,
            (&storage as *const c::sockaddr_storage).cast::<c::sockaddr>(),
            len,
        )
    };
    if n < 0 {
        return Err(net_err("sendto"));
    }
    Ok(n as usize)
}

/// `recvfrom`, blocking until one datagram arrives. A datagram larger
/// than `buf` is truncated to `buf`'s length, matching `recvfrom(2)`'s
/// own truncation behavior for `SOCK_DGRAM` — not detected or reported
/// here, since the kernel gives no signal distinguishing "exactly
/// `buf.len()` bytes arrived" from "more arrived and got truncated".
pub fn udp_recv_from(fd: &OwnedFd, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
    use std::os::fd::AsRawFd;
    // SAFETY: `storage`/`len` are valid, exclusively borrowed out-params
    // the kernel fills; `buf` is valid for `buf.len()` bytes; `fd` is
    // caller-owned.
    let (n, storage) = unsafe {
        let mut storage: c::sockaddr_storage = std::mem::zeroed();
        let mut len = std::mem::size_of::<c::sockaddr_storage>() as c::socklen_t;
        let n = c::recvfrom(
            fd.as_raw_fd(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            0,
            (&mut storage as *mut c::sockaddr_storage).cast::<c::sockaddr>(),
            &mut len,
        );
        (n, storage)
    };
    if n < 0 {
        return Err(net_err("recvfrom"));
    }
    let peer = from_sockaddr(&storage)?;
    Ok((n as usize, peer))
}
