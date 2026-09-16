//! `Net`/`TcpStream`/`TcpListener` trait impls over the sys layer. No
//! `unsafe` here.
//!
//! Each concrete socket type also gets (rustils#41, rusty_tail's
//! `rusty_tokio` hand-rolled async runtime, a reactor built directly
//! against `platform-linux` rather than reimplementing socket setup):
//!
//! - `AsFd`/`AsRawFd`, delegating to the private `OwnedFd` — matching
//!   the std-interop precedent `LinuxFile`/`LinuxDir` (`fs.rs`) already
//!   established, not a new convention.
//! - `From<OwnedFd>`, so an externally-created fd (std's own socket
//!   types, or anything else already `OwnedFd`) can be adopted directly,
//!   the same "any fd works" shape `LinuxFile`'s own `From<OwnedFd>`
//!   documents.
//! - A concrete constructor (`connect`/`bind`, matching `std::net`'s own
//!   naming) that calls the exact same `sysnet::` sockaddr-packing/
//!   error-mapping/stale-socket-cleanup logic `Net`'s trait impl below
//!   uses — returning the concrete type directly instead of `Box<dyn
//!   Trait>`. This is the missing piece without which `AsFd`/
//!   `set_nonblocking` would be unreachable: `Net::tcp_connect` and
//!   friends only ever hand out `Box<dyn TcpStream>` etc., which erases
//!   the concrete type with no safe way back (the traits are
//!   object-safe, not `Any`) — deliberately left that way rather than
//!   widening the object-safe traits themselves, per RFC v2 §5.1's
//!   instance/object-safe/std-interop split.
//!
//! `Net`'s own trait methods are thin wrappers over these constructors
//! now, not a second copy of the same logic.

use std::net::SocketAddr;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

use platform::error::Result;
use platform::net::{Net, TcpListener, TcpStream, UdpSocket, UnixListener, UnixStream};

use crate::sys::fdio;
use crate::sys::net as sysnet;

/// The Linux backend's [`Net`] capability. Stateless — every operation
/// is a fresh syscall, mirroring how [`crate::LinuxSpawner`] carries no
/// state of its own either.
pub struct LinuxNet;

impl Net for LinuxNet {
    fn tcp_connect(&self, addr: SocketAddr) -> Result<Box<dyn TcpStream>> {
        Ok(Box::new(LinuxTcpStream::connect(addr)?))
    }

    fn tcp_listen(&self, addr: SocketAddr) -> Result<Box<dyn TcpListener>> {
        Ok(Box::new(LinuxTcpListener::bind(addr)?))
    }

    fn unix_connect(&self, path: &Path) -> Result<Box<dyn UnixStream>> {
        Ok(Box::new(LinuxUnixStream::connect(path)?))
    }

    fn unix_listen(&self, path: &Path) -> Result<Box<dyn UnixListener>> {
        Ok(Box::new(LinuxUnixListener::bind(path)?))
    }

    fn udp_bind(&self, addr: SocketAddr) -> Result<Box<dyn UdpSocket>> {
        Ok(Box::new(LinuxUdpSocket::bind(addr)?))
    }
}

/// A connected TCP stream backed by an `OwnedFd`. Public for std interop
/// (RFC v2 §5.1), the same reasoning `LinuxFile` documents.
pub struct LinuxTcpStream {
    fd: OwnedFd,
}

impl LinuxTcpStream {
    /// `socket` + `connect`, blocking until the connection completes or
    /// fails — the same `sysnet::tcp_connect` [`Net::tcp_connect`] calls,
    /// returned as the concrete type instead of `Box<dyn TcpStream>`.
    pub fn connect(addr: SocketAddr) -> Result<Self> {
        Ok(Self {
            fd: sysnet::tcp_connect(addr)?,
        })
    }

    /// Toggle `O_NONBLOCK` on the underlying fd (rustils#41). Additive:
    /// existing blocking callers are unaffected unless they opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.fd, nonblocking)
    }
}

impl TcpStream for LinuxTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // A connected socket's fd is read/written exactly like a plain
        // fd — no socket-specific syscall needed for the byte-transfer
        // path itself, only for setup/teardown/options.
        fdio::read(&self.fd, buf)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        fdio::write(&self.fd, buf)
    }

    fn set_nodelay(&self, nodelay: bool) -> Result<()> {
        sysnet::set_nodelay(&self.fd, nodelay)
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        sysnet::peer_addr(&self.fd)
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        sysnet::local_addr(&self.fd)
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        sysnet::set_read_timeout(&self.fd, timeout)
    }
}

impl AsFd for LinuxTcpStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for LinuxTcpStream {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// Any already-connected stream socket's fd works as a [`LinuxTcpStream`]
/// — the same "any fd works" shape `LinuxFile`'s own `From<OwnedFd>`
/// documents.
impl From<OwnedFd> for LinuxTcpStream {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

/// A listening TCP socket backed by an `OwnedFd`.
pub struct LinuxTcpListener {
    fd: OwnedFd,
}

impl LinuxTcpListener {
    /// `socket` + `SO_REUSEADDR` + `bind` + `listen`, the same
    /// `sysnet::tcp_listen` [`Net::tcp_listen`] calls, returned as the
    /// concrete type instead of `Box<dyn TcpListener>`.
    pub fn bind(addr: SocketAddr) -> Result<Self> {
        Ok(Self {
            fd: sysnet::tcp_listen(addr)?,
        })
    }

    /// `accept4`, returning the concrete [`LinuxTcpStream`] directly —
    /// the concrete-typed counterpart of [`TcpListener::accept`], named
    /// to match `std::net::TcpListener::accept`.
    pub fn accept(&self) -> Result<(LinuxTcpStream, SocketAddr)> {
        let (fd, peer) = sysnet::tcp_accept(&self.fd)?;
        Ok((LinuxTcpStream { fd }, peer))
    }

    /// Toggle `O_NONBLOCK` on the underlying fd (rustils#41). Additive:
    /// existing blocking callers are unaffected unless they opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.fd, nonblocking)
    }
}

impl TcpListener for LinuxTcpListener {
    fn accept(&self) -> Result<(Box<dyn TcpStream>, SocketAddr)> {
        let (stream, peer) = LinuxTcpListener::accept(self)?;
        Ok((Box::new(stream), peer))
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        sysnet::local_addr(&self.fd)
    }
}

impl AsFd for LinuxTcpListener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for LinuxTcpListener {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl From<OwnedFd> for LinuxTcpListener {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

/// A connected Unix domain stream socket backed by an `OwnedFd`. Public
/// for the same std-interop reasoning `LinuxTcpStream` documents.
pub struct LinuxUnixStream {
    fd: OwnedFd,
}

impl LinuxUnixStream {
    /// `socket` + `connect`, the same `sysnet::unix_connect`
    /// [`Net::unix_connect`] calls, returned as the concrete type
    /// instead of `Box<dyn UnixStream>`.
    pub fn connect(path: &Path) -> Result<Self> {
        Ok(Self {
            fd: sysnet::unix_connect(path)?,
        })
    }

    /// Toggle `O_NONBLOCK` on the underlying fd (rustils#41). Additive:
    /// existing blocking callers are unaffected unless they opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.fd, nonblocking)
    }
}

impl UnixStream for LinuxUnixStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Same reasoning as `LinuxTcpStream::read`: a connected AF_UNIX
        // socket's fd is read exactly like a plain fd.
        fdio::read(&self.fd, buf)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        fdio::write(&self.fd, buf)
    }

    fn peer_addr(&self) -> Result<Option<PathBuf>> {
        sysnet::unix_peer_addr(&self.fd)
    }

    fn local_addr(&self) -> Result<Option<PathBuf>> {
        sysnet::unix_local_addr(&self.fd)
    }
}

impl AsFd for LinuxUnixStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for LinuxUnixStream {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl From<OwnedFd> for LinuxUnixStream {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

/// A listening Unix domain socket backed by an `OwnedFd`.
pub struct LinuxUnixListener {
    fd: OwnedFd,
}

impl LinuxUnixListener {
    /// `socket` + `bind` (stale-cleanup retried once) + mode-`0600`
    /// `chmod` + `listen`, the same `sysnet::unix_listen`
    /// [`Net::unix_listen`] calls, returned as the concrete type instead
    /// of `Box<dyn UnixListener>`.
    pub fn bind(path: &Path) -> Result<Self> {
        Ok(Self {
            fd: sysnet::unix_listen(path)?,
        })
    }

    /// `accept4`, returning the concrete [`LinuxUnixStream`] directly —
    /// the concrete-typed counterpart of [`UnixListener::accept`], named
    /// to match `std::os::unix::net::UnixListener::accept`.
    pub fn accept(&self) -> Result<(LinuxUnixStream, Option<PathBuf>)> {
        let (fd, peer) = sysnet::unix_accept(&self.fd)?;
        Ok((LinuxUnixStream { fd }, peer))
    }

    /// Toggle `O_NONBLOCK` on the underlying fd (rustils#41). Additive:
    /// existing blocking callers are unaffected unless they opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.fd, nonblocking)
    }
}

impl UnixListener for LinuxUnixListener {
    fn accept(&self) -> Result<(Box<dyn UnixStream>, Option<PathBuf>)> {
        let (stream, peer) = LinuxUnixListener::accept(self)?;
        Ok((Box::new(stream), peer))
    }

    fn local_addr(&self) -> Result<Option<PathBuf>> {
        sysnet::unix_local_addr(&self.fd)
    }
}

impl AsFd for LinuxUnixListener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for LinuxUnixListener {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl From<OwnedFd> for LinuxUnixListener {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

/// A UDP datagram socket backed by an `OwnedFd`.
pub struct LinuxUdpSocket {
    fd: OwnedFd,
}

impl LinuxUdpSocket {
    /// `socket` + `bind`, the same `sysnet::udp_bind` [`Net::udp_bind`]
    /// calls, returned as the concrete type instead of `Box<dyn
    /// UdpSocket>`.
    pub fn bind(addr: SocketAddr) -> Result<Self> {
        Ok(Self {
            fd: sysnet::udp_bind(addr)?,
        })
    }

    /// Toggle `O_NONBLOCK` on the underlying fd (rustils#41). Additive:
    /// existing blocking callers are unaffected unless they opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.fd, nonblocking)
    }
}

impl UdpSocket for LinuxUdpSocket {
    fn send_to(&self, buf: &[u8], addr: SocketAddr) -> Result<usize> {
        sysnet::udp_send_to(&self.fd, buf, addr)
    }

    fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        sysnet::udp_recv_from(&self.fd, buf)
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        sysnet::local_addr(&self.fd)
    }
}

impl AsFd for LinuxUdpSocket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for LinuxUdpSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl From<OwnedFd> for LinuxUdpSocket {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}
