//! Windows Sockets (Winsock2) — `winsock2.h`, a new module added in
//! round 2, previously excluded by `ARCHITECTURE.md`'s non-goals (see
//! `docs/archive/gap-analysis.md`'s, now closed out and archived, "Round
//! 2: previously out-of-scope subsystems"
//! sweep), now in scope per explicit round-2 direction.
//!
//! Scope: basic TCP/UDP client+server socket programming, the same core
//! subset `rusty_libc` wraps for POSIX sockets. Overlapped/IOCP-based
//! async I/O, `WSAPoll`, and protocol-specific options beyond the
//! ordinary set are all explicitly out of scope for this first pass.
//!
//! This first piece is Winsock's own load/unload lifecycle —
//! `WSAStartup`/`WSACleanup`, the one primitive with no POSIX/
//! `rusty_libc` analog: every other Winsock call is documented undefined
//! behavior before a matching `WSAStartup` or after `WSACleanup`.
//! Windows reference-counts nested `WSAStartup`/`WSACleanup` pairs
//! internally, so no shared guard/RAII type is needed here — two plain
//! functions, matching this crate's existing no-`Drop`-anywhere
//! convention (`volume::FindVolumes`/`security::PathSecurityInfo`/
//! `security::BuiltAcl` are the only exceptions, none of which apply to
//! a process-global load count like this one).

extern crate alloc;
use alloc::vec::Vec;

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSAStartup(version_requested: u16, wsa_data: *mut WsaData) -> i32;
    fn WSACleanup() -> i32;
    fn WSAGetLastError() -> i32;
    // The real Win32/BSD-sockets symbol is lowercase `socket`, which
    // would otherwise collide with this module's own `socket` wrapper
    // function below -- `#[link_name]` keeps the real symbol name for
    // linking while giving the Rust binding a distinct identifier.
    #[link_name = "socket"]
    fn raw_socket(address_family: i32, kind: i32, protocol: i32) -> usize;
    fn closesocket(sock: usize) -> i32;
    fn ioctlsocket(sock: usize, cmd: i32, argp: *mut u32) -> i32;
    // Same lowercase-symbol collision as `socket` above -- `bind` would
    // otherwise clash with this module's own `bind` wrapper function.
    #[link_name = "bind"]
    fn raw_bind(sock: usize, name: *const u8, namelen: i32) -> i32;
    // Same lowercase-symbol collision as `socket`/`bind` above -- `listen`
    // would otherwise clash with this module's own `listen` wrapper
    // function.
    #[link_name = "listen"]
    fn raw_listen(sock: usize, backlog: i32) -> i32;
    // Same lowercase-symbol collision as `socket`/`bind`/`listen` above --
    // `accept` would otherwise clash with this module's own `accept`
    // wrapper function.
    #[link_name = "accept"]
    fn raw_accept(sock: usize, addr: *mut u8, addrlen: *mut i32) -> usize;
    // Same lowercase-symbol collision as `socket`/`bind`/`listen`/`accept`
    // above -- `connect` would otherwise clash with this module's own
    // `connect` wrapper function.
    #[link_name = "connect"]
    fn raw_connect(sock: usize, name: *const u8, namelen: i32) -> i32;
    // Same lowercase-symbol collision as `socket`/`bind`/`listen`/
    // `accept`/`connect` above -- `send`/`recv` would otherwise clash
    // with this module's own `send`/`recv` wrapper functions.
    #[link_name = "send"]
    fn raw_send(sock: usize, buf: *const u8, len: i32, flags: i32) -> i32;
    #[link_name = "recv"]
    fn raw_recv(sock: usize, buf: *mut u8, len: i32, flags: i32) -> i32;
    // Same lowercase-symbol collision as `socket`/`bind`/`listen`/
    // `accept`/`connect`/`send`/`recv` above -- `sendto`/`recvfrom` would
    // otherwise clash with this module's own `sendto`/`recvfrom` wrapper
    // functions.
    #[link_name = "sendto"]
    fn raw_sendto(
        sock: usize,
        buf: *const u8,
        len: i32,
        flags: i32,
        to: *const u8,
        tolen: i32,
    ) -> i32;
    #[link_name = "recvfrom"]
    fn raw_recvfrom(
        sock: usize,
        buf: *mut u8,
        len: i32,
        flags: i32,
        from: *mut u8,
        fromlen: *mut i32,
    ) -> i32;
    // Same lowercase-symbol collision as the rest of this module's
    // BSD-socket wrappers -- `shutdown` would otherwise clash with this
    // module's own `shutdown` wrapper function.
    #[link_name = "shutdown"]
    fn raw_shutdown(sock: usize, how: i32) -> i32;
    // No collision here: the real symbols are `setsockopt`/`getsockopt`
    // (no underscore), distinct from this module's `set_sockopt`/
    // `get_sockopt` wrapper functions -- no `#[link_name]` needed.
    fn setsockopt(sock: usize, level: i32, optname: i32, optval: *const u8, optlen: i32) -> i32;
    fn getsockopt(sock: usize, level: i32, optname: i32, optval: *mut u8, optlen: *mut i32) -> i32;
    // No collision here either: the real symbols are `getsockname`/
    // `getpeername`, distinct from this module's `local_addr`/
    // `peer_addr` wrapper functions -- no `#[link_name]` needed.
    fn getsockname(sock: usize, name: *mut u8, namelen: *mut i32) -> i32;
    fn getpeername(sock: usize, name: *mut u8, namelen: *mut i32) -> i32;
    // No collision here either: the real symbols are `getaddrinfo`/
    // `freeaddrinfo`, distinct from this module's `resolve` wrapper
    // function -- no `#[link_name]` needed.
    fn getaddrinfo(
        node: *const u8,
        service: *const u8,
        hints: *const AddrInfoRaw,
        res: *mut *mut AddrInfoRaw,
    ) -> i32;
    fn freeaddrinfo(res: *mut AddrInfoRaw);
    // Same lowercase-symbol collision as the rest of this module's
    // Winsock wrappers -- `htons`/`htonl`/`ntohs`/`ntohl` would otherwise
    // clash with this module's own functions of the same name below.
    #[link_name = "htons"]
    fn raw_htons(hostshort: u16) -> u16;
    #[link_name = "htonl"]
    fn raw_htonl(hostlong: u32) -> u32;
    #[link_name = "ntohs"]
    fn raw_ntohs(netshort: u16) -> u16;
    #[link_name = "ntohl"]
    fn raw_ntohl(netlong: u32) -> u32;
}

/// `INVALID_SOCKET` — the sentinel `socket` returns on failure (real
/// error code obtained separately via `WSAGetLastError`). Verified
/// against mingw-w64's own `winsock2.h` with a compiled `_Static_assert`
/// probe.
const INVALID_SOCKET: usize = usize::MAX;

/// A raw Windows `SOCKET` — matching `std::os::windows::io::RawSocket`
/// and mingw's own `SOCKET` typedef (`UINT_PTR`, pointer-sized). A
/// distinct handle namespace from [`crate::handle::RawHandle`]: a
/// `SOCKET` is closed via [`close_socket`]/`closesocket`, never
/// `CloseHandle`.
pub type RawSocket = usize;

/// `AF_INET`/`AF_INET6`/`AF_UNIX` — the address families this module
/// supports (`AF_IPX`/`AF_BTH`/… remain out of scope). The two IP
/// families were verified against mingw-w64's own `winsock2.h` with a
/// compiled `_Static_assert` probe.
///
/// `Unix` (`AF_UNIX` = 1) arrived later, for Windows 10 1803+'s real
/// `afunix.h` support: a filesystem-path socket, the same abstraction
/// Unix domain sockets provide, usable for local IPC without a loopback
/// port. Its addresses do not fit [`SocketAddr`] — see
/// [`UnixSocketAddr`] for why that is a separate type rather than a
/// third variant.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFamily {
    Unix = 1,
    Inet = 2,
    Inet6 = 23,
}

/// `SOCK_STREAM`/`SOCK_DGRAM` — the two socket types this module
/// supports (`SOCK_RAW`/`SOCK_RDM`/`SOCK_SEQPACKET` are out of scope).
/// Verified against mingw-w64's own `winsock2.h` with a compiled
/// `_Static_assert` probe.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketKind {
    Stream = 1,
    Dgram = 2,
}

/// `IPPROTO_TCP`/`IPPROTO_UDP` — the protocols this module supports.
/// Both verified against mingw-w64's own `winsock2.h` with a compiled
/// `_Static_assert` probe.
///
/// `Unspecified` (`0`) is not a protocol at all but Winsock's "pick the
/// only sensible one for this family and type" value — required for
/// [`AddressFamily::Unix`], which has no protocol numbers of its own,
/// and equally valid for the IP families (where it resolves to TCP for
/// `Stream` and UDP for `Dgram`).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Unspecified = 0,
    Tcp = 6,
    Udp = 17,
}

/// Create a new socket — `socket`. Requires [`startup`] to have been
/// called first (undefined behavior otherwise, per this module's own
/// scope note).
pub fn socket(
    family: AddressFamily,
    kind: SocketKind,
    protocol: Protocol,
) -> Result<RawSocket, crate::error::Win32Error> {
    // SAFETY: `family`/`kind`/`protocol` are plain enum-backed integer
    // values, not pointers.
    let sock = unsafe { raw_socket(family as i32, kind as i32, protocol as i32) };
    if sock == INVALID_SOCKET {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(sock)
    }
}

/// Close a socket opened by [`socket`] — `closesocket`. Never
/// [`crate::handle::close`]/`CloseHandle`: a `SOCKET`'s destructor is
/// always this one.
///
/// # Safety
///
/// `sock` must be a currently-open, valid socket from [`socket`], not
/// already closed.
pub unsafe fn close_socket(sock: RawSocket) -> Result<(), crate::error::Win32Error> {
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract.
    let ok = unsafe { closesocket(sock) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(())
    }
}

// WSADATA (64-bit layout, per mingw-w64's own `psdk_inc/_wsadata.h`):
// `size_of` 408 — verified field-by-field with a compiled
// `_Static_assert` probe. Never read by this crate: `startup`'s only
// interesting output (the error code, if any) comes back as
// `WSAStartup`'s own return value, matching this crate's existing
// "reports failure via its own return value directly" LSTATUS-style
// convention — so this is scratch space only, the same treatment
// `service::control`'s `ServiceStatusRaw` gets.
#[repr(C)]
struct WsaData {
    version: u16,
    high_version: u16,
    max_sockets: u16,
    max_udp_dg: u16,
    vendor_info: *mut u8,
    description: [u8; 257],
    system_status: [u8; 129],
}
const _: () = assert!(core::mem::size_of::<WsaData>() == 408);

/// `MAKEWORD(2, 2)` — Winsock 2.2, the version every modern Windows
/// ships and the only one this crate requests.
const WINSOCK_VERSION_2_2: u16 = 0x0202;

/// Initialize Winsock — `WSAStartup`, requesting version 2.2 (the
/// version every modern Windows ships). Must be called at least once
/// before any other function in this module; Windows reference-counts
/// nested calls internally, so calling this more than once (matched by
/// an equal number of [`cleanup`] calls) is documented as safe, not a
/// caller error this crate needs to guard against.
///
/// Reports failure via its own return value directly — never
/// `GetLastError`/`WSAGetLastError` — so a nonzero return is passed
/// straight to [`crate::error::Win32Error::from_raw`] rather than
/// `Win32Error::last`.
pub fn startup() -> Result<(), crate::error::Win32Error> {
    let mut wsa_data = core::mem::MaybeUninit::<WsaData>::uninit();
    // SAFETY: `wsa_data` is a valid, correctly-sized out-buffer;
    // `WSAStartup` fully initializes it on success, and its contents are
    // otherwise never read by this crate.
    let status = unsafe { WSAStartup(WINSOCK_VERSION_2_2, wsa_data.as_mut_ptr()) };
    if status != 0 {
        Err(crate::error::Win32Error::from_raw(status as u32))
    } else {
        Ok(())
    }
}

/// Tear down Winsock — `WSACleanup`. Every [`startup`] call must be
/// matched by exactly one `cleanup` call (Windows reference-counts
/// nested pairs internally); calling any other function in this module
/// after the reference count reaches zero is documented undefined
/// behavior.
///
/// Unlike [`startup`], failure is reported the ordinary
/// `GetLastError`-equivalent way — `WSAGetLastError`, a distinct
/// per-thread error slot Winsock keeps separately from the regular
/// `GetLastError`/`SetLastError` one.
pub fn cleanup() -> Result<(), crate::error::Win32Error> {
    // SAFETY: `WSACleanup` takes no arguments.
    let status = unsafe { WSACleanup() };
    if status != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        let err = unsafe { WSAGetLastError() };
        Err(crate::error::Win32Error::from_raw(err as u32))
    } else {
        Ok(())
    }
}

/// A local or peer socket address, IPv4 or IPv6 — the `{ip, port}`
/// representation every address-taking function in this module
/// (`bind`/`connect`/`accept`/`sendto`/`recvfrom`/`local_addr`/
/// `peer_addr`) uses, converting to/from the real `sockaddr_in`/`sockaddr_in6` wire
/// format. `ip` octets are stored exactly as they appear on the wire
/// (already address-order, not a multi-byte integer needing an
/// endian conversion) — only `port` (and, for IPv6, nothing else) needs
/// network-byte-order handling, done internally by `to_sockaddr`/
/// `from_sockaddr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketAddr {
    V4 {
        ip: [u8; 4],
        port: u16,
    },
    V6 {
        ip: [u8; 16],
        port: u16,
        /// `sin6_flowinfo` — an opaque 32-bit value most callers leave
        /// `0`; exposed raw and policy-free like this crate's other
        /// bitmask-shaped fields, never interpreted or byte-swapped by
        /// this module.
        flow_info: u32,
        /// `sin6_scope_id` — the IPv6 zone/interface index for
        /// link-local addresses; `0` for a global address. Exposed raw,
        /// same treatment as `flow_info`.
        scope_id: u32,
    },
}

// sockaddr_in: `size_of` 16 — verified field-by-field against
// mingw-w64's own `psdk_inc/_ip_types.h` with a compiled
// `_Static_assert` probe.
#[repr(C)]
#[derive(Clone, Copy)]
struct SockAddrIn {
    family: i16,
    port: u16,
    addr: [u8; 4],
    zero: [u8; 8],
}
const _: () = assert!(core::mem::size_of::<SockAddrIn>() == 16);

// sockaddr_in6: `size_of` 28 — verified field-by-field against
// mingw-w64's own `ws2ipdef.h` with a compiled `_Static_assert` probe.
#[repr(C)]
#[derive(Clone, Copy)]
struct SockAddrIn6 {
    family: i16,
    port: u16,
    flow_info: u32,
    addr: [u8; 16],
    scope_id: u32,
}
const _: () = assert!(core::mem::size_of::<SockAddrIn6>() == 28);

/// A `sockaddr`-shaped byte buffer big enough for either `sockaddr_in`
/// or `sockaddr_in6`, plus the real length to pass as a Win32
/// `namelen`/`addrlen` parameter — the encoded form [`to_sockaddr`]
/// produces, ready to hand to `bind`/`connect`/… as a `(*const u8,
/// i32)` pair via [`RawSockAddr::as_ptr`]/[`RawSockAddr::len`].
pub(crate) struct RawSockAddr {
    bytes: [u8; 28],
    len: i32,
}

impl RawSockAddr {
    pub(crate) fn as_ptr(&self) -> *const u8 {
        self.bytes.as_ptr()
    }

    pub(crate) fn len(&self) -> i32 {
        self.len
    }
}

/// Encode `addr` into its real `sockaddr_in`/`sockaddr_in6` wire form —
/// backing every address-taking function in this module. The reverse of
/// [`from_sockaddr`].
pub(crate) fn to_sockaddr(addr: &SocketAddr) -> RawSockAddr {
    let mut bytes = [0u8; 28];
    let len = match *addr {
        SocketAddr::V4 { ip, port } => {
            let raw = SockAddrIn {
                family: AddressFamily::Inet as i16,
                port: port.to_be(),
                addr: ip,
                zero: [0; 8],
            };
            let size = core::mem::size_of::<SockAddrIn>();
            // SAFETY: `raw` is a plain-old-data `#[repr(C)]` value (only
            // integer/byte-array fields, no padding this crate reads
            // uninitialized), valid to reinterpret as its own `size_of`
            // bytes.
            let raw_bytes =
                unsafe { core::slice::from_raw_parts((&raw as *const SockAddrIn).cast(), size) };
            bytes[..size].copy_from_slice(raw_bytes);
            size as i32
        }
        SocketAddr::V6 {
            ip,
            port,
            flow_info,
            scope_id,
        } => {
            let raw = SockAddrIn6 {
                family: AddressFamily::Inet6 as i16,
                port: port.to_be(),
                flow_info,
                addr: ip,
                scope_id,
            };
            let size = core::mem::size_of::<SockAddrIn6>();
            // SAFETY: same reasoning as the `SockAddrIn` case above.
            let raw_bytes =
                unsafe { core::slice::from_raw_parts((&raw as *const SockAddrIn6).cast(), size) };
            bytes[..size].copy_from_slice(raw_bytes);
            size as i32
        }
    };
    RawSockAddr { bytes, len }
}

/// Decode a `sockaddr_in`/`sockaddr_in6` wire-format buffer back into a
/// [`SocketAddr`] — the reverse of [`to_sockaddr`], used by functions
/// that report a peer/local address (`accept`/`recvfrom`/`local_addr`/
/// `peer_addr`).
///
/// # Safety
///
/// `ptr` must point to at least `len` readable bytes, and (if `len` is
/// large enough to name one) a valid `sin_family`/`sin6_family` at
/// offset `0`.
pub(crate) unsafe fn from_sockaddr(
    ptr: *const u8,
    len: i32,
) -> Result<SocketAddr, crate::error::Win32Error> {
    let len = len as usize;
    if len >= core::mem::size_of::<i16>() {
        // SAFETY: `ptr` is caller-supplied per this function's own
        // safety contract, with at least `size_of::<i16>()` bytes
        // readable (just checked above).
        let family = unsafe { core::ptr::read_unaligned(ptr.cast::<i16>()) };
        if family as i32 == AddressFamily::Inet as i32 && len >= core::mem::size_of::<SockAddrIn>()
        {
            // SAFETY: `ptr` has at least `size_of::<SockAddrIn>()`
            // readable bytes, just checked above.
            let raw: SockAddrIn = unsafe { core::ptr::read_unaligned(ptr.cast()) };
            return Ok(SocketAddr::V4 {
                ip: raw.addr,
                port: u16::from_be(raw.port),
            });
        }
        if family as i32 == AddressFamily::Inet6 as i32
            && len >= core::mem::size_of::<SockAddrIn6>()
        {
            // SAFETY: `ptr` has at least `size_of::<SockAddrIn6>()`
            // readable bytes, just checked above.
            let raw: SockAddrIn6 = unsafe { core::ptr::read_unaligned(ptr.cast()) };
            return Ok(SocketAddr::V6 {
                ip: raw.addr,
                port: u16::from_be(raw.port),
                flow_info: raw.flow_info,
                scope_id: raw.scope_id,
            });
        }
    }
    Err(crate::error::Win32Error::ERROR_INVALID_PARAMETER)
}

/// Attach a local address/port to a socket — `bind`, needed before
/// [`socket`]'s result can accept incoming connections ([`SocketKind::Stream`])
/// or datagrams ([`SocketKind::Dgram`]) on a specific address, or before
/// a UDP socket sends from a fixed source port.
///
/// # Safety
///
/// `sock` must be a currently-open, valid socket from [`socket`].
pub unsafe fn bind(sock: RawSocket, addr: &SocketAddr) -> Result<(), crate::error::Win32Error> {
    let raw = to_sockaddr(addr);
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `raw` is a valid `sockaddr`-shaped buffer with `raw.len()`
    // naming its exact encoded length.
    let ok = unsafe { raw_bind(sock, raw.as_ptr(), raw.len()) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(())
    }
}

/// Mark a bound TCP socket passive/listening — `listen`, needed before
/// [`socket`]'s result (already [`bind`]-ed) can accept incoming
/// connections. `backlog` is the maximum length of the pending-
/// connection queue, passed through to `listen` unmodified — this crate
/// applies no policy (e.g. clamping) to it.
///
/// # Safety
///
/// `sock` must be a currently-open, valid, already-[`bind`]-ed
/// [`SocketKind::Stream`] socket from [`socket`].
pub unsafe fn listen(sock: RawSocket, backlog: i32) -> Result<(), crate::error::Win32Error> {
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `backlog` is a plain integer, not a pointer.
    let ok = unsafe { raw_listen(sock, backlog) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(())
    }
}

/// Accept one incoming TCP connection — `accept`, returning a new,
/// already-connected socket plus the peer's address. `sock` itself stays
/// open and listening afterward, ready to accept further connections.
///
/// # Safety
///
/// `sock` must be a currently-open, valid, already-[`listen`]-ing
/// socket from [`socket`].
pub unsafe fn accept(sock: RawSocket) -> Result<(RawSocket, SocketAddr), crate::error::Win32Error> {
    let mut buf = [0u8; 28];
    let mut addr_len: i32 = buf.len() as i32;
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `buf` is a valid buffer matched by `addr_len` naming its
    // exact capacity.
    let new_sock = unsafe { raw_accept(sock, buf.as_mut_ptr(), &mut addr_len) };
    if new_sock == INVALID_SOCKET {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        return Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ));
    }
    // SAFETY: a successful `accept` guarantees `buf` was filled with
    // `addr_len` valid bytes naming the peer's `sockaddr_in`/
    // `sockaddr_in6`.
    let peer = unsafe { from_sockaddr(buf.as_ptr(), addr_len) }?;
    Ok((new_sock, peer))
}

/// TCP client connect, or fix a UDP socket's default peer — `connect`.
/// For a [`SocketKind::Stream`] socket this actively opens a TCP
/// connection to `addr` (blocking until it succeeds or fails); for a
/// [`SocketKind::Dgram`] socket it doesn't send anything on the wire,
/// just records `addr` as the default destination [`crate::net`]'s
/// future `send`/`recv` (rather than `sendto`/`recvfrom`) calls use.
///
/// # Safety
///
/// `sock` must be a currently-open, valid socket from [`socket`].
pub unsafe fn connect(sock: RawSocket, addr: &SocketAddr) -> Result<(), crate::error::Win32Error> {
    let raw = to_sockaddr(addr);
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `raw` is a valid `sockaddr`-shaped buffer with `raw.len()`
    // naming its exact encoded length.
    let ok = unsafe { raw_connect(sock, raw.as_ptr(), raw.len()) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(())
    }
}

/// Send up to `buf.len()` bytes on a connected socket in one call —
/// `send`, the Winsock analog of `console`'s I/O shape. `sock`
/// must already be connected (a [`SocketKind::Stream`] socket from
/// [`accept`]/after [`connect`], or a [`SocketKind::Dgram`] socket with a
/// default peer set via [`connect`]) — `sendto` (a later round-2 item)
/// is the connectionless alternative for a UDP socket with no fixed
/// peer.
///
/// # Safety
///
/// `sock` must be a currently-open, valid, connected socket.
pub unsafe fn send(sock: RawSocket, buf: &[u8]) -> Result<usize, crate::error::Win32Error> {
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `buf` is a valid, `buf.len()`-byte readable buffer;
    // `flags = 0` requests ordinary blocking send behavior, this
    // module's only supported case.
    let sent = unsafe { raw_send(sock, buf.as_ptr(), buf.len() as i32, 0) };
    if sent < 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(sent as usize)
    }
}

/// Read up to `buf.len()` bytes from a connected socket in one call —
/// `recv`, the Winsock analog of [`crate::console::read`]'s shape. `Ok(0)`
/// means the peer performed an orderly shutdown (the TCP analog of
/// `ReadFile` reporting end-of-file) — not itself an error.
///
/// # Safety
///
/// `sock` must be a currently-open, valid, connected socket.
pub unsafe fn recv(sock: RawSocket, buf: &mut [u8]) -> Result<usize, crate::error::Win32Error> {
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `buf` is a valid, `buf.len()`-byte writable buffer;
    // `flags = 0` requests ordinary blocking receive behavior, this
    // module's only supported case.
    let received = unsafe { raw_recv(sock, buf.as_mut_ptr(), buf.len() as i32, 0) };
    if received < 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(received as usize)
    }
}

/// Send `buf` to `addr` on a connectionless (typically
/// [`SocketKind::Dgram`]) socket — `sendto`, the bare UDP round trip's
/// send half, marshaling `addr` into a `sockaddr_in`/`sockaddr_in6`
/// each call (unlike [`send`], which needs no address
/// since [`connect`] already fixed the peer).
///
/// # Safety
///
/// `sock` must be a currently-open, valid socket from [`socket`].
pub unsafe fn sendto(
    sock: RawSocket,
    buf: &[u8],
    addr: &SocketAddr,
) -> Result<usize, crate::error::Win32Error> {
    let raw = to_sockaddr(addr);
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `buf` is a valid, `buf.len()`-byte readable buffer;
    // `raw` is a valid `sockaddr`-shaped buffer with `raw.len()` naming
    // its exact encoded length; `flags = 0` requests ordinary blocking
    // send behavior, this module's only supported case.
    let sent = unsafe {
        raw_sendto(
            sock,
            buf.as_ptr(),
            buf.len() as i32,
            0,
            raw.as_ptr(),
            raw.len(),
        )
    };
    if sent < 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(sent as usize)
    }
}

/// Read up to `buf.len()` bytes from a connectionless (typically
/// [`SocketKind::Dgram`]) socket in one call, reporting the sender's
/// address — `recvfrom`, the bare UDP round trip's receive half. Unlike
/// [`recv`], this decodes the sender's `sockaddr_in`/`sockaddr_in6` back
/// into a [`SocketAddr`] on every call, since a
/// connectionless socket has no single fixed peer.
///
/// # Safety
///
/// `sock` must be a currently-open, valid socket from [`socket`].
pub unsafe fn recvfrom(
    sock: RawSocket,
    buf: &mut [u8],
) -> Result<(usize, SocketAddr), crate::error::Win32Error> {
    let mut from_buf = [0u8; 28];
    let mut from_len: i32 = from_buf.len() as i32;
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `buf` is a valid, `buf.len()`-byte writable buffer;
    // `from_buf` is a valid buffer matched by `from_len` naming its
    // exact capacity; `flags = 0` requests ordinary blocking receive
    // behavior, this module's only supported case.
    let received = unsafe {
        raw_recvfrom(
            sock,
            buf.as_mut_ptr(),
            buf.len() as i32,
            0,
            from_buf.as_mut_ptr(),
            &mut from_len,
        )
    };
    if received < 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        return Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ));
    }
    // SAFETY: a successful `recvfrom` guarantees `from_buf` was filled
    // with `from_len` valid bytes naming the sender's `sockaddr_in`/
    // `sockaddr_in6`.
    let sender = unsafe { from_sockaddr(from_buf.as_ptr(), from_len) }?;
    Ok((received as usize, sender))
}

/// `SD_RECEIVE`/`SD_SEND`/`SD_BOTH` — which direction(s) [`shutdown`]
/// closes. Verified against mingw-w64's own `winsock2.h` with a compiled
/// `_Static_assert` probe.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownHow {
    Receive = 0,
    Send = 1,
    Both = 2,
}

/// Half-close a TCP socket's send and/or receive direction — `shutdown`,
/// for a clean FIN before [`close_socket`]. Unlike `close_socket`
/// itself, this leaves `sock` open (still a valid handle, still needing
/// its own eventual `close_socket`) — it only signals the peer that no
/// more data is coming (`ShutdownHow::Send`), stops accepting further
/// reads (`ShutdownHow::Receive`), or both.
///
/// # Safety
///
/// `sock` must be a currently-open, valid, connected socket.
pub unsafe fn shutdown(sock: RawSocket, how: ShutdownHow) -> Result<(), crate::error::Win32Error> {
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `how` is a plain enum-backed integer value, not a
    // pointer.
    let ok = unsafe { raw_shutdown(sock, how as i32) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(())
    }
}

/// `SOL_SOCKET` — the socket-level option level [`set_sockopt`]/
/// [`get_sockopt`] use for every option except [`SockOpt::TcpNoDelay`]/
/// [`SockOptKind::TcpNoDelay`] (which is `IPPROTO_TCP`-level instead).
/// Verified against mingw-w64's own `winsock2.h` with a compiled
/// `_Static_assert` probe.
const SOL_SOCKET: i32 = 0xffff;

/// `SO_REUSEADDR`/`SO_RCVTIMEO`/`SO_SNDTIMEO`/`SO_ERROR` — the
/// `SOL_SOCKET`-level option numbers this module supports. Verified
/// against mingw-w64's own `winsock2.h` with a compiled `_Static_assert`
/// probe.
const SO_REUSEADDR: i32 = 0x0004;
const SO_RCVTIMEO: i32 = 0x1006;
const SO_SNDTIMEO: i32 = 0x1005;
const SO_ERROR: i32 = 0x1007;

/// `TCP_NODELAY` — the one `IPPROTO_TCP`-level option this module
/// supports. Verified against mingw-w64's own `winsock2.h` with a
/// compiled `_Static_assert` probe.
const TCP_NODELAY: i32 = 0x0001;

/// An option settable via [`set_sockopt`]. Every variant here is a plain
/// 4-byte `BOOL`/`DWORD` on the wire, unlike POSIX's `timeval`-based
/// `SO_RCVTIMEO`/`SO_SNDTIMEO` — Windows takes a plain millisecond
/// `DWORD` for both instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SockOpt {
    /// `SO_REUSEADDR` — allow [`bind`] to succeed on a local
    /// address/port still lingering in `TIME_WAIT` from a previous
    /// socket.
    ReuseAddr(bool),
    /// `SO_RCVTIMEO` — the blocking-[`recv`]/[`recvfrom`] timeout, in
    /// milliseconds. `0` (the default) means block forever.
    RecvTimeout(u32),
    /// `SO_SNDTIMEO` — the blocking-[`send`]/[`sendto`] timeout, in
    /// milliseconds. `0` (the default) means block forever.
    SendTimeout(u32),
    /// `TCP_NODELAY` (`IPPROTO_TCP` level) — disable Nagle's algorithm,
    /// so small writes go out immediately instead of being batched.
    TcpNoDelay(bool),
}

/// Which option [`get_sockopt`] reports — see [`SockOptValue`] for what
/// each kind returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SockOptKind {
    ReuseAddr,
    RecvTimeout,
    SendTimeout,
    TcpNoDelay,
    /// `SO_ERROR` — the socket's pending error status. Reading it also
    /// clears it (a Winsock-documented side effect of this particular
    /// option, not something this crate adds).
    Error,
}

/// [`get_sockopt`]'s result — the value's shape depends on which
/// [`SockOptKind`] was queried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SockOptValue {
    Bool(bool),
    Millis(u32),
    ErrorCode(i32),
}

/// Set a socket option — `setsockopt`.
///
/// # Safety
///
/// `sock` must be a currently-open, valid socket from [`socket`].
pub unsafe fn set_sockopt(sock: RawSocket, opt: SockOpt) -> Result<(), crate::error::Win32Error> {
    let (level, optname, bytes): (i32, i32, [u8; 4]) = match opt {
        SockOpt::ReuseAddr(on) => (SOL_SOCKET, SO_REUSEADDR, (on as i32).to_ne_bytes()),
        SockOpt::RecvTimeout(ms) => (SOL_SOCKET, SO_RCVTIMEO, ms.to_ne_bytes()),
        SockOpt::SendTimeout(ms) => (SOL_SOCKET, SO_SNDTIMEO, ms.to_ne_bytes()),
        SockOpt::TcpNoDelay(on) => (Protocol::Tcp as i32, TCP_NODELAY, (on as i32).to_ne_bytes()),
    };
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `bytes` is a valid 4-byte buffer, matching the
    // `BOOL`/`DWORD` width every option above uses, with its exact
    // length passed as `optlen`.
    let ok = unsafe { setsockopt(sock, level, optname, bytes.as_ptr(), bytes.len() as i32) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(())
    }
}

/// Read a socket option — `getsockopt`.
///
/// # Safety
///
/// `sock` must be a currently-open, valid socket from [`socket`].
pub unsafe fn get_sockopt(
    sock: RawSocket,
    kind: SockOptKind,
) -> Result<SockOptValue, crate::error::Win32Error> {
    let (level, optname) = match kind {
        SockOptKind::ReuseAddr => (SOL_SOCKET, SO_REUSEADDR),
        SockOptKind::RecvTimeout => (SOL_SOCKET, SO_RCVTIMEO),
        SockOptKind::SendTimeout => (SOL_SOCKET, SO_SNDTIMEO),
        SockOptKind::TcpNoDelay => (Protocol::Tcp as i32, TCP_NODELAY),
        SockOptKind::Error => (SOL_SOCKET, SO_ERROR),
    };
    let mut bytes = [0u8; 4];
    let mut optlen: i32 = bytes.len() as i32;
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `bytes` is a valid 4-byte buffer matched by `optlen`
    // naming its exact capacity.
    let ok = unsafe { getsockopt(sock, level, optname, bytes.as_mut_ptr(), &mut optlen) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        return Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ));
    }
    let raw = i32::from_ne_bytes(bytes);
    Ok(match kind {
        SockOptKind::ReuseAddr | SockOptKind::TcpNoDelay => SockOptValue::Bool(raw != 0),
        SockOptKind::RecvTimeout | SockOptKind::SendTimeout => SockOptValue::Millis(raw as u32),
        SockOptKind::Error => SockOptValue::ErrorCode(raw),
    })
}

/// Read back a socket's own local bound address — `getsockname`. Useful
/// after [`bind`]-ing to port `0` (an OS-assigned ephemeral port) to
/// discover which port was actually chosen.
///
/// # Safety
///
/// `sock` must be a currently-open, valid, already-[`bind`]-ed socket
/// from [`socket`].
pub unsafe fn local_addr(sock: RawSocket) -> Result<SocketAddr, crate::error::Win32Error> {
    let mut buf = [0u8; 28];
    let mut addr_len: i32 = buf.len() as i32;
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `buf` is a valid buffer matched by `addr_len` naming its
    // exact capacity.
    let ok = unsafe { getsockname(sock, buf.as_mut_ptr(), &mut addr_len) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        return Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ));
    }
    // SAFETY: a successful `getsockname` guarantees `buf` was filled with
    // `addr_len` valid bytes naming the local `sockaddr_in`/
    // `sockaddr_in6`.
    unsafe { from_sockaddr(buf.as_ptr(), addr_len) }
}

/// Read a connected socket's peer address — `getpeername`.
///
/// # Safety
///
/// `sock` must be a currently-open, valid, connected socket (a
/// [`SocketKind::Stream`] socket from [`accept`]/after [`connect`], or a
/// [`SocketKind::Dgram`] socket with a default peer set via [`connect`]).
pub unsafe fn peer_addr(sock: RawSocket) -> Result<SocketAddr, crate::error::Win32Error> {
    let mut buf = [0u8; 28];
    let mut addr_len: i32 = buf.len() as i32;
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `buf` is a valid buffer matched by `addr_len` naming its
    // exact capacity.
    let ok = unsafe { getpeername(sock, buf.as_mut_ptr(), &mut addr_len) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        return Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ));
    }
    // SAFETY: a successful `getpeername` guarantees `buf` was filled with
    // `addr_len` valid bytes naming the peer's `sockaddr_in`/
    // `sockaddr_in6`.
    unsafe { from_sockaddr(buf.as_ptr(), addr_len) }
}

// addrinfo (64-bit layout, per mingw-w64's own `ws2tcpip.h`): `size_of`
// 48, 8-byte aligned, every field's offset verified with a compiled
// `_Static_assert` probe. `ai_canonname` is never read by this module
// (this crate requests no `AI_CANONNAME` flag) -- kept only so the
// struct's layout matches Windows' own exactly.
#[repr(C)]
struct AddrInfoRaw {
    ai_flags: i32,
    ai_family: i32,
    ai_socktype: i32,
    ai_protocol: i32,
    ai_addrlen: usize,
    ai_canonname: *mut u8,
    ai_addr: *mut u8,
    ai_next: *mut AddrInfoRaw,
}
const _: () = assert!(core::mem::size_of::<AddrInfoRaw>() == 48);

fn address_family_from_raw(value: i32) -> Option<AddressFamily> {
    match value {
        v if v == AddressFamily::Inet as i32 => Some(AddressFamily::Inet),
        v if v == AddressFamily::Inet6 as i32 => Some(AddressFamily::Inet6),
        _ => None,
    }
}

fn socket_kind_from_raw(value: i32) -> Option<SocketKind> {
    match value {
        v if v == SocketKind::Stream as i32 => Some(SocketKind::Stream),
        v if v == SocketKind::Dgram as i32 => Some(SocketKind::Dgram),
        _ => None,
    }
}

fn protocol_from_raw(value: i32) -> Option<Protocol> {
    match value {
        v if v == Protocol::Tcp as i32 => Some(Protocol::Tcp),
        v if v == Protocol::Udp as i32 => Some(Protocol::Udp),
        _ => None,
    }
}

/// Hints narrowing [`resolve`]'s query — mirrors the subset of
/// `addrinfo`'s input fields `getaddrinfo` reads. `None` in any field
/// means "any" (`AF_UNSPEC`/`0`), matching a zeroed `hints` struct with
/// no flags set — this module requests no `AI_*` flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AddrInfoHints {
    pub family: Option<AddressFamily>,
    pub socktype: Option<SocketKind>,
    pub protocol: Option<Protocol>,
}

/// One address `getaddrinfo` returned via [`resolve`]. `getaddrinfo`
/// itself can return other address families/socket types/protocols than
/// the ones this module supports (a mismatched hint, or `AF_UNSPEC`
/// turning up something exotic); [`resolve`] silently skips any entry
/// it can't represent, the same "explicitly out of scope" policy this
/// module's other enums already apply at the `socket`/`bind` boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedAddr {
    pub family: AddressFamily,
    pub socktype: SocketKind,
    pub protocol: Protocol,
    pub addr: SocketAddr,
}

/// Resolve a host and service name to one or more addresses —
/// `getaddrinfo`, walking Windows' own returned linked list and copying
/// every entry out before `freeaddrinfo`-ing it (so nothing borrowed from
/// Windows-owned memory escapes this call). `host`/`service` may each be
/// a numeric address/port (parsed directly, no name resolution
/// attempted) or a real hostname/service name (resolved via DNS/the
/// local services database, potentially blocking).
///
/// Reports failure via its own return value directly — Windows' `EAI_*`
/// codes are aliases of the ordinary `WSA*` error codes (see
/// `ws2tcpip.h`), so no separate `WSAGetLastError` call is needed, the
/// same convention [`startup`] documents for `WSAStartup`.
pub fn resolve(
    host: &str,
    service: &str,
    hints: &AddrInfoHints,
) -> Result<Vec<ResolvedAddr>, crate::error::Win32Error> {
    let host_cstr: Vec<u8> = host.bytes().chain(core::iter::once(0)).collect();
    let service_cstr: Vec<u8> = service.bytes().chain(core::iter::once(0)).collect();

    let raw_hints = AddrInfoRaw {
        ai_flags: 0,
        ai_family: hints.family.map_or(0, |f| f as i32),
        ai_socktype: hints.socktype.map_or(0, |k| k as i32),
        ai_protocol: hints.protocol.map_or(0, |p| p as i32),
        ai_addrlen: 0,
        ai_canonname: core::ptr::null_mut(),
        ai_addr: core::ptr::null_mut(),
        ai_next: core::ptr::null_mut(),
    };

    let mut result_head: *mut AddrInfoRaw = core::ptr::null_mut();
    // SAFETY: `host_cstr`/`service_cstr` are valid, nul-terminated byte
    // buffers; `raw_hints` is a valid, fully-initialized `addrinfo` used
    // only for its input fields; `result_head` is a valid out-pointer.
    let status = unsafe {
        getaddrinfo(
            host_cstr.as_ptr(),
            service_cstr.as_ptr(),
            &raw_hints,
            &mut result_head,
        )
    };
    if status != 0 {
        return Err(crate::error::Win32Error::from_raw(status as u32));
    }

    let mut results = Vec::new();
    let mut node = result_head;
    while !node.is_null() {
        // SAFETY: `node` is non-null, produced by the successful
        // `getaddrinfo` call above -- Windows guarantees each linked-list
        // entry is a fully-initialized `addrinfo` until `freeaddrinfo`.
        let entry = unsafe { &*node };
        if let (Some(family), Some(socktype), Some(protocol)) = (
            address_family_from_raw(entry.ai_family),
            socket_kind_from_raw(entry.ai_socktype),
            protocol_from_raw(entry.ai_protocol),
        ) {
            // SAFETY: `entry.ai_addr` points to `entry.ai_addrlen` valid
            // bytes naming a `sockaddr_in`/`sockaddr_in6`, per
            // `getaddrinfo`'s own contract.
            if let Ok(addr) = unsafe { from_sockaddr(entry.ai_addr, entry.ai_addrlen as i32) } {
                results.push(ResolvedAddr {
                    family,
                    socktype,
                    protocol,
                    addr,
                });
            }
        }
        node = entry.ai_next;
    }

    // SAFETY: `result_head` is exactly the pointer `getaddrinfo` returned
    // on success above, not yet freed.
    unsafe { freeaddrinfo(result_head) };

    Ok(results)
}

/// Convert a 16-bit value from host to network (big-endian) byte order —
/// `htons`. This module's own `to_sockaddr` helper already applies this
/// conversion internally to every port it encodes; this standalone
/// wrapper is for callers building/parsing their own raw wire fields
/// (e.g. an application-level protocol) rather than going through
/// [`SocketAddr`].
pub fn htons(hostshort: u16) -> u16 {
    // SAFETY: `htons` is a pure, side-effect-free byte-swap with no
    // preconditions -- unlike every other function in this module, it
    // doesn't touch a socket or need `startup` to have been called
    // first.
    unsafe { raw_htons(hostshort) }
}

/// Convert a 32-bit value from host to network (big-endian) byte order —
/// `htonl`. Same use case as [`htons`], for a raw 32-bit wire field (a
/// packed IPv4 address, a sequence number, …) rather than a 16-bit port.
pub fn htonl(hostlong: u32) -> u32 {
    // SAFETY: same reasoning as `htons` above.
    unsafe { raw_htonl(hostlong) }
}

/// Convert a 16-bit value from network (big-endian) to host byte order —
/// `ntohs`, the reverse of [`htons`].
pub fn ntohs(netshort: u16) -> u16 {
    // SAFETY: same reasoning as `htons` above.
    unsafe { raw_ntohs(netshort) }
}

/// Convert a 32-bit value from network (big-endian) to host byte order —
/// `ntohl`, the reverse of [`htonl`].
pub fn ntohl(netlong: u32) -> u32 {
    // SAFETY: same reasoning as `htons` above.
    unsafe { raw_ntohl(netlong) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_then_cleanup_round_trips() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");
        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn nested_startup_cleanup_pairs_are_reference_counted() {
        // Windows documents WSAStartup/WSACleanup as reference-counted:
        // two startups followed by two cleanups should both succeed,
        // rather than the second cleanup failing once the "real" count
        // has already reached zero after the first.
        startup().expect("first WSAStartup should succeed");
        startup().expect("nested WSAStartup should also succeed");
        cleanup().expect("first WSACleanup should succeed");
        cleanup().expect("second WSACleanup should succeed, matching the nested startup");
    }

    #[test]
    fn socket_then_close_socket_round_trips_for_tcp_and_udp() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let tcp = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating a TCP/IPv4 socket");
        // SAFETY: `tcp` was just created above and hasn't been closed
        // yet.
        unsafe { close_socket(tcp) }
            .expect("closesocket should succeed on a freshly created socket");

        let udp = socket(AddressFamily::Inet, SocketKind::Dgram, Protocol::Udp)
            .expect("socket should succeed creating a UDP/IPv4 socket");
        // SAFETY: `udp` was just created above and hasn't been closed
        // yet.
        unsafe { close_socket(udp) }
            .expect("closesocket should succeed on a freshly created socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn socket_supports_ipv6() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let sock = socket(AddressFamily::Inet6, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating a TCP/IPv6 socket");
        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { close_socket(sock) }
            .expect("closesocket should succeed on a freshly created socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn to_sockaddr_then_from_sockaddr_round_trips_an_ipv4_address() {
        let addr = SocketAddr::V4 {
            ip: [127, 0, 0, 1],
            port: 8080,
        };
        let raw = to_sockaddr(&addr);
        assert_eq!(raw.len(), 16, "an encoded IPv4 sockaddr should be 16 bytes");

        // SAFETY: `raw` was just filled with exactly `raw.len()` valid
        // bytes above.
        let decoded = unsafe { from_sockaddr(raw.as_ptr(), raw.len()) }
            .expect("decoding a just-encoded sockaddr should succeed");
        assert_eq!(decoded, addr);
    }

    #[test]
    fn to_sockaddr_then_from_sockaddr_round_trips_an_ipv6_address() {
        let addr = SocketAddr::V6 {
            ip: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            port: 9090,
            flow_info: 0,
            scope_id: 0,
        };
        let raw = to_sockaddr(&addr);
        assert_eq!(raw.len(), 28, "an encoded IPv6 sockaddr should be 28 bytes");

        // SAFETY: `raw` was just filled with exactly `raw.len()` valid
        // bytes above.
        let decoded = unsafe { from_sockaddr(raw.as_ptr(), raw.len()) }
            .expect("decoding a just-encoded sockaddr should succeed");
        assert_eq!(decoded, addr);
    }

    #[test]
    fn to_sockaddr_stores_the_port_in_network_byte_order() {
        let addr = SocketAddr::V4 {
            ip: [10, 0, 0, 1],
            port: 0x1234,
        };
        let raw = to_sockaddr(&addr);
        // SAFETY: `raw` was just filled with exactly `raw.len()` valid
        // bytes above; `sin_port` is at byte offset 2 in `sockaddr_in`.
        let port_bytes = unsafe { core::slice::from_raw_parts(raw.as_ptr().add(2), 2) };
        assert_eq!(
            port_bytes,
            &[0x12, 0x34],
            "sin_port should be big-endian (network byte order)"
        );
    }

    #[test]
    fn from_sockaddr_fails_for_an_unrecognized_address_family() {
        let bytes = [0u8; 16];
        // SAFETY: `bytes` is a valid 16-byte buffer; its first two bytes
        // (`sin_family`, all zero) don't match `AF_INET`/`AF_INET6`.
        let err = unsafe { from_sockaddr(bytes.as_ptr(), bytes.len() as i32) }
            .expect_err("from_sockaddr should fail for an unrecognized address family");
        assert_eq!(err, crate::error::Win32Error::ERROR_INVALID_PARAMETER);
    }

    #[test]
    fn bind_then_listen_succeeds_on_a_loopback_tcp_socket() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let sock = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating a TCP/IPv4 socket");
        let addr = SocketAddr::V4 {
            ip: [127, 0, 0, 1],
            // Port 0 asks Windows to assign any free ephemeral port --
            // this test only needs bind/listen to succeed, not a
            // specific port number.
            port: 0,
        };
        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { bind(sock, &addr) }.expect("bind should succeed on 127.0.0.1:0");
        // SAFETY: `sock` is still open, now bound.
        unsafe { listen(sock, 5) }.expect("listen should succeed on a freshly bound TCP socket");

        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { close_socket(sock) }
            .expect("closesocket should succeed on a freshly created socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn accept_returns_a_connected_socket_and_the_peers_address() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let sock = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating a TCP/IPv4 socket");
        // A fixed (not port-0/ephemeral) port -- this crate doesn't have
        // `connect`/`getsockname` yet (later round-2 items), so the test's
        // `std::net::TcpStream` client below needs a port number it can
        // already know in advance.
        const TEST_PORT: u16 = 47950;
        let addr = SocketAddr::V4 {
            ip: [127, 0, 0, 1],
            port: TEST_PORT,
        };
        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { bind(sock, &addr) }.expect("bind should succeed on 127.0.0.1:TEST_PORT");
        // SAFETY: `sock` is still open, now bound.
        unsafe { listen(sock, 1) }.expect("listen should succeed on a freshly bound TCP socket");

        // A real client connection, via `std::net` (always linked in
        // this test harness) rather than this crate's own `connect`
        // (not yet implemented) -- run on a background thread since
        // `accept` below blocks until a connection arrives.
        let client_thread = std::thread::spawn(move || {
            std::net::TcpStream::connect(("127.0.0.1", TEST_PORT))
                .expect("the std::net client should succeed connecting to our listening socket")
        });

        // SAFETY: `sock` is open and listening from the calls above.
        let (new_sock, peer) =
            unsafe { accept(sock) }.expect("accept should succeed once the client connects");
        let _client = client_thread
            .join()
            .expect("the client thread should not panic");

        match peer {
            SocketAddr::V4 { ip, .. } => {
                assert_eq!(ip, [127, 0, 0, 1], "the peer's address should be loopback")
            }
            SocketAddr::V6 { .. } => panic!("expected an IPv4 peer address, got: {peer:?}"),
        }

        // SAFETY: `new_sock`/`sock` were both just created/opened above
        // and haven't been closed yet.
        unsafe { close_socket(new_sock) }
            .expect("closesocket should succeed on the accepted socket");
        unsafe { close_socket(sock) }.expect("closesocket should succeed on the listening socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn connect_then_accept_completes_a_full_local_tcp_handshake() {
        // Unlike `accept_returns_a_connected_socket_and_the_peers_address`
        // (which needed `std::net::TcpStream` as a stand-in client since
        // this crate had no `connect` yet), this test uses only this
        // crate's own primitives on both ends of the connection.
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let server = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating the server's TCP/IPv4 socket");
        const TEST_PORT: u16 = 47951;
        let addr = SocketAddr::V4 {
            ip: [127, 0, 0, 1],
            port: TEST_PORT,
        };
        // SAFETY: `server` was just created above and hasn't been closed
        // yet.
        unsafe { bind(server, &addr) }.expect("bind should succeed on 127.0.0.1:TEST_PORT");
        // SAFETY: `server` is still open, now bound.
        unsafe { listen(server, 1) }
            .expect("listen should succeed on the freshly bound server socket");

        // `accept` blocks until a connection arrives, so it runs on a
        // background thread while the client connects below.
        let accept_thread = std::thread::spawn(move || {
            // SAFETY: `server` is open and listening from the calls
            // above, for the whole lifetime of this thread.
            unsafe { accept(server) }
        });

        let client = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating the client's TCP/IPv4 socket");
        // SAFETY: `client` was just created above and hasn't been closed
        // yet.
        unsafe { connect(client, &addr) }
            .expect("connect should succeed reaching the listening server socket");

        let (accepted, peer) = accept_thread
            .join()
            .expect("the accept thread should not panic")
            .expect("accept should succeed once the client connects");
        match peer {
            SocketAddr::V4 { ip, .. } => {
                assert_eq!(ip, [127, 0, 0, 1], "the peer's address should be loopback")
            }
            SocketAddr::V6 { .. } => panic!("expected an IPv4 peer address, got: {peer:?}"),
        }

        // SAFETY: `client`/`accepted`/`server` were all just
        // created/opened above and haven't been closed yet.
        unsafe { close_socket(client) }.expect("closesocket should succeed on the client socket");
        unsafe { close_socket(accepted) }
            .expect("closesocket should succeed on the accepted socket");
        unsafe { close_socket(server) }.expect("closesocket should succeed on the server socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn send_then_recv_carries_bytes_over_a_local_tcp_connection() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let server = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating the server's TCP/IPv4 socket");
        const TEST_PORT: u16 = 47952;
        let addr = SocketAddr::V4 {
            ip: [127, 0, 0, 1],
            port: TEST_PORT,
        };
        // SAFETY: `server` was just created above and hasn't been closed
        // yet.
        unsafe { bind(server, &addr) }.expect("bind should succeed on 127.0.0.1:TEST_PORT");
        // SAFETY: `server` is still open, now bound.
        unsafe { listen(server, 1) }
            .expect("listen should succeed on the freshly bound server socket");

        let accept_thread = std::thread::spawn(move || {
            // SAFETY: `server` is open and listening from the calls
            // above, for the whole lifetime of this thread.
            unsafe { accept(server) }
        });

        let client = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating the client's TCP/IPv4 socket");
        // SAFETY: `client` was just created above and hasn't been closed
        // yet.
        unsafe { connect(client, &addr) }
            .expect("connect should succeed reaching the listening server socket");

        let (accepted, _peer) = accept_thread
            .join()
            .expect("the accept thread should not panic")
            .expect("accept should succeed once the client connects");

        const MESSAGE: &[u8] = b"hello over rusty_win32 net::send/recv";
        // SAFETY: `client` is connected from the calls above.
        let sent = unsafe { send(client, MESSAGE) }.expect("send should succeed on the client");
        assert_eq!(
            sent,
            MESSAGE.len(),
            "send should report the full message written"
        );

        let mut buf = [0u8; MESSAGE.len()];
        // SAFETY: `accepted` is a valid, connected socket from `accept`
        // above.
        let received =
            unsafe { recv(accepted, &mut buf) }.expect("recv should succeed on the accepted end");
        assert_eq!(
            received,
            MESSAGE.len(),
            "recv should report the full message read"
        );
        assert_eq!(&buf[..received], MESSAGE);

        // SAFETY: `client`/`accepted`/`server` were all just
        // created/opened above and haven't been closed yet.
        unsafe { close_socket(client) }.expect("closesocket should succeed on the client socket");
        unsafe { close_socket(accepted) }
            .expect("closesocket should succeed on the accepted socket");
        unsafe { close_socket(server) }.expect("closesocket should succeed on the server socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn sendto_then_recvfrom_carries_a_datagram_and_the_senders_address() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let receiver = socket(AddressFamily::Inet, SocketKind::Dgram, Protocol::Udp)
            .expect("socket should succeed creating the receiver's UDP/IPv4 socket");
        const RECEIVER_PORT: u16 = 47953;
        const SENDER_PORT: u16 = 47954;
        let receiver_addr = SocketAddr::V4 {
            ip: [127, 0, 0, 1],
            port: RECEIVER_PORT,
        };
        let sender_addr = SocketAddr::V4 {
            ip: [127, 0, 0, 1],
            port: SENDER_PORT,
        };
        // SAFETY: `receiver` was just created above and hasn't been
        // closed yet.
        unsafe { bind(receiver, &receiver_addr) }
            .expect("bind should succeed on the receiver's fixed loopback port");

        let sender = socket(AddressFamily::Inet, SocketKind::Dgram, Protocol::Udp)
            .expect("socket should succeed creating the sender's UDP/IPv4 socket");
        // Binding the sender to its own fixed port (rather than an
        // ephemeral one) lets this test assert the exact source port
        // `recvfrom` reports, without needing `getsockname` (not yet
        // implemented -- a later round-2 item).
        // SAFETY: `sender` was just created above and hasn't been closed
        // yet.
        unsafe { bind(sender, &sender_addr) }
            .expect("bind should succeed on the sender's fixed loopback port");

        const MESSAGE: &[u8] = b"hello over rusty_win32 net::sendto/recvfrom";
        // SAFETY: `sender` is bound from the call above.
        let sent = unsafe { sendto(sender, MESSAGE, &receiver_addr) }
            .expect("sendto should succeed targeting the receiver's bound address");
        assert_eq!(
            sent,
            MESSAGE.len(),
            "sendto should report the full datagram written"
        );

        let mut buf = [0u8; MESSAGE.len()];
        // SAFETY: `receiver` is bound from the call above.
        let (received, from) = unsafe { recvfrom(receiver, &mut buf) }
            .expect("recvfrom should succeed reading the datagram just sent");
        assert_eq!(
            received,
            MESSAGE.len(),
            "recvfrom should report the full datagram read"
        );
        assert_eq!(&buf[..received], MESSAGE);
        assert_eq!(
            from, sender_addr,
            "recvfrom should report the sender's own bound address"
        );

        // SAFETY: `sender`/`receiver` were both just created/opened
        // above and haven't been closed yet.
        unsafe { close_socket(sender) }.expect("closesocket should succeed on the sender socket");
        unsafe { close_socket(receiver) }
            .expect("closesocket should succeed on the receiver socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn shutdown_send_causes_the_peer_to_see_a_clean_end_of_stream() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let server = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating the server's TCP/IPv4 socket");
        const TEST_PORT: u16 = 47955;
        let addr = SocketAddr::V4 {
            ip: [127, 0, 0, 1],
            port: TEST_PORT,
        };
        // SAFETY: `server` was just created above and hasn't been closed
        // yet.
        unsafe { bind(server, &addr) }.expect("bind should succeed on 127.0.0.1:TEST_PORT");
        // SAFETY: `server` is still open, now bound.
        unsafe { listen(server, 1) }
            .expect("listen should succeed on the freshly bound server socket");

        let accept_thread = std::thread::spawn(move || {
            // SAFETY: `server` is open and listening from the calls
            // above, for the whole lifetime of this thread.
            unsafe { accept(server) }
        });

        let client = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating the client's TCP/IPv4 socket");
        // SAFETY: `client` was just created above and hasn't been closed
        // yet.
        unsafe { connect(client, &addr) }
            .expect("connect should succeed reaching the listening server socket");

        let (accepted, _peer) = accept_thread
            .join()
            .expect("the accept thread should not panic")
            .expect("accept should succeed once the client connects");

        // SAFETY: `client` is connected from the calls above.
        unsafe { shutdown(client, ShutdownHow::Send) }
            .expect("shutdown(Send) should succeed on the connected client socket");

        let mut buf = [0u8; 16];
        // SAFETY: `accepted` is a valid, connected socket from `accept`
        // above.
        let received = unsafe { recv(accepted, &mut buf) }
            .expect("recv should succeed reading end-of-stream after the peer's shutdown(Send)");
        assert_eq!(
            received, 0,
            "recv should report 0 bytes once the peer has shut down its send direction"
        );

        // SAFETY: `client`/`accepted`/`server` were all just
        // created/opened above and haven't been closed yet.
        unsafe { close_socket(client) }.expect("closesocket should succeed on the client socket");
        unsafe { close_socket(accepted) }
            .expect("closesocket should succeed on the accepted socket");
        unsafe { close_socket(server) }.expect("closesocket should succeed on the server socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn set_sockopt_reuse_addr_then_get_sockopt_round_trips() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let sock = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating a TCP/IPv4 socket");

        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { set_sockopt(sock, SockOpt::ReuseAddr(true)) }
            .expect("set_sockopt(ReuseAddr(true)) should succeed");
        // SAFETY: `sock` is still open from the call above.
        let value = unsafe { get_sockopt(sock, SockOptKind::ReuseAddr) }
            .expect("get_sockopt(ReuseAddr) should succeed");
        assert_eq!(value, SockOptValue::Bool(true));

        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { close_socket(sock) }
            .expect("closesocket should succeed on a freshly created socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn set_sockopt_tcp_nodelay_then_get_sockopt_round_trips() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let sock = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating a TCP/IPv4 socket");

        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { set_sockopt(sock, SockOpt::TcpNoDelay(true)) }
            .expect("set_sockopt(TcpNoDelay(true)) should succeed");
        // SAFETY: `sock` is still open from the call above.
        let value = unsafe { get_sockopt(sock, SockOptKind::TcpNoDelay) }
            .expect("get_sockopt(TcpNoDelay) should succeed");
        assert_eq!(value, SockOptValue::Bool(true));

        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { close_socket(sock) }
            .expect("closesocket should succeed on a freshly created socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn set_sockopt_recv_timeout_then_get_sockopt_round_trips_in_milliseconds() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let sock = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating a TCP/IPv4 socket");

        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { set_sockopt(sock, SockOpt::RecvTimeout(250)) }
            .expect("set_sockopt(RecvTimeout(250)) should succeed");
        // SAFETY: `sock` is still open from the call above.
        let value = unsafe { get_sockopt(sock, SockOptKind::RecvTimeout) }
            .expect("get_sockopt(RecvTimeout) should succeed");
        assert_eq!(
            value,
            SockOptValue::Millis(250),
            "SO_RCVTIMEO should round-trip as a plain millisecond DWORD, not a timeval"
        );

        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { close_socket(sock) }
            .expect("closesocket should succeed on a freshly created socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn get_sockopt_error_reports_zero_for_a_healthy_socket() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let sock = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating a TCP/IPv4 socket");

        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        let value = unsafe { get_sockopt(sock, SockOptKind::Error) }
            .expect("get_sockopt(Error) should succeed on a healthy socket");
        assert_eq!(value, SockOptValue::ErrorCode(0));

        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { close_socket(sock) }
            .expect("closesocket should succeed on a freshly created socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn local_addr_reports_the_bound_port_after_binding_to_an_ephemeral_port() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let sock = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating a TCP/IPv4 socket");
        let addr = SocketAddr::V4 {
            ip: [127, 0, 0, 1],
            // Port 0 asks Windows to assign any free ephemeral port --
            // this test verifies local_addr can discover which one was
            // actually chosen.
            port: 0,
        };
        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { bind(sock, &addr) }.expect("bind should succeed on 127.0.0.1:0");

        // SAFETY: `sock` is still open, now bound.
        let bound =
            unsafe { local_addr(sock) }.expect("local_addr should succeed on a bound socket");
        match bound {
            SocketAddr::V4 { ip, port } => {
                assert_eq!(ip, [127, 0, 0, 1], "the bound address should be loopback");
                assert_ne!(
                    port, 0,
                    "local_addr should report the OS-assigned ephemeral port, not 0"
                );
            }
            SocketAddr::V6 { .. } => panic!("expected an IPv4 local address, got: {bound:?}"),
        }

        // SAFETY: `sock` was just created above and hasn't been closed
        // yet.
        unsafe { close_socket(sock) }
            .expect("closesocket should succeed on a freshly created socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn peer_addr_reports_the_connected_peers_address() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let server = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating the server's TCP/IPv4 socket");
        const TEST_PORT: u16 = 47956;
        let addr = SocketAddr::V4 {
            ip: [127, 0, 0, 1],
            port: TEST_PORT,
        };
        // SAFETY: `server` was just created above and hasn't been closed
        // yet.
        unsafe { bind(server, &addr) }.expect("bind should succeed on 127.0.0.1:TEST_PORT");
        // SAFETY: `server` is still open, now bound.
        unsafe { listen(server, 1) }
            .expect("listen should succeed on the freshly bound server socket");

        let accept_thread = std::thread::spawn(move || {
            // SAFETY: `server` is open and listening from the calls
            // above, for the whole lifetime of this thread.
            unsafe { accept(server) }
        });

        let client = socket(AddressFamily::Inet, SocketKind::Stream, Protocol::Tcp)
            .expect("socket should succeed creating the client's TCP/IPv4 socket");
        // SAFETY: `client` was just created above and hasn't been closed
        // yet.
        unsafe { connect(client, &addr) }
            .expect("connect should succeed reaching the listening server socket");

        let (accepted, _peer) = accept_thread
            .join()
            .expect("the accept thread should not panic")
            .expect("accept should succeed once the client connects");

        // SAFETY: `client` is connected from the call above.
        let client_peer =
            unsafe { peer_addr(client) }.expect("peer_addr should succeed on a connected socket");
        assert_eq!(
            client_peer, addr,
            "the client's peer address should be the server's bound address"
        );

        // SAFETY: `accepted` is bound (inheriting the listening socket's
        // local address) from `accept` above.
        let accepted_local = unsafe { local_addr(accepted) }
            .expect("local_addr should succeed on the accepted socket");
        assert_eq!(
            accepted_local, addr,
            "the accepted socket's local address should match the listening socket's bound address"
        );

        // SAFETY: `client`/`accepted`/`server` were all just
        // created/opened above and haven't been closed yet.
        unsafe { close_socket(client) }.expect("closesocket should succeed on the client socket");
        unsafe { close_socket(accepted) }
            .expect("closesocket should succeed on the accepted socket");
        unsafe { close_socket(server) }.expect("closesocket should succeed on the server socket");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn resolve_returns_the_parsed_address_for_a_numeric_ipv4_host_and_service() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let hints = AddrInfoHints {
            family: Some(AddressFamily::Inet),
            socktype: Some(SocketKind::Stream),
            protocol: Some(Protocol::Tcp),
        };
        // A numeric host/port needs no real name resolution -- getaddrinfo
        // parses it directly, keeping this test deterministic and
        // network-independent.
        let results = resolve("127.0.0.1", "8080", &hints)
            .expect("resolve should succeed for a numeric host/service");
        assert!(
            !results.is_empty(),
            "resolve should return at least one address"
        );
        let first = results[0];
        assert_eq!(first.family, AddressFamily::Inet);
        assert_eq!(first.socktype, SocketKind::Stream);
        assert_eq!(first.protocol, Protocol::Tcp);
        assert_eq!(
            first.addr,
            SocketAddr::V4 {
                ip: [127, 0, 0, 1],
                port: 8080
            }
        );

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn resolve_returns_the_parsed_address_for_a_numeric_ipv6_host() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let hints = AddrInfoHints {
            family: Some(AddressFamily::Inet6),
            socktype: Some(SocketKind::Stream),
            protocol: Some(Protocol::Tcp),
        };
        let results =
            resolve("::1", "9090", &hints).expect("resolve should succeed for a numeric IPv6 host");
        assert!(
            !results.is_empty(),
            "resolve should return at least one address"
        );
        let first = results[0];
        assert_eq!(first.family, AddressFamily::Inet6);
        assert_eq!(
            first.addr,
            SocketAddr::V6 {
                ip: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
                port: 9090,
                flow_info: 0,
                scope_id: 0,
            }
        );

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn resolve_defaults_to_any_family_when_hints_leave_it_unspecified() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let hints = AddrInfoHints {
            family: None,
            socktype: Some(SocketKind::Stream),
            protocol: Some(Protocol::Tcp),
        };
        let results = resolve("127.0.0.1", "80", &hints)
            .expect("resolve should succeed leaving family unspecified (AF_UNSPEC)");
        assert!(
            !results.is_empty(),
            "resolve should return at least one address"
        );
        match results[0].addr {
            SocketAddr::V4 { ip, port } => {
                assert_eq!(ip, [127, 0, 0, 1]);
                assert_eq!(port, 80);
            }
            SocketAddr::V6 { .. } => panic!("expected an IPv4 address for a numeric IPv4 host"),
        }

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn resolve_fails_for_an_unrecognized_service_name() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");

        let hints = AddrInfoHints {
            family: Some(AddressFamily::Inet),
            socktype: Some(SocketKind::Stream),
            protocol: Some(Protocol::Tcp),
        };
        // A made-up service name (not a number, and not expected to be in
        // the local services database) should fail to resolve without
        // needing any actual network access.
        resolve("127.0.0.1", "not-a-real-service-name-xyz", &hints)
            .expect_err("resolve should fail for an unrecognized, made-up service name");

        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn htons_then_ntohs_round_trips() {
        // No startup()/cleanup() needed here -- unlike every other
        // function in this module, htons/htonl/ntohs/ntohl are pure
        // byte-swaps with no socket/Winsock-lifecycle dependency.
        assert_eq!(ntohs(htons(0x1234)), 0x1234);
    }

    #[test]
    fn htonl_then_ntohl_round_trips() {
        assert_eq!(ntohl(htonl(0x1234_5678)), 0x1234_5678);
    }

    #[test]
    fn htons_matches_to_sockaddrs_own_port_byte_swap() {
        // to_sockaddr already applies this same conversion internally
        // (verified by to_sockaddr_stores_the_port_in_network_byte_order
        // above) -- htons should agree with it exactly.
        assert_eq!(htons(0x1234), 0x1234u16.to_be());
    }

    #[test]
    fn htonl_is_the_identity_on_a_palindromic_value() {
        // 0xAABBBBAA's bytes (AA BB BB AA) read the same forwards and
        // backwards, so it's unaffected by any byte-swap -- a simple,
        // endian-independent sanity check that doesn't rely on assuming
        // the host's own byte order.
        assert_eq!(htonl(0xAABBBBAA), 0xAABBBBAA);
    }
}

// --- Non-blocking mode -------------------------------------------------

/// `FIONBIO` — the `ioctlsocket` command selecting blocking vs
/// non-blocking mode. Winsock defines it as `0x8004667E`; read as the
/// `i32` the call's parameter actually is, that is a negative value, so
/// it is written here as the exact bit pattern rather than a decimal
/// literal that would need a cast to be believed.
const FIONBIO: i32 = 0x8004_667E_u32 as i32;

/// Switch `sock` between blocking (the default) and non-blocking mode —
/// `ioctlsocket(FIONBIO)`, Winsock's equivalent of
/// `fcntl(fd, F_SETFL, O_NONBLOCK)`.
///
/// In non-blocking mode a call that would otherwise wait — `accept` with
/// no pending connection, `recv`/`recvfrom` with nothing queued,
/// `connect` mid-handshake — returns `WSAEWOULDBLOCK` (10035) instead.
/// That is an ordinary "not ready yet", not a failure of the socket, and
/// distinguishing the two is the caller's job: this crate reports it as
/// the plain [`crate::error::Win32Error`] it is rather than inventing a
/// separate would-block signal.
///
/// There is no read-side counterpart. Winsock provides no way to query a
/// socket's current blocking mode — `FIONBIO` is set-only, and
/// `getsockopt` has no equivalent option. A caller that needs to know
/// must track what it set, which is a real API limitation of the OS and
/// not an omission here.
///
/// # Safety
///
/// `sock` must be a currently-open, valid socket from [`socket`].
pub unsafe fn set_nonblocking(
    sock: RawSocket,
    nonblocking: bool,
) -> Result<(), crate::error::Win32Error> {
    let mut mode: u32 = u32::from(nonblocking);
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `mode` is a valid, initialized `u32` out-parameter that
    // `ioctlsocket` reads (and, for `FIONBIO`, only reads) for the
    // duration of the call.
    let r = unsafe { ioctlsocket(sock, FIONBIO, &mut mode) };
    if r != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(())
    }
}

// --- AF_UNIX -----------------------------------------------------------

/// `sun_path`'s capacity in `sockaddr_un` — 108 bytes, the same figure
/// every BSD-derived `sockaddr_un` uses and the one Windows'
/// `afunix.h` carries. One byte is the NUL terminator, so 107 is the
/// usable path length; [`UnixSocketAddr::new`] enforces that.
pub const UNIX_PATH_CAPACITY: usize = 108;

// sockaddr_un: `size_of` 110 (a 2-byte family followed by 108 path
// bytes), `align_of` 2 — no padding, since every field is byte- or
// u16-aligned. Pinned by the asserts below so a mistranscription of
// `afunix.h` cannot survive a build.
#[repr(C)]
#[derive(Clone, Copy)]
struct SockAddrUn {
    family: u16,
    path: [u8; UNIX_PATH_CAPACITY],
}

const _: () = assert!(core::mem::size_of::<SockAddrUn>() == 110);
const _: () = assert!(core::mem::align_of::<SockAddrUn>() == 2);

/// A Unix-domain socket address: a filesystem path.
///
/// Deliberately **not** a third [`SocketAddr`] variant. `sockaddr_un` is
/// 110 bytes against `sockaddr_in6`'s 28, so folding it in would have
/// quadrupled the size of every IPv4 address this module passes around,
/// to serve a family whose calls share no address-handling code with the
/// IP ones anyway. Two types, each the size of what it describes.
///
/// Windows supports only *pathname* addresses here — no Linux-style
/// abstract namespace (a leading NUL byte), which `afunix.h` does not
/// implement. A path is stored NUL-terminated exactly as it goes on the
/// wire.
#[derive(Clone, Copy)]
pub struct UnixSocketAddr {
    raw: SockAddrUn,
    /// Bytes of `path` in use, excluding the NUL terminator.
    len: usize,
}

impl core::fmt::Debug for UnixSocketAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UnixSocketAddr")
            .field("path", &self.path())
            .finish()
    }
}

impl PartialEq for UnixSocketAddr {
    fn eq(&self, other: &Self) -> bool {
        self.path_bytes() == other.path_bytes()
    }
}

impl Eq for UnixSocketAddr {}

impl UnixSocketAddr {
    /// Build an address from a filesystem path.
    ///
    /// Takes bytes rather than `&str` because a socket path is a
    /// filesystem path, and this crate does not get to decide that a
    /// caller's paths are UTF-8. Rejects an embedded NUL (which would
    /// silently truncate the path the OS sees) and anything longer than
    /// 107 bytes, both with `ERROR_INVALID_PARAMETER` — the same code
    /// Winsock itself reports for a malformed address, rather than a
    /// bespoke error this crate invents.
    pub fn new(path: &[u8]) -> Result<Self, crate::error::Win32Error> {
        if path.is_empty() || path.len() >= UNIX_PATH_CAPACITY || path.contains(&0) {
            return Err(crate::error::Win32Error::ERROR_INVALID_PARAMETER);
        }
        let mut raw = SockAddrUn {
            family: AddressFamily::Unix as u16,
            path: [0u8; UNIX_PATH_CAPACITY],
        };
        raw.path[..path.len()].copy_from_slice(path);
        Ok(UnixSocketAddr {
            raw,
            len: path.len(),
        })
    }

    /// The path bytes, without the NUL terminator.
    pub fn path_bytes(&self) -> &[u8] {
        &self.raw.path[..self.len]
    }

    /// The path as UTF-8, or `None` if it isn't. Convenience over
    /// [`Self::path_bytes`] for the common case; the bytes stay
    /// authoritative.
    pub fn path(&self) -> Option<&str> {
        core::str::from_utf8(self.path_bytes()).ok()
    }

    /// The encoded length to pass as Winsock's `namelen`: the family
    /// field plus the path plus its NUL terminator. Not
    /// `size_of::<SockAddrUn>()` — Winsock accepts (and `getsockname`
    /// reports) the used prefix, and passing the full 110 bytes for a
    /// short path makes the trailing zeros part of the address on some
    /// paths.
    fn encoded_len(&self) -> i32 {
        (core::mem::size_of::<u16>() + self.len + 1) as i32
    }

    fn as_ptr(&self) -> *const u8 {
        (&self.raw as *const SockAddrUn).cast()
    }

    /// Decode a Winsock-filled `sockaddr_un` buffer.
    ///
    /// # Safety
    ///
    /// `buf` must point at `len` valid, initialized bytes that Winsock
    /// filled with a `sockaddr_un`.
    unsafe fn from_raw(buf: *const u8, len: i32) -> Result<Self, crate::error::Win32Error> {
        let len = len.max(0) as usize;
        if len < core::mem::size_of::<u16>() {
            return Err(crate::error::Win32Error::ERROR_INVALID_PARAMETER);
        }
        // SAFETY: the caller guarantees `len` initialized bytes at `buf`,
        // and `len >= 2` was just checked, so reading the family field is
        // in bounds. `read_unaligned` because Winsock makes no alignment
        // promise about the buffer it filled.
        let family = unsafe { core::ptr::read_unaligned(buf.cast::<u16>()) };
        if family != AddressFamily::Unix as u16 {
            return Err(crate::error::Win32Error::ERROR_INVALID_PARAMETER);
        }
        // The path occupies whatever follows the family field, up to the
        // first NUL. An `accept`ed peer on Windows is commonly reported
        // as an *unnamed* address (family only, `len == 2`) — that is
        // normal, not an error, and yields an empty path.
        let path_len = len - core::mem::size_of::<u16>();
        // SAFETY: same guarantee as above; `path_len` bytes follow the
        // 2-byte family field within the caller's `len` bytes.
        let path = unsafe { core::slice::from_raw_parts(buf.add(2), path_len) };
        let path = match path.iter().position(|&b| b == 0) {
            Some(nul) => &path[..nul],
            None => path,
        };
        if path.is_empty() {
            let mut raw = SockAddrUn {
                family: AddressFamily::Unix as u16,
                path: [0u8; UNIX_PATH_CAPACITY],
            };
            raw.path[0] = 0;
            return Ok(UnixSocketAddr { raw, len: 0 });
        }
        UnixSocketAddr::new(path)
    }
}

/// Bind `sock` to a filesystem path — `bind` with a `sockaddr_un`.
///
/// The path must not already exist: Windows, like Unix, creates a
/// filesystem entry for the socket and refuses to bind over one that is
/// already there (`ERROR_ADDRESS_ALREADY_ASSOCIATED`/`WSAEADDRINUSE`).
/// Removing a stale entry left by a crashed process is the caller's
/// policy decision — this crate will not unlink a path behind a caller's
/// back, since "stale" is indistinguishable from "in use by a live
/// server" without probing.
///
/// # Safety
///
/// `sock` must be a currently-open, valid [`AddressFamily::Unix`] socket
/// from [`socket`].
pub unsafe fn bind_unix(
    sock: RawSocket,
    addr: &UnixSocketAddr,
) -> Result<(), crate::error::Win32Error> {
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `addr` owns a valid `sockaddr_un` and `encoded_len`
    // names the prefix of it that is in use.
    let ok = unsafe { raw_bind(sock, addr.as_ptr(), addr.encoded_len()) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(())
    }
}

/// Connect `sock` to a bound Unix-domain path — `connect` with a
/// `sockaddr_un`.
///
/// # Safety
///
/// `sock` must be a currently-open, valid [`AddressFamily::Unix`] socket
/// from [`socket`].
pub unsafe fn connect_unix(
    sock: RawSocket,
    addr: &UnixSocketAddr,
) -> Result<(), crate::error::Win32Error> {
    // SAFETY: as in `bind_unix` above.
    let ok = unsafe { raw_connect(sock, addr.as_ptr(), addr.encoded_len()) };
    if ok != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ))
    } else {
        Ok(())
    }
}

/// Accept one incoming Unix-domain connection — `accept` over a
/// `sockaddr_un` buffer. The [`AddressFamily::Unix`] counterpart of
/// [`accept`], which sizes its buffer for `sockaddr_in6` and would
/// truncate a `sockaddr_un`.
///
/// The returned peer address is usually *unnamed* (an empty path):
/// Windows does not autobind a connecting Unix-domain socket to a path
/// the way it assigns an ephemeral port to a TCP client, so unless the
/// client explicitly bound itself there is no path to report. That is
/// the OS's behavior, surfaced rather than papered over with a
/// synthesized name.
///
/// # Safety
///
/// `sock` must be a currently-open, valid, already-[`listen`]-ing
/// [`AddressFamily::Unix`] socket from [`socket`].
pub unsafe fn accept_unix(
    sock: RawSocket,
) -> Result<(RawSocket, UnixSocketAddr), crate::error::Win32Error> {
    let mut buf = [0u8; core::mem::size_of::<SockAddrUn>()];
    let mut addr_len: i32 = buf.len() as i32;
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `buf` is a valid buffer matched by `addr_len` naming its
    // exact capacity.
    let new_sock = unsafe { raw_accept(sock, buf.as_mut_ptr(), &mut addr_len) };
    if new_sock == INVALID_SOCKET {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        return Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ));
    }
    // SAFETY: a successful `accept` guarantees `buf` holds `addr_len`
    // valid bytes naming the peer's `sockaddr_un`.
    let peer = unsafe { UnixSocketAddr::from_raw(buf.as_ptr(), addr_len) }?;
    Ok((new_sock, peer))
}

/// The local path `sock` is bound to — `getsockname` over a
/// `sockaddr_un` buffer, the [`AddressFamily::Unix`] counterpart of
/// [`local_addr`].
///
/// # Safety
///
/// `sock` must be a currently-open, valid [`AddressFamily::Unix`] socket
/// from [`socket`].
pub unsafe fn local_addr_unix(sock: RawSocket) -> Result<UnixSocketAddr, crate::error::Win32Error> {
    let mut buf = [0u8; core::mem::size_of::<SockAddrUn>()];
    let mut addr_len: i32 = buf.len() as i32;
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `buf`/`addr_len` are a valid buffer and its exact
    // capacity.
    let r = unsafe { getsockname(sock, buf.as_mut_ptr(), &mut addr_len) };
    if r != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        return Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ));
    }
    // SAFETY: a successful `getsockname` filled `buf` with `addr_len`
    // valid bytes.
    unsafe { UnixSocketAddr::from_raw(buf.as_ptr(), addr_len) }
}

/// The peer's path on a connected Unix-domain socket — `getpeername`
/// over a `sockaddr_un` buffer, the [`AddressFamily::Unix`] counterpart
/// of [`peer_addr`].
///
/// On Windows an `accept`ed peer is commonly *unnamed* (family only, no
/// path): unlike a TCP client's ephemeral port, Windows does not autobind
/// a connecting `AF_UNIX` socket to a path. That is the OS's own
/// behavior, surfaced here as an empty [`UnixSocketAddr::path_bytes`]
/// rather than an error.
///
/// # Safety
///
/// `sock` must be a currently-open, valid, connected
/// [`AddressFamily::Unix`] socket (from [`accept_unix`]/after
/// [`connect_unix`]).
pub unsafe fn peer_addr_unix(sock: RawSocket) -> Result<UnixSocketAddr, crate::error::Win32Error> {
    let mut buf = [0u8; core::mem::size_of::<SockAddrUn>()];
    let mut addr_len: i32 = buf.len() as i32;
    // SAFETY: `sock` is caller-supplied per this function's own safety
    // contract; `buf`/`addr_len` are a valid buffer and its exact
    // capacity.
    let r = unsafe { getpeername(sock, buf.as_mut_ptr(), &mut addr_len) };
    if r != 0 {
        // SAFETY: `WSAGetLastError` takes no arguments; calling it
        // immediately after a failing Winsock call is documented to
        // report that same call's error.
        return Err(crate::error::Win32Error::from_raw(
            unsafe { WSAGetLastError() } as u32,
        ));
    }
    // SAFETY: a successful `getpeername` filled `buf` with `addr_len`
    // valid bytes.
    unsafe { UnixSocketAddr::from_raw(buf.as_ptr(), addr_len) }
}

#[cfg(all(test, windows))]
mod unix_tests {
    use super::*;

    #[test]
    fn an_address_round_trips_its_path() {
        let addr = UnixSocketAddr::new(b"C:\\Temp\\rusty_win32.sock").expect("valid path");
        assert_eq!(addr.path_bytes(), b"C:\\Temp\\rusty_win32.sock");
        assert_eq!(addr.path(), Some("C:\\Temp\\rusty_win32.sock"));
    }

    #[test]
    fn an_embedded_nul_is_rejected() {
        assert_eq!(
            UnixSocketAddr::new(b"a\0b").unwrap_err(),
            crate::error::Win32Error::ERROR_INVALID_PARAMETER
        );
    }

    #[test]
    fn an_oversized_path_is_rejected() {
        let too_long = [b'a'; UNIX_PATH_CAPACITY];
        assert_eq!(
            UnixSocketAddr::new(&too_long).unwrap_err(),
            crate::error::Win32Error::ERROR_INVALID_PARAMETER
        );
        // One byte under the capacity leaves room for the NUL and is fine.
        let longest = [b'a'; UNIX_PATH_CAPACITY - 1];
        assert!(UnixSocketAddr::new(&longest).is_ok());
    }

    #[test]
    fn an_empty_path_is_rejected() {
        assert!(UnixSocketAddr::new(b"").is_err());
    }

    #[test]
    fn encoded_len_covers_family_path_and_terminator() {
        let addr = UnixSocketAddr::new(b"abc").expect("valid path");
        assert_eq!(addr.encoded_len(), 2 + 3 + 1);
    }

    // Every function below this point drives a real listener/client pair
    // over `AF_UNIX`, so this is the only place `peer_addr_unix` (and,
    // incidentally, `bind_unix`/`connect_unix`/`accept_unix`/
    // `local_addr_unix`) actually gets exercised — the address-round-trip
    // tests above only ever encode and decode bytes.

    fn temp_socket_path(name: &str) -> alloc::string::String {
        let path = std::env::temp_dir().join(name);
        // Windows doesn't unlink an `AF_UNIX` bind's backing file on
        // close (unlike a Linux `unlink(2)`-then-exit shell), and this
        // module's own `bind_unix` doesn't paper over that — `sys::net`'s
        // stale-socket retry (see this crate's rustils consumer) is
        // deliberately a caller-side concern, not this crate's. This
        // CI's own two `cargo test` invocations against the same runner
        // (no-default-features, then `--features std`) are exactly that
        // caller: without this, the second run's bind_unix fails
        // `WSAEADDRINUSE` on the first run's leftover file.
        let _ = std::fs::remove_file(&path);
        alloc::string::String::from(path.to_str().expect("temp path should be valid UTF-8"))
    }

    #[test]
    fn peer_addr_unix_reports_the_client_as_unnamed() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");
        let path = temp_socket_path("rusty_win32_peer_addr_unix_unnamed.sock");
        let addr = UnixSocketAddr::new(path.as_bytes()).expect("valid path");

        let listener = socket(
            AddressFamily::Unix,
            SocketKind::Stream,
            Protocol::Unspecified,
        )
        .expect("socket should succeed creating an AF_UNIX socket");
        // SAFETY: `listener` was just created and is closed at scope end
        // via `close_socket` below.
        unsafe { bind_unix(listener, &addr) }.expect("bind_unix should succeed on a fresh path");
        // SAFETY: `listener` is the socket just bound above.
        unsafe { listen(listener, 1) }.expect("listen should succeed on a bound socket");

        let client = socket(
            AddressFamily::Unix,
            SocketKind::Stream,
            Protocol::Unspecified,
        )
        .expect("socket should succeed creating a second AF_UNIX socket");
        // SAFETY: `client` was just created; `listener` is bound and
        // listening.
        unsafe { connect_unix(client, &addr) }.expect("connect_unix should succeed");

        // SAFETY: `listener` is listening and has a pending connection
        // from `client` above.
        let (server, _peer_of_accept) =
            unsafe { accept_unix(listener) }.expect("accept_unix should succeed");

        // The point of this test: Windows does not autobind a connecting
        // AF_UNIX client to a path (this module's own doc comment on
        // `peer_addr_unix` records why), so the server's view of its
        // peer is unnamed — an empty path, not an error.
        // SAFETY: `server` is the connected socket `accept_unix` returned.
        let peer = unsafe { peer_addr_unix(server) }.expect("peer_addr_unix should succeed");
        assert_eq!(
            peer.path_bytes(),
            b"",
            "an AF_UNIX client Windows never autobinds should report as unnamed"
        );

        // SAFETY: each handle above is open exactly once and closed
        // exactly once here.
        unsafe {
            close_socket(server).expect("closesocket should succeed");
            close_socket(client).expect("closesocket should succeed");
            close_socket(listener).expect("closesocket should succeed");
        }
        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }

    #[test]
    fn peer_addr_unix_matches_local_addr_unix_from_the_other_side() {
        startup().expect("WSAStartup should succeed requesting Winsock 2.2");
        let path = temp_socket_path("rusty_win32_peer_addr_unix_matches.sock");
        let addr = UnixSocketAddr::new(path.as_bytes()).expect("valid path");

        let listener = socket(
            AddressFamily::Unix,
            SocketKind::Stream,
            Protocol::Unspecified,
        )
        .expect("socket should succeed");
        // SAFETY: `listener` was just created.
        unsafe { bind_unix(listener, &addr) }.expect("bind_unix should succeed");
        // SAFETY: `listener` is bound above.
        unsafe { listen(listener, 1) }.expect("listen should succeed");

        let client = socket(
            AddressFamily::Unix,
            SocketKind::Stream,
            Protocol::Unspecified,
        )
        .expect("socket should succeed");
        // SAFETY: `client` was just created; `listener` is listening.
        unsafe { connect_unix(client, &addr) }.expect("connect_unix should succeed");
        // SAFETY: `listener` has a pending connection from `client`.
        let (server, _) = unsafe { accept_unix(listener) }.expect("accept_unix should succeed");

        // The server's local address (what it's bound to) is exactly
        // what the client sees as its own peer — the two sides of the
        // same named endpoint.
        // SAFETY: `server` is a connected, valid socket.
        let server_local = unsafe { local_addr_unix(server) }.expect("local_addr_unix");
        // SAFETY: `client` is a connected, valid socket.
        let client_peer = unsafe { peer_addr_unix(client) }.expect("peer_addr_unix");
        assert_eq!(server_local.path_bytes(), client_peer.path_bytes());
        assert_eq!(server_local.path_bytes(), path.as_bytes());

        // SAFETY: each handle above is open exactly once and closed
        // exactly once here.
        unsafe {
            close_socket(server).expect("closesocket should succeed");
            close_socket(client).expect("closesocket should succeed");
            close_socket(listener).expect("closesocket should succeed");
        }
        cleanup().expect("WSACleanup should succeed matching the startup call above");
    }
}
