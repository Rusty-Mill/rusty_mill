//! Raw socket primitives (rustils#48, net-only slice) — ported from
//! `platform-linux::sys::net`, differing exactly where the BSD syscall
//! surface differs from Linux's (this crate's `lib.rs` doc comment has
//! the inventory): no `SOCK_CLOEXEC`/`SOCK_NONBLOCK` socket-type flags
//! (close-on-exec is set after the fact via `fcntl(F_SETFD)`), no
//! `accept4` (plain `accept` + the same post-creation `fcntl`), and a
//! leading length byte on every sockaddr variant (`sin_len`/`sin6_len`/
//! `sun_len`), handled by building each one via `zeroed()` + field
//! assignment rather than a full struct literal so the extra field
//! never needs naming.
//!
//! Written for Darwin first and widened to generic BSD in rustils#86.
//! Only the third difference is truly universal; the first two describe
//! Darwin, and the other four BSDs in the gate do have `SOCK_CLOEXEC`
//! and `accept4`. Nothing here changed to widen it, because the
//! Darwin-shaped path is the *intersection* of the five — correct
//! everywhere, optimal on one. `set_cloexec` below carries what that
//! costs.

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
        // `AF_UNIX` connect/bind: `ENOENT` is "no file at all" (the
        // Unix-socket counterpart of TCP's `ConnectionRefused` for
        // "nothing there") — a socket file that exists but has nothing
        // listening still surfaces as `ECONNREFUSED` above, same as
        // Linux.
        libc::ENOENT => ErrorKind::NotFound,
        _ => ErrorKind::Other,
    }
}

fn net_err(op: &'static str) -> PlatformError {
    let e = errno();
    PlatformError::new(kind_of(e), OsCode::Errno(e), op)
}

/// `fcntl(F_SETFD, FD_CLOEXEC)` on a freshly created fd — Darwin has no
/// `SOCK_CLOEXEC` type flag at `socket(2)`/`accept(2)` to request this
/// atomically, so every socket-creating call in this module sets it as
/// an explicit second step instead. There is a theoretical fork+exec
/// race between creation and this call (another thread forking in
/// between would leak the fd into its child); `platform`'s own process
/// surface has no thread-safety story broader than that today, and
/// Linux's own `SOCK_CLOEXEC` avoiding the race entirely is exactly the
/// asymmetry this module's doc comment flags rather than papers over.
///
/// Since rustils#86 that asymmetry is no longer Linux-vs-BSD but
/// Darwin-vs-everyone: FreeBSD, OpenBSD, NetBSD and DragonFly all have
/// `SOCK_CLOEXEC` and `accept4` too, so on four of this crate's five
/// targets the race is one this function *chooses* rather than one the
/// kernel imposes. Kept deliberately: a single code path across the
/// gate, at the cost of an atomicity nothing in this workspace depends
/// on yet. Closing it means admitting `SOCK_CLOEXEC`/`accept4` to
/// `ffi::libc_surface` behind a per-OS `cfg` and splitting every
/// socket-creating function below — mechanical, and pinned by the
/// `tests/` suites, but not free, and not warranted without a consumer
/// that needs it.
fn set_cloexec(fd: &OwnedFd) -> Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: `fd` is caller-owned and valid; `FD_CLOEXEC` is the sole
    // variadic argument `F_SETFD` expects.
    let r = unsafe { c::fcntl(fd.as_raw_fd(), c::F_SETFD, c::FD_CLOEXEC) };
    if r < 0 {
        return Err(net_err("fcntl(F_SETFD)"));
    }
    Ok(())
}

/// Pack a [`SocketAddr`] into a kernel-layout `sockaddr_storage` and the
/// length of the variant actually filled in (`sockaddr_in` or
/// `sockaddr_in6`) — the pair `connect`/`bind` want. Built field by
/// field (not a struct literal) so `sin_len`/`sin6_len` — the BSD-only
/// leading length byte Linux's variants don't have — is set without
/// ever needing to be named at the call site below.
fn to_sockaddr(addr: SocketAddr) -> (c::sockaddr_storage, c::socklen_t) {
    // SAFETY: not actually unsafe to construct — `sockaddr_storage` is
    // plain-old-data for which all-zeroes is a valid (if meaningless)
    // value; only the variant `ss_family` selects is ever read back.
    let mut storage: c::sockaddr_storage = unsafe { std::mem::zeroed() };
    let len = match addr {
        SocketAddr::V4(v4) => {
            // SAFETY: all-zeroes is a valid starting `sockaddr_in`
            // (the same reasoning `storage` above relies on); every
            // field is then explicitly assigned before use.
            let mut sin: c::sockaddr_in = unsafe { std::mem::zeroed() };
            sin.sin_len = std::mem::size_of::<c::sockaddr_in>() as u8;
            sin.sin_family = c::AF_INET as _;
            sin.sin_port = v4.port().to_be();
            sin.sin_addr = libc::in_addr {
                // `s_addr` is a raw 32-bit blob whose in-memory bytes
                // must equal the address octets in order —
                // `from_ne_bytes` reproduces exactly that byte pattern
                // on any host, not a byte-order conversion.
                s_addr: u32::from_ne_bytes(v4.ip().octets()),
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
            // SAFETY: see the V4 arm above.
            let mut sin6: c::sockaddr_in6 = unsafe { std::mem::zeroed() };
            sin6.sin6_len = std::mem::size_of::<c::sockaddr_in6>() as u8;
            sin6.sin6_family = c::AF_INET6 as _;
            sin6.sin6_port = v6.port().to_be();
            sin6.sin6_flowinfo = v6.flowinfo();
            sin6.sin6_addr = libc::in6_addr {
                s6_addr: v6.ip().octets(),
            };
            sin6.sin6_scope_id = v6.scope_id();
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
    // SAFETY: plain integer arguments, no memory referenced. No
    // `SOCK_CLOEXEC` in this crate's libc surface (this module's doc
    // comment) — `set_cloexec` below is the second step that stands in
    // for it.
    let fd = unsafe { c::socket(family, c::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(net_err("socket"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    set_cloexec(&fd)?;
    Ok(fd)
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

/// `accept` (no `accept4` in this crate's libc surface — this module's
/// doc comment) + `fcntl(F_SETFD, FD_CLOEXEC)` on the returned fd,
/// returning the accepted connection and the peer's address.
pub fn tcp_accept(listen_fd: &OwnedFd) -> Result<(OwnedFd, SocketAddr)> {
    use std::os::fd::AsRawFd;
    // SAFETY: `storage`/`len` are valid, exclusively borrowed out-params
    // the kernel fills; `listen_fd` is a valid, listening socket.
    let (fd, storage) = unsafe {
        let mut storage: c::sockaddr_storage = std::mem::zeroed();
        let mut len = std::mem::size_of::<c::sockaddr_storage>() as c::socklen_t;
        let fd = c::accept(
            listen_fd.as_raw_fd(),
            (&mut storage as *mut c::sockaddr_storage).cast::<c::sockaddr>(),
            &mut len,
        );
        (fd, storage)
    };
    if fd < 0 {
        return Err(net_err("accept"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    set_cloexec(&owned)?;
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

/// `setsockopt(SOL_SOCKET, SO_RCVTIMEO, ...)`. `None` sets an all-zero
/// `timeval`, which `SO_RCVTIMEO` treats as "no timeout" on the BSDs
/// the same as on Linux.
pub fn set_read_timeout(fd: &OwnedFd, timeout: Option<Duration>) -> Result<()> {
    use std::os::fd::AsRawFd;
    let tv = match timeout {
        Some(d) => c::timeval {
            tv_sec: d.as_secs() as c::time_t,
            // `suseconds_t` is the one type in this module that varies
            // *within* the gate, not just against Linux: `i32` on
            // Darwin, `long` on the other BSDs (so `i64` on their 64-bit
            // targets). Linux's is an `i64` that `From<u32>` accepts
            // directly; a plain `as` is what covers all of them at once,
            // narrowing or widening as the target requires, the same way
            // `tv_sec`'s cast above already does for `time_t`.
            tv_usec: d.subsec_micros() as c::suseconds_t,
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
/// already-open socket (rustils#41, ported from `platform-linux` as
/// part of this crate's first slice per the issue's own request).
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

// --- Unix domain sockets -----------------------------------------------
//
// Mirrors the TCP block above and `platform-linux::sys::net`'s own Unix
// block: `sockaddr_un` in place of `sockaddr_in{,6}`, `PathBuf` in place
// of `SocketAddr`, mode-0600 narrowing after bind, and the same
// probe-then-retry-once stale-cleanup dance (`docs/behavior/net.md`).

/// The byte offset of `sockaddr_un::sun_path` within `sockaddr_un` —
/// measured once rather than hard-coded, the same reasoning
/// `platform-linux::sys::net`'s copy of this function documents.
fn sun_path_offset() -> usize {
    // SAFETY: all-zeroes is a valid (if meaningless) `sockaddr_un`; the
    // fields' addresses are taken, never read before being written.
    let addr: c::sockaddr_un = unsafe { std::mem::zeroed() };
    let base = std::ptr::addr_of!(addr) as usize;
    let path = std::ptr::addr_of!(addr.sun_path) as usize;
    path - base
}

/// Pack a filesystem `path` into a kernel-layout `sockaddr_un` and the
/// length of the filled-in prefix. Built via `zeroed()` + field
/// assignment (not a struct literal) so `sun_len` — the BSD-only leading
/// length byte — is set without ever needing to be named, the same
/// treatment `to_sockaddr` gives `sin_len`/`sin6_len` above.
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
    addr.sun_len = len as u8;
    Ok((addr, len as c::socklen_t))
}

/// Unpack a kernel-filled `sockaddr_un` back into a path, or `None` for
/// an unnamed (anonymous) `AF_UNIX` endpoint.
///
/// The returned `len` cannot be trusted to delimit the path. Two
/// separate real-hardware findings, neither catchable by
/// cross-compile-check:
///
/// - **Darwin** (rustils#48, its first `macos-latest` run): on an
///   anonymous/unbound endpoint, `len` is not shrunk back to the
///   header-only size — it can span the whole zeroed `sun_path` even
///   though no path was ever written, so the Linux-only `len <= offset`
///   test alone reported `Some("\0\0…")` instead of `None`.
/// - **OpenBSD** (rustils#86, its first VM run): the same non-shrinking
///   behavior, but on a *bound* socket — `getsockname` returns the path
///   followed by the rest of the buffer as NUL padding, so popping a
///   single trailing NUL (enough on Linux and Darwin, where `len` is
///   exactly `offset + strlen + 1`) left the padding attached and
///   produced `Some("/tmp/….sock\0\0…")`.
///
/// So `len` only ever bounds where the path *might* end, never where it
/// does. `sun_path` is a C string, and this now reads it as one:
/// truncate at the first NUL within the `len`-bounded window. That is
/// correct on a kernel that shrinks `len` and on one that doesn't, for
/// bound and unbound endpoints alike, and it subsumes both fixes above —
/// an unbound endpoint's all-zero buffer truncates to empty, which is
/// `None`.
///
/// Sound because `to_sockaddr_un` refuses any path containing an
/// embedded NUL, so no legitimate path can be cut short by this.
/// `bsd_unix_conforms` (`net_parity.rs`) pins both halves — the bound
/// listener's `local_addr` and the unbound client's `peer_addr` — and
/// runs on the macOS, FreeBSD and OpenBSD legs.
fn from_sockaddr_un(addr: &c::sockaddr_un, len: c::socklen_t) -> Result<Option<PathBuf>> {
    let offset = sun_path_offset();
    let len = len as usize;
    if len <= offset {
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
    if let Some(nul) = bytes.iter().position(|&b| b == 0) {
        bytes.truncate(nul);
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(std::ffi::OsStr::from_bytes(&bytes))))
}

fn new_unix_socket() -> Result<OwnedFd> {
    // SAFETY: plain integer arguments, no memory referenced.
    let fd = unsafe { c::socket(c::AF_UNIX, c::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(net_err("socket"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    set_cloexec(&fd)?;
    Ok(fd)
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

/// Probe whether the `AF_UNIX` path at `path` is a stale leftover or
/// genuinely held by a live listener, via a throwaway `connect` — same
/// reasoning as `platform-linux::sys::net::is_stale_socket`.
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
        return false;
    }
    errno() == libc::ECONNREFUSED
}

/// `socket` + `bind` (stale-cleanup retried once) + mode-`0600` `chmod`
/// + `listen(SOMAXCONN)`.
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
        // outliving the call; `AT_FDCWD` is the well-known sentinel for
        // "resolve relative to the process cwd".
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

/// `accept` + `fcntl(F_SETFD, FD_CLOEXEC)`, returning the accepted
/// connection and the peer's path (`None` for an unnamed peer).
pub fn unix_accept(listen_fd: &OwnedFd) -> Result<(OwnedFd, Option<PathBuf>)> {
    use std::os::fd::AsRawFd;
    // SAFETY: `addr`/`len` are valid, exclusively borrowed out-params
    // the kernel fills; `listen_fd` is a valid, listening socket.
    let (fd, addr, len) = unsafe {
        let mut addr: c::sockaddr_un = std::mem::zeroed();
        let mut len = std::mem::size_of::<c::sockaddr_un>() as c::socklen_t;
        let fd = c::accept(
            listen_fd.as_raw_fd(),
            (&mut addr as *mut c::sockaddr_un).cast::<c::sockaddr>(),
            &mut len,
        );
        (fd, addr, len)
    };
    if fd < 0 {
        return Err(net_err("accept"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    set_cloexec(&owned)?;
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

// --- UDP datagram sockets ------------------------------------------------

fn new_udp_socket(addr: SocketAddr) -> Result<OwnedFd> {
    let family = match addr {
        SocketAddr::V4(_) => c::AF_INET,
        SocketAddr::V6(_) => c::AF_INET6,
    };
    // SAFETY: plain integer arguments, no memory referenced.
    let fd = unsafe { c::socket(family, c::SOCK_DGRAM, 0) };
    if fd < 0 {
        return Err(net_err("socket"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    set_cloexec(&fd)?;
    Ok(fd)
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
/// own `SOCK_DGRAM` truncation behavior.
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
