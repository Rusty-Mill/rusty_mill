//! TCP/UDP sockets over IPv4 (`AF_INET`) and IPv6 (`AF_INET6`), and
//! stream sockets bound to a path (`AF_UNIX`).
//!
//! Kernel `struct sockaddr_in`/`sockaddr_in6` layouts (checked by
//! `const` size/offset assertions, not copied from glibc), matching this
//! crate's usual convention. `port`/`flowinfo` are stored in network byte
//! order exactly as the kernel expects them -- [`SockAddrIn::new`]/
//! [`SockAddrIn6::new`] take a host-order `u16` port and do the conversion
//! for you. Address octets need no such conversion: they're already in
//! transmission order (`[127, 0, 0, 1]` for `127.0.0.1`), so they're stored
//! and read back as a plain byte array, not swapped through a numeric type.
//!
//! Blocking by default, like every other fd primitive in this crate: pass
//! [`SOCK_NONBLOCK`] to [`socket`] for non-blocking I/O instead of assuming
//! an async runtime. See [ADR-0003](../../docs/adr/0003-add-sockets-tcp-udp-and-dns-resolution.md)
//! for why this module exists and what it deliberately leaves out (no
//! `setsockopt`/`getsockopt` -- nothing beyond what `/dev/tcp`/`/dev/udp`-style
//! redirection and the `dns` resolver module that sits on top of this one
//! need). Unix-domain sockets cover one more need: a non-blocking `connect`
//! to a path, which `std` cannot do, so a client can give up on a server
//! whose accept backlog is full; only pathname addresses are supported.
//! And one more: passing descriptors over a Unix socket (`SCM_RIGHTS`,
//! [`send_with_fds`]/[`recv_with_fds`]), which `std` cannot do either,
//! for rusty_mux's agent launch (C75).

use crate::arch::nr;
use crate::arch::{from_ret, from_ret_i32, syscall2, syscall3, syscall4, syscall6, Errno};

/// Address family: Unix-domain (local) sockets.
pub const AF_UNIX: i32 = 1;
/// Address family: IPv4.
pub const AF_INET: i32 = 2;
/// Address family: IPv6.
pub const AF_INET6: i32 = 10;

/// Socket type: reliable, connection-oriented byte stream (TCP).
pub const SOCK_STREAM: i32 = 1;
/// Socket type: connectionless, unreliable datagrams (UDP).
pub const SOCK_DGRAM: i32 = 2;
/// OR into `type` to atomically set close-on-exec on the new socket fd (a
/// Linux extension, same idea as [`crate::fd::O_CLOEXEC`]).
pub const SOCK_CLOEXEC: i32 = 0o2000000;
/// OR into `type` (or an `accept4` `flags` argument) to atomically set
/// non-blocking I/O on the new socket fd.
pub const SOCK_NONBLOCK: i32 = 0o0004000;

/// `shutdown(2)` mode: no further receives.
pub const SHUT_RD: i32 = 0;
/// `shutdown(2)` mode: no further sends.
pub const SHUT_WR: i32 = 1;
/// `shutdown(2)` mode: no further sends or receives.
pub const SHUT_RDWR: i32 = 2;

/// The longest [`SockAddrUn`] path, leaving room for its terminating NUL.
pub const UNIX_PATH_MAX: usize = 107;

/// A Unix-domain socket address naming a path (kernel `struct sockaddr_un`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SockAddrUn {
    family: u16,
    path: [u8; UNIX_PATH_MAX + 1], // NUL-terminated
}

const _: () = assert!(core::mem::size_of::<SockAddrUn>() == 110);
const _: () = assert!(core::mem::offset_of!(SockAddrUn, path) == 2);

impl SockAddrUn {
    /// The address of the socket at `path`, or `None` when `path` is empty,
    /// longer than [`UNIX_PATH_MAX`] or holds a NUL (abstract addresses are
    /// not supported).
    pub fn new(path: &[u8]) -> Option<Self> {
        if path.is_empty() || path.len() > UNIX_PATH_MAX || path.contains(&0) {
            return None;
        }
        let mut addr = SockAddrUn {
            family: AF_UNIX as u16,
            path: [0; UNIX_PATH_MAX + 1],
        };
        addr.path[..path.len()].copy_from_slice(path);
        Some(addr)
    }

    /// The path, without its terminating NUL.
    pub fn path(&self) -> &[u8] {
        let len = self
            .path
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(UNIX_PATH_MAX);
        &self.path[..len]
    }
}

/// An IPv4 socket address (kernel `struct sockaddr_in`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SockAddrIn {
    family: u16,
    port: u16,     // network byte order
    addr: [u8; 4], // already in transmission (dotted-quad) order, no swap needed
    zero: [u8; 8],
}

const _: () = assert!(core::mem::size_of::<SockAddrIn>() == 16);
const _: () = assert!(core::mem::offset_of!(SockAddrIn, port) == 2);
const _: () = assert!(core::mem::offset_of!(SockAddrIn, addr) == 4);

impl SockAddrIn {
    /// Build an IPv4 address from dotted-quad octets and a host-order port.
    pub fn new(octets: [u8; 4], port: u16) -> Self {
        SockAddrIn {
            family: AF_INET as u16,
            port: port.to_be(),
            addr: octets,
            zero: [0; 8],
        }
    }

    /// The address's dotted-quad octets.
    pub fn octets(&self) -> [u8; 4] {
        self.addr
    }

    /// The port, in host byte order.
    pub fn port(&self) -> u16 {
        u16::from_be(self.port)
    }
}

/// An IPv6 socket address (kernel `struct sockaddr_in6`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SockAddrIn6 {
    family: u16,
    port: u16,     // network byte order
    flowinfo: u32, // network byte order
    addr: [u8; 16],
    scope_id: u32,
}

const _: () = assert!(core::mem::size_of::<SockAddrIn6>() == 28);
const _: () = assert!(core::mem::offset_of!(SockAddrIn6, port) == 2);
const _: () = assert!(core::mem::offset_of!(SockAddrIn6, flowinfo) == 4);
const _: () = assert!(core::mem::offset_of!(SockAddrIn6, addr) == 8);
const _: () = assert!(core::mem::offset_of!(SockAddrIn6, scope_id) == 24);

impl SockAddrIn6 {
    /// Build an IPv6 address from 16 address octets and a host-order port.
    pub fn new(octets: [u8; 16], port: u16) -> Self {
        SockAddrIn6 {
            family: AF_INET6 as u16,
            port: port.to_be(),
            flowinfo: 0,
            addr: octets,
            scope_id: 0,
        }
    }

    /// The address's 16 octets.
    pub fn octets(&self) -> [u8; 16] {
        self.addr
    }

    /// The port, in host byte order.
    pub fn port(&self) -> u16 {
        u16::from_be(self.port)
    }
}

/// Create an endpoint for communication, returning the new socket fd.
/// `domain` is [`AF_INET`]/[`AF_INET6`]; `ty` is [`SOCK_STREAM`]/
/// [`SOCK_DGRAM`], optionally OR'd with [`SOCK_NONBLOCK`]/[`SOCK_CLOEXEC`];
/// `protocol` is `0` to let the kernel pick the default for `(domain, ty)`.
pub fn socket(domain: i32, ty: i32, protocol: i32) -> Result<i32, Errno> {
    // SAFETY: plain integer arguments.
    let ret = unsafe { syscall3(nr::SOCKET, domain as usize, ty as usize, protocol as usize) };
    from_ret_i32(ret)
}

/// Bind `fd` to a local IPv4 address.
pub fn bind_in(fd: i32, addr: &SockAddrIn) -> Result<(), Errno> {
    // SAFETY: `addr` is a valid, correctly-sized `sockaddr_in` the kernel
    // only reads.
    let ret = unsafe {
        syscall3(
            nr::BIND,
            fd as usize,
            addr as *const SockAddrIn as usize,
            core::mem::size_of::<SockAddrIn>(),
        )
    };
    from_ret(ret).map(|_| ())
}

/// Bind `fd` to a local IPv6 address.
pub fn bind_in6(fd: i32, addr: &SockAddrIn6) -> Result<(), Errno> {
    // SAFETY: `addr` is a valid, correctly-sized `sockaddr_in6` the kernel
    // only reads.
    let ret = unsafe {
        syscall3(
            nr::BIND,
            fd as usize,
            addr as *const SockAddrIn6 as usize,
            core::mem::size_of::<SockAddrIn6>(),
        )
    };
    from_ret(ret).map(|_| ())
}

/// Connect `fd` to a remote IPv4 address. Blocks until the connection is
/// established (or fails) unless `fd` was opened with [`SOCK_NONBLOCK`].
pub fn connect_in(fd: i32, addr: &SockAddrIn) -> Result<(), Errno> {
    // SAFETY: `addr` is a valid, correctly-sized `sockaddr_in` the kernel
    // only reads.
    let ret = unsafe {
        syscall3(
            nr::CONNECT,
            fd as usize,
            addr as *const SockAddrIn as usize,
            core::mem::size_of::<SockAddrIn>(),
        )
    };
    from_ret(ret).map(|_| ())
}

/// Connect `fd` to a remote IPv6 address. Blocks until the connection is
/// established (or fails) unless `fd` was opened with [`SOCK_NONBLOCK`].
pub fn connect_in6(fd: i32, addr: &SockAddrIn6) -> Result<(), Errno> {
    // SAFETY: `addr` is a valid, correctly-sized `sockaddr_in6` the kernel
    // only reads.
    let ret = unsafe {
        syscall3(
            nr::CONNECT,
            fd as usize,
            addr as *const SockAddrIn6 as usize,
            core::mem::size_of::<SockAddrIn6>(),
        )
    };
    from_ret(ret).map(|_| ())
}

/// Bind `fd` to a path, creating the socket file there.
pub fn bind_un(fd: i32, addr: &SockAddrUn) -> Result<(), Errno> {
    // SAFETY: `addr` is a valid, correctly-sized `sockaddr_un` the kernel
    // only reads.
    let ret = unsafe {
        syscall3(
            nr::BIND,
            fd as usize,
            addr as *const SockAddrUn as usize,
            core::mem::size_of::<SockAddrUn>(),
        )
    };
    from_ret(ret).map(|_| ())
}

/// Connect `fd` to the socket at a path. Blocks while the listener's accept
/// backlog is full unless `fd` was opened with [`SOCK_NONBLOCK`], when it
/// fails with [`Errno::EAGAIN`] instead and may be retried; it never
/// returns `EINPROGRESS`.
pub fn connect_un(fd: i32, addr: &SockAddrUn) -> Result<(), Errno> {
    // SAFETY: `addr` is a valid, correctly-sized `sockaddr_un` the kernel
    // only reads.
    let ret = unsafe {
        syscall3(
            nr::CONNECT,
            fd as usize,
            addr as *const SockAddrUn as usize,
            core::mem::size_of::<SockAddrUn>(),
        )
    };
    from_ret(ret).map(|_| ())
}

/// Mark `fd` (a bound `SOCK_STREAM` socket) as accepting incoming
/// connections, with a backlog of up to `backlog` pending connections.
pub fn listen(fd: i32, backlog: i32) -> Result<(), Errno> {
    // SAFETY: plain integer arguments.
    let ret = unsafe { syscall2(nr::LISTEN, fd as usize, backlog as usize) };
    from_ret(ret).map(|_| ())
}

/// Accept a pending connection on the listening socket `fd`, returning the
/// new connected socket's fd. Discards the peer's address; use
/// [`accept4_in`]/[`accept4_in6`] to also learn who connected or to pass
/// `flags`.
pub fn accept(fd: i32) -> Result<i32, Errno> {
    // accept(fd, NULL, NULL).
    // SAFETY: null addr/addrlen is the documented way to discard the peer
    // address.
    let ret = unsafe { syscall3(nr::ACCEPT, fd as usize, 0, 0) };
    from_ret_i32(ret)
}

/// Accept a pending connection on an IPv4 listening socket `fd`, returning
/// the new connected socket's fd and the peer's address. `flags` may
/// include [`SOCK_NONBLOCK`]/[`SOCK_CLOEXEC`] for the new fd.
pub fn accept4_in(fd: i32, flags: i32) -> Result<(i32, SockAddrIn), Errno> {
    let mut addr = SockAddrIn {
        family: 0,
        port: 0,
        addr: [0; 4],
        zero: [0; 8],
    };
    let mut addrlen: u32 = core::mem::size_of::<SockAddrIn>() as u32;
    // SAFETY: `addr`/`addrlen` are valid, correctly-sized out-parameters the
    // kernel fills in.
    let ret = unsafe {
        syscall4(
            nr::ACCEPT4,
            fd as usize,
            &mut addr as *mut SockAddrIn as usize,
            &mut addrlen as *mut u32 as usize,
            flags as usize,
        )
    };
    let newfd = from_ret_i32(ret)?;
    Ok((newfd, addr))
}

/// Accept a pending connection on an IPv6 listening socket `fd`, returning
/// the new connected socket's fd and the peer's address. `flags` may
/// include [`SOCK_NONBLOCK`]/[`SOCK_CLOEXEC`] for the new fd.
pub fn accept4_in6(fd: i32, flags: i32) -> Result<(i32, SockAddrIn6), Errno> {
    let mut addr = SockAddrIn6 {
        family: 0,
        port: 0,
        flowinfo: 0,
        addr: [0; 16],
        scope_id: 0,
    };
    let mut addrlen: u32 = core::mem::size_of::<SockAddrIn6>() as u32;
    // SAFETY: `addr`/`addrlen` are valid, correctly-sized out-parameters the
    // kernel fills in.
    let ret = unsafe {
        syscall4(
            nr::ACCEPT4,
            fd as usize,
            &mut addr as *mut SockAddrIn6 as usize,
            &mut addrlen as *mut u32 as usize,
            flags as usize,
        )
    };
    let newfd = from_ret_i32(ret)?;
    Ok((newfd, addr))
}

/// Get the local IPv4 address `fd` is bound to (the address `bind_in`
/// picked, or `connect_in` bound implicitly).
pub fn getsockname_in(fd: i32) -> Result<SockAddrIn, Errno> {
    let mut addr = SockAddrIn {
        family: 0,
        port: 0,
        addr: [0; 4],
        zero: [0; 8],
    };
    let mut addrlen: u32 = core::mem::size_of::<SockAddrIn>() as u32;
    // SAFETY: `addr`/`addrlen` are valid, correctly-sized out-parameters the
    // kernel fills in.
    let ret = unsafe {
        syscall3(
            nr::GETSOCKNAME,
            fd as usize,
            &mut addr as *mut SockAddrIn as usize,
            &mut addrlen as *mut u32 as usize,
        )
    };
    from_ret(ret)?;
    Ok(addr)
}

/// Get the local IPv6 address `fd` is bound to. See [`getsockname_in`].
pub fn getsockname_in6(fd: i32) -> Result<SockAddrIn6, Errno> {
    let mut addr = SockAddrIn6 {
        family: 0,
        port: 0,
        flowinfo: 0,
        addr: [0; 16],
        scope_id: 0,
    };
    let mut addrlen: u32 = core::mem::size_of::<SockAddrIn6>() as u32;
    // SAFETY: `addr`/`addrlen` are valid, correctly-sized out-parameters the
    // kernel fills in.
    let ret = unsafe {
        syscall3(
            nr::GETSOCKNAME,
            fd as usize,
            &mut addr as *mut SockAddrIn6 as usize,
            &mut addrlen as *mut u32 as usize,
        )
    };
    from_ret(ret)?;
    Ok(addr)
}

/// Get the remote IPv4 address `fd` is connected to (via [`connect_in`] or
/// as returned by [`accept4_in`]).
pub fn getpeername_in(fd: i32) -> Result<SockAddrIn, Errno> {
    let mut addr = SockAddrIn {
        family: 0,
        port: 0,
        addr: [0; 4],
        zero: [0; 8],
    };
    let mut addrlen: u32 = core::mem::size_of::<SockAddrIn>() as u32;
    // SAFETY: `addr`/`addrlen` are valid, correctly-sized out-parameters the
    // kernel fills in.
    let ret = unsafe {
        syscall3(
            nr::GETPEERNAME,
            fd as usize,
            &mut addr as *mut SockAddrIn as usize,
            &mut addrlen as *mut u32 as usize,
        )
    };
    from_ret(ret)?;
    Ok(addr)
}

/// Get the remote IPv6 address `fd` is connected to. See [`getpeername_in`].
pub fn getpeername_in6(fd: i32) -> Result<SockAddrIn6, Errno> {
    let mut addr = SockAddrIn6 {
        family: 0,
        port: 0,
        flowinfo: 0,
        addr: [0; 16],
        scope_id: 0,
    };
    let mut addrlen: u32 = core::mem::size_of::<SockAddrIn6>() as u32;
    // SAFETY: `addr`/`addrlen` are valid, correctly-sized out-parameters the
    // kernel fills in.
    let ret = unsafe {
        syscall3(
            nr::GETPEERNAME,
            fd as usize,
            &mut addr as *mut SockAddrIn6 as usize,
            &mut addrlen as *mut u32 as usize,
        )
    };
    from_ret(ret)?;
    Ok(addr)
}

/// Send `buf` on a connected socket `fd` (TCP, or a UDP socket that has
/// called `connect`), returning the number of bytes actually sent.
pub fn send(fd: i32, buf: &[u8], flags: i32) -> Result<usize, Errno> {
    // sendto(fd, buf, len, flags, NULL, 0).
    // SAFETY: `buf` is a valid slice of `buf.len()` bytes the kernel only
    // reads.
    let ret = unsafe {
        syscall6(
            nr::SENDTO,
            fd as usize,
            buf.as_ptr() as usize,
            buf.len(),
            flags as usize,
            0,
            0,
        )
    };
    from_ret(ret)
}

/// Receive into `buf` on a connected socket `fd`, returning the number of
/// bytes actually received (`0` means the peer closed the connection).
pub fn recv(fd: i32, buf: &mut [u8], flags: i32) -> Result<usize, Errno> {
    // recvfrom(fd, buf, len, flags, NULL, NULL).
    // SAFETY: `buf` is a valid, exclusively-borrowed slice of `buf.len()`
    // bytes the kernel writes at most that many of.
    let ret = unsafe {
        syscall6(
            nr::RECVFROM,
            fd as usize,
            buf.as_mut_ptr() as usize,
            buf.len(),
            flags as usize,
            0,
            0,
        )
    };
    from_ret(ret)
}

/// Send `buf` as a single UDP datagram to `dest` (an unconnected `fd` is
/// fine -- this is the usual way to use `SOCK_DGRAM`), returning the number
/// of bytes sent.
pub fn sendto_in(fd: i32, buf: &[u8], flags: i32, dest: &SockAddrIn) -> Result<usize, Errno> {
    // SAFETY: `buf` and `dest` are valid, correctly-sized inputs the kernel
    // only reads.
    let ret = unsafe {
        syscall6(
            nr::SENDTO,
            fd as usize,
            buf.as_ptr() as usize,
            buf.len(),
            flags as usize,
            dest as *const SockAddrIn as usize,
            core::mem::size_of::<SockAddrIn>(),
        )
    };
    from_ret(ret)
}

/// Send `buf` as a single UDP datagram to `dest` (IPv6).
pub fn sendto_in6(fd: i32, buf: &[u8], flags: i32, dest: &SockAddrIn6) -> Result<usize, Errno> {
    // SAFETY: `buf` and `dest` are valid, correctly-sized inputs the kernel
    // only reads.
    let ret = unsafe {
        syscall6(
            nr::SENDTO,
            fd as usize,
            buf.as_ptr() as usize,
            buf.len(),
            flags as usize,
            dest as *const SockAddrIn6 as usize,
            core::mem::size_of::<SockAddrIn6>(),
        )
    };
    from_ret(ret)
}

/// Receive a single UDP datagram into `buf`, returning the number of bytes
/// received and the sender's address.
pub fn recvfrom_in(fd: i32, buf: &mut [u8], flags: i32) -> Result<(usize, SockAddrIn), Errno> {
    let mut addr = SockAddrIn {
        family: 0,
        port: 0,
        addr: [0; 4],
        zero: [0; 8],
    };
    let mut addrlen: u32 = core::mem::size_of::<SockAddrIn>() as u32;
    // SAFETY: `buf` is a valid, exclusively-borrowed slice the kernel writes
    // at most `buf.len()` bytes of; `addr`/`addrlen` are valid,
    // correctly-sized out-parameters.
    let ret = unsafe {
        syscall6(
            nr::RECVFROM,
            fd as usize,
            buf.as_mut_ptr() as usize,
            buf.len(),
            flags as usize,
            &mut addr as *mut SockAddrIn as usize,
            &mut addrlen as *mut u32 as usize,
        )
    };
    let n = from_ret(ret)?;
    Ok((n, addr))
}

/// Receive a single UDP datagram into `buf` (IPv6), returning the number of
/// bytes received and the sender's address.
pub fn recvfrom_in6(fd: i32, buf: &mut [u8], flags: i32) -> Result<(usize, SockAddrIn6), Errno> {
    let mut addr = SockAddrIn6 {
        family: 0,
        port: 0,
        flowinfo: 0,
        addr: [0; 16],
        scope_id: 0,
    };
    let mut addrlen: u32 = core::mem::size_of::<SockAddrIn6>() as u32;
    // SAFETY: `buf` is a valid, exclusively-borrowed slice the kernel writes
    // at most `buf.len()` bytes of; `addr`/`addrlen` are valid,
    // correctly-sized out-parameters.
    let ret = unsafe {
        syscall6(
            nr::RECVFROM,
            fd as usize,
            buf.as_mut_ptr() as usize,
            buf.len(),
            flags as usize,
            &mut addr as *mut SockAddrIn6 as usize,
            &mut addrlen as *mut u32 as usize,
        )
    };
    let n = from_ret(ret)?;
    Ok((n, addr))
}

/// Shut down part or all of a full-duplex connection on `fd` without
/// closing the fd itself (`how` is [`SHUT_RD`]/[`SHUT_WR`]/[`SHUT_RDWR`]).
pub fn shutdown(fd: i32, how: i32) -> Result<(), Errno> {
    // SAFETY: plain integer arguments.
    let ret = unsafe { syscall2(nr::SHUTDOWN, fd as usize, how as usize) };
    from_ret(ret).map(|_| ())
}

/// The most descriptors [`send_with_fds`] sends, or [`recv_with_fds`]
/// receives, in one call.
pub const MAX_FDS: usize = 8;

/// `recvmsg` flag: the control message was cut short, and the descriptors
/// that did not fit were closed by the kernel.
pub const MSG_CTRUNC: i32 = 0x8;
/// `recvmsg` flag: received descriptors are close-on-exec from the start.
pub const MSG_CMSG_CLOEXEC: i32 = 0x4000_0000;

const SOL_SOCKET: i32 = 1;
const SCM_RIGHTS: i32 = 1;

/// Kernel `struct msghdr` (64-bit).
#[repr(C)]
struct MsgHdr {
    name: usize,
    namelen: u32,
    iov: usize,
    iovlen: usize,
    control: usize,
    controllen: usize,
    flags: i32,
}

const _: () = assert!(core::mem::size_of::<MsgHdr>() == 56);
const _: () = assert!(core::mem::offset_of!(MsgHdr, iov) == 16);
const _: () = assert!(core::mem::offset_of!(MsgHdr, control) == 32);
const _: () = assert!(core::mem::offset_of!(MsgHdr, flags) == 48);

/// Kernel `struct iovec`.
#[repr(C)]
struct IoVec {
    base: usize,
    len: usize,
}

/// Kernel `struct cmsghdr` (64-bit): the header before `SCM_RIGHTS` data.
#[repr(C)]
struct CmsgHdr {
    len: usize,
    level: i32,
    ty: i32,
}

const CMSG_HDR: usize = core::mem::size_of::<CmsgHdr>();
const _: () = assert!(CMSG_HDR == 16);
/// `CMSG_SPACE(MAX_FDS * 4)`: the header, then the descriptors padded to 8.
const CONTROL: usize = CMSG_HDR + (MAX_FDS * 4).next_multiple_of(8);

/// A control buffer aligned as `struct cmsghdr` needs.
#[repr(C, align(8))]
struct Control([u8; CONTROL]);

/// What [`recv_with_fds`] received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Received {
    /// Bytes read into the buffer; `0` means the peer closed.
    pub bytes: usize,
    /// The received descriptors, in the order sent; only the first
    /// `count` are valid. The caller owns them and must close them.
    pub fds: [i32; MAX_FDS],
    /// How many of `fds` are valid.
    pub count: usize,
    /// The sender passed more than [`MAX_FDS`] (or the control message was
    /// cut short otherwise); the kernel closed those that did not fit.
    pub truncated: bool,
}

impl Received {
    /// The valid descriptors.
    pub fn fds(&self) -> &[i32] {
        &self.fds[..self.count]
    }
}

/// Send `buf` on a connected Unix stream or datagram socket, passing
/// `fds` along as one `SCM_RIGHTS` control message (the receiver gets
/// its own copies; the caller keeps its descriptors open). Returns the
/// bytes sent, as [`send`] does.
///
/// `EINVAL` when `fds` holds more than [`MAX_FDS`], or when `buf` is
/// empty while `fds` is not: a stream needs at least one byte to carry
/// descriptors.
pub fn send_with_fds(fd: i32, buf: &[u8], fds: &[i32], flags: i32) -> Result<usize, Errno> {
    if fds.len() > MAX_FDS || (buf.is_empty() && !fds.is_empty()) {
        return Err(Errno::EINVAL);
    }
    let mut control = Control([0; CONTROL]);
    let used = match fds.len() {
        0 => 0,
        n => {
            let len = CMSG_HDR + n * 4;
            let header = CmsgHdr {
                len,
                level: SOL_SOCKET,
                ty: SCM_RIGHTS,
            };
            // SAFETY: `control` is 8-aligned and at least `CMSG_HDR` bytes,
            // so a `CmsgHdr` fits at its start.
            unsafe { core::ptr::write(control.0.as_mut_ptr().cast::<CmsgHdr>(), header) };
            for (i, fd) in fds.iter().enumerate() {
                let at = CMSG_HDR + i * 4;
                control.0[at..at + 4].copy_from_slice(&fd.to_ne_bytes());
            }
            len.next_multiple_of(8)
        }
    };
    let iov = IoVec {
        base: buf.as_ptr() as usize,
        len: buf.len(),
    };
    let msg = MsgHdr {
        name: 0,
        namelen: 0,
        iov: &iov as *const IoVec as usize,
        iovlen: 1,
        control: if used == 0 {
            0
        } else {
            control.0.as_ptr() as usize
        },
        controllen: used,
        flags: 0,
    };
    // SAFETY: `msg`, `iov`, `buf` and `control` outlive the call, and the
    // kernel only reads them.
    let ret = unsafe {
        syscall3(
            nr::SENDMSG,
            fd as usize,
            &msg as *const MsgHdr as usize,
            flags as usize,
        )
    };
    from_ret(ret)
}

/// Receive into `buf` from a connected Unix socket, with any descriptors
/// sent alongside. [`MSG_CMSG_CLOEXEC`] is always added to `flags`, so a
/// received descriptor never leaks into a program this process starts.
///
/// The kernel never merges data sent with descriptors into an earlier
/// read, so the descriptors belong to the first byte of `buf`.
pub fn recv_with_fds(fd: i32, buf: &mut [u8], flags: i32) -> Result<Received, Errno> {
    let mut control = Control([0; CONTROL]);
    let iov = IoVec {
        base: buf.as_mut_ptr() as usize,
        len: buf.len(),
    };
    let mut msg = MsgHdr {
        name: 0,
        namelen: 0,
        iov: &iov as *const IoVec as usize,
        iovlen: 1,
        control: control.0.as_mut_ptr() as usize,
        controllen: CONTROL,
        flags: 0,
    };
    // SAFETY: `msg` points at `iov`, which points at `buf`, and at
    // `control`; all are exclusively borrowed for the call and the kernel
    // writes at most their stated lengths.
    let ret = unsafe {
        syscall3(
            nr::RECVMSG,
            fd as usize,
            &mut msg as *mut MsgHdr as usize,
            (flags | MSG_CMSG_CLOEXEC) as usize,
        )
    };
    let bytes = from_ret(ret)?;
    let mut got = Received {
        bytes,
        fds: [-1; MAX_FDS],
        count: 0,
        truncated: msg.flags & MSG_CTRUNC != 0,
    };
    let end = msg.controllen.min(CONTROL);
    let mut at = 0;
    while at + CMSG_HDR <= end {
        // SAFETY: `at` is a multiple of 8 within `control`, which the kernel
        // filled with well-formed headers up to `controllen`.
        let header = unsafe { core::ptr::read(control.0.as_ptr().add(at).cast::<CmsgHdr>()) };
        if header.len < CMSG_HDR || at + header.len > end {
            break;
        }
        if header.level == SOL_SOCKET && header.ty == SCM_RIGHTS {
            let data = &control.0[at + CMSG_HDR..at + header.len];
            for raw in data.as_chunks::<4>().0 {
                let fd = i32::from_ne_bytes(*raw);
                match got.count < MAX_FDS {
                    true => {
                        got.fds[got.count] = fd;
                        got.count += 1;
                    }
                    // Can't happen with a buffer of MAX_FDS, but never leak one.
                    false => {
                        let _ = crate::fd::close(fd);
                        got.truncated = true;
                    }
                }
            }
        }
        at += header.len.next_multiple_of(8);
    }
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fd;

    const LOOPBACK: [u8; 4] = [127, 0, 0, 1];
    const LOOPBACK6: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

    #[test]
    fn sockaddr_un_takes_paths_that_fit() {
        let a = SockAddrUn::new(b"/run/x.sock").expect("fits");
        assert_eq!(a.path(), b"/run/x.sock");
        let longest = [b'a'; UNIX_PATH_MAX];
        assert_eq!(
            SockAddrUn::new(&longest).expect("fits").path(),
            &longest[..]
        );
        assert_eq!(SockAddrUn::new(&[b'a'; UNIX_PATH_MAX + 1]), None);
        assert_eq!(SockAddrUn::new(b""), None);
        assert_eq!(SockAddrUn::new(b"\0abstract"), None);
    }

    #[test]
    fn unix_connect_fails_fast_on_a_full_backlog_when_non_blocking() {
        let path = std::env::temp_dir().join(format!("rusty-libc-un-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let addr = SockAddrUn::new(path.as_os_str().as_encoded_bytes()).expect("fits");
        let listener = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0).expect("socket");
        bind_un(listener, &addr).expect("bind");
        // A backlog of 0 holds one pending connection.
        listen(listener, 0).expect("listen");

        let flags = SOCK_STREAM | SOCK_CLOEXEC | SOCK_NONBLOCK;
        let first = socket(AF_UNIX, flags, 0).expect("socket");
        connect_un(first, &addr).expect("queued");
        let second = socket(AF_UNIX, flags, 0).expect("socket");
        assert_eq!(connect_un(second, &addr), Err(Errno::EAGAIN));

        // Accepting makes room, and the same socket connects on retry.
        let accepted = accept(listener).expect("accept");
        connect_un(second, &addr).expect("retried");

        for fd in [accepted, second, first, listener] {
            fd::close(fd).expect("close");
        }
        std::fs::remove_file(&path).expect("remove");
    }

    #[test]
    fn sockaddr_in_roundtrips_octets_and_port() {
        let a = SockAddrIn::new([192, 168, 1, 42], 8080);
        assert_eq!(a.octets(), [192, 168, 1, 42]);
        assert_eq!(a.port(), 8080);
    }

    #[test]
    fn sockaddr_in6_roundtrips_octets_and_port() {
        let a = SockAddrIn6::new(LOOPBACK6, 53);
        assert_eq!(a.octets(), LOOPBACK6);
        assert_eq!(a.port(), 53);
    }

    #[test]
    fn tcp_loopback_echo_ipv4() {
        // Bind to a fixed high port rather than an ephemeral one (port 0) to
        // keep this test independent of getsockname_in (exercised in its own
        // test below); retry a couple of times in the unlikely case it's
        // already in use.
        let mut last_err = None;
        for port in [58231u16, 58232, 58233] {
            let listener = socket(AF_INET, SOCK_STREAM, 0).expect("socket");
            if bind_in(listener, &SockAddrIn::new(LOOPBACK, port)).is_err() {
                fd::close(listener).ok();
                last_err = Some(port);
                continue;
            }
            listen(listener, 1).expect("listen");

            let client = socket(AF_INET, SOCK_STREAM, 0).expect("socket");
            connect_in(client, &SockAddrIn::new(LOOPBACK, port)).expect("connect");

            let (server_side, peer) = accept4_in(listener, 0).expect("accept4");
            assert_eq!(peer.octets(), LOOPBACK);

            send(client, b"ping", 0).expect("send");
            let mut buf = [0u8; 4];
            let n = recv(server_side, &mut buf, 0).expect("recv");
            assert_eq!(&buf[..n], b"ping");

            send(server_side, b"pong", 0).expect("send reply");
            let mut reply = [0u8; 4];
            let n = recv(client, &mut reply, 0).expect("recv reply");
            assert_eq!(&reply[..n], b"pong");

            shutdown(client, SHUT_RDWR).ok();
            fd::close(client).ok();
            fd::close(server_side).ok();
            fd::close(listener).ok();
            return;
        }
        panic!("all candidate ports were in use: {last_err:?}");
    }

    #[test]
    fn udp_loopback_datagram_ipv4() {
        let server = socket(AF_INET, SOCK_DGRAM, 0).expect("socket");
        bind_in(server, &SockAddrIn::new(LOOPBACK, 58234)).expect("bind");

        let client = socket(AF_INET, SOCK_DGRAM, 0).expect("socket");

        sendto_in(client, b"hello", 0, &SockAddrIn::new(LOOPBACK, 58234)).expect("sendto");

        let mut buf = [0u8; 5];
        let (n, from) = recvfrom_in(server, &mut buf, 0).expect("recvfrom");
        assert_eq!(&buf[..n], b"hello");
        assert_eq!(from.octets(), LOOPBACK);

        fd::close(client).ok();
        fd::close(server).ok();
    }

    #[test]
    fn connect_refused_when_nothing_listens() {
        let client = socket(AF_INET, SOCK_STREAM, 0).expect("socket");
        // Nothing should be listening on this port.
        let err = connect_in(client, &SockAddrIn::new(LOOPBACK, 58239)).unwrap_err();
        assert_eq!(err, Errno::ECONNREFUSED);
        fd::close(client).ok();
    }

    #[test]
    fn bad_domain_is_eafnosupport_or_einval() {
        let err = socket(999, SOCK_STREAM, 0).unwrap_err();
        // EAFNOSUPPORT (97) on Linux; accept EINVAL too in case of kernel
        // variance rather than hardcode a magic number this crate has no
        // named constant for.
        assert!(err == Errno(97) || err == Errno::EINVAL, "{err:?}");
    }

    #[test]
    fn accept_on_non_listening_socket_is_einval() {
        let s = socket(AF_INET, SOCK_STREAM, 0).expect("socket");
        assert_eq!(accept(s), Err(Errno::EINVAL));
        fd::close(s).ok();
    }

    #[test]
    fn getsockname_and_getpeername_report_the_real_endpoints() {
        // Bind to an ephemeral port (0) and let the kernel pick one --
        // exactly the port-discovery use case getsockname_in exists for.
        let listener = socket(AF_INET, SOCK_STREAM, 0).expect("socket");
        bind_in(listener, &SockAddrIn::new(LOOPBACK, 0)).expect("bind");
        listen(listener, 1).expect("listen");
        let listener_addr = getsockname_in(listener).expect("getsockname listener");
        assert_eq!(listener_addr.octets(), LOOPBACK);
        assert_ne!(listener_addr.port(), 0, "kernel should have picked a port");

        let client = socket(AF_INET, SOCK_STREAM, 0).expect("socket");
        connect_in(client, &SockAddrIn::new(LOOPBACK, listener_addr.port())).expect("connect");
        let client_addr = getsockname_in(client).expect("getsockname client");
        assert_eq!(client_addr.octets(), LOOPBACK);
        assert_ne!(client_addr.port(), 0);

        let (server_side, _) = accept4_in(listener, 0).expect("accept4");

        // The server side's peer is the client's own (ephemeral) address.
        let peer_of_server = getpeername_in(server_side).expect("getpeername server");
        assert_eq!(peer_of_server.octets(), LOOPBACK);
        assert_eq!(peer_of_server.port(), client_addr.port());

        // The client's peer is the listener's address.
        let peer_of_client = getpeername_in(client).expect("getpeername client");
        assert_eq!(peer_of_client.octets(), LOOPBACK);
        assert_eq!(peer_of_client.port(), listener_addr.port());

        fd::close(client).ok();
        fd::close(server_side).ok();
        fd::close(listener).ok();
    }

    #[test]
    fn getsockname_on_unbound_socket_reports_wildcard() {
        let s = socket(AF_INET, SOCK_DGRAM, 0).expect("socket");
        let addr = getsockname_in(s).expect("getsockname");
        assert_eq!(addr.octets(), [0, 0, 0, 0]);
        assert_eq!(addr.port(), 0);
        fd::close(s).ok();
    }

    #[test]
    fn getpeername_on_unconnected_socket_is_enotconn() {
        let s = socket(AF_INET, SOCK_STREAM, 0).expect("socket");
        assert_eq!(getpeername_in(s), Err(Errno(107))); // ENOTCONN
        fd::close(s).ok();
    }

    fn inode(fd: i32) -> (u64, u64) {
        use std::os::fd::BorrowedFd;
        use std::os::unix::fs::MetadataExt;
        // SAFETY: `fd` is open for the duration of the borrow.
        let file = unsafe { BorrowedFd::borrow_raw(fd) }
            .try_clone_to_owned()
            .expect("dup");
        let meta = std::fs::File::from(file).metadata().expect("stat");
        (meta.dev(), meta.ino())
    }

    fn pair() -> (
        std::os::unix::net::UnixStream,
        std::os::unix::net::UnixStream,
    ) {
        std::os::unix::net::UnixStream::pair().expect("pair")
    }

    #[test]
    fn descriptors_arrive_in_order_close_on_exec() {
        use std::os::fd::AsRawFd;
        let (a, b) = pair();
        let (r, w) = fd::pipe2(0).expect("pipe");
        let file = std::fs::File::open("/proc/self/exe").expect("open");
        let sent = [w, file.as_raw_fd(), r];
        assert_eq!(send_with_fds(a.as_raw_fd(), b"L", &sent, 0), Ok(1));
        let mut buf = [0u8; 4];
        let got = recv_with_fds(b.as_raw_fd(), &mut buf, 0).expect("recv");
        assert_eq!(
            (got.bytes, &buf[..1], got.count, got.truncated),
            (1, &b"L"[..], 3, false)
        );
        for (theirs, ours) in got.fds().iter().zip(sent) {
            assert_ne!(*theirs, ours);
            assert_eq!(inode(*theirs), inode(ours));
            let flags = fd::fcntl(*theirs, fd::F_GETFD, 0).expect("F_GETFD");
            assert_eq!(flags & fd::FD_CLOEXEC, fd::FD_CLOEXEC);
        }
        for fd in got.fds().iter().chain(&[r, w]) {
            fd::close(*fd).expect("close");
        }
    }

    #[test]
    fn data_without_descriptors_reads_as_none() {
        use std::os::fd::AsRawFd;
        let (a, b) = pair();
        assert_eq!(send_with_fds(a.as_raw_fd(), b"plain", &[], 0), Ok(5));
        let mut buf = [0u8; 8];
        let got = recv_with_fds(b.as_raw_fd(), &mut buf, 0).expect("recv");
        assert_eq!((got.bytes, got.count, got.truncated), (5, 0, false));
        drop(a);
        let got = recv_with_fds(b.as_raw_fd(), &mut buf, 0).expect("recv");
        assert_eq!((got.bytes, got.count), (0, 0));
    }

    #[test]
    fn too_many_or_no_byte_is_refused() {
        use std::os::fd::AsRawFd;
        let (a, _b) = pair();
        let fds = [a.as_raw_fd(); MAX_FDS + 1];
        assert_eq!(
            send_with_fds(a.as_raw_fd(), b"x", &fds, 0),
            Err(Errno::EINVAL)
        );
        assert_eq!(
            send_with_fds(a.as_raw_fd(), b"", &fds[..1], 0),
            Err(Errno::EINVAL)
        );
    }

    #[test]
    fn more_than_fit_is_truncated_and_the_rest_closed() {
        use std::os::fd::AsRawFd;
        // Send MAX_FDS + 1 with a raw sendmsg the API itself refuses to make.
        let (a, b) = pair();
        let one = a.as_raw_fd();
        let n = MAX_FDS + 1;
        let len = CMSG_HDR + n * 4;
        #[repr(C, align(8))]
        struct Big([u8; 64]);
        let mut control = Big([0; 64]);
        let header = CmsgHdr {
            len,
            level: SOL_SOCKET,
            ty: SCM_RIGHTS,
        };
        // SAFETY: 8-aligned buffer, larger than the header.
        unsafe { core::ptr::write(control.0.as_mut_ptr().cast::<CmsgHdr>(), header) };
        for i in 0..n {
            control.0[CMSG_HDR + i * 4..CMSG_HDR + i * 4 + 4].copy_from_slice(&one.to_ne_bytes());
        }
        let byte = *b"x";
        let iov = IoVec {
            base: byte.as_ptr() as usize,
            len: 1,
        };
        let msg = MsgHdr {
            name: 0,
            namelen: 0,
            iov: &iov as *const IoVec as usize,
            iovlen: 1,
            control: control.0.as_ptr() as usize,
            controllen: len.next_multiple_of(8),
            flags: 0,
        };
        // SAFETY: as in `send_with_fds`.
        let ret = unsafe { syscall3(nr::SENDMSG, one as usize, &msg as *const MsgHdr as usize, 0) };
        assert_eq!(from_ret(ret), Ok(1));
        let mut buf = [0u8; 1];
        let got = recv_with_fds(b.as_raw_fd(), &mut buf, 0).expect("recv");
        assert!(got.truncated);
        assert!(got.count <= MAX_FDS);
        for fd in got.fds() {
            fd::close(*fd).expect("close");
        }
    }
}
