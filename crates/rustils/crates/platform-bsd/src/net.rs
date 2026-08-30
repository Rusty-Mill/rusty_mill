//! `Net`/`TcpStream`/`TcpListener`/`UnixStream`/`UnixListener`/
//! `UdpSocket` trait impls over the sys layer (rustils#48). No `unsafe`
//! here. Mirrors `platform-linux::net`'s shape exactly, including the
//! rustils#41 std-interop surface (`AsFd`/`AsRawFd`, `From<OwnedFd>`,
//! concrete constructors, `set_nonblocking`) ported here as part of this
//! crate's first slice rather than left as a follow-up, per the issue's
//! own request.

use std::net::SocketAddr;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

use platform::error::Result;
use platform::net::{Net, TcpListener, TcpStream, UdpSocket, UnixListener, UnixStream};

use crate::sys::fdio;
use crate::sys::net as sysnet;

/// The BSD backend's [`Net`] capability. Stateless, mirroring
/// [`crate::BsdNet`]'s Linux counterpart `LinuxNet`.
pub struct BsdNet;

impl Net for BsdNet {
    fn tcp_connect(&self, addr: SocketAddr) -> Result<Box<dyn TcpStream>> {
        Ok(Box::new(BsdTcpStream::connect(addr)?))
    }

    fn tcp_listen(&self, addr: SocketAddr) -> Result<Box<dyn TcpListener>> {
        Ok(Box::new(BsdTcpListener::bind(addr)?))
    }

    fn unix_connect(&self, path: &Path) -> Result<Box<dyn UnixStream>> {
        Ok(Box::new(BsdUnixStream::connect(path)?))
    }

    fn unix_listen(&self, path: &Path) -> Result<Box<dyn UnixListener>> {
        Ok(Box::new(BsdUnixListener::bind(path)?))
    }

    fn udp_bind(&self, addr: SocketAddr) -> Result<Box<dyn UdpSocket>> {
        Ok(Box::new(BsdUdpSocket::bind(addr)?))
    }
}

/// A connected TCP stream backed by an `OwnedFd`. Public for std
/// interop (RFC v2 §5.1).
pub struct BsdTcpStream {
    fd: OwnedFd,
}

impl BsdTcpStream {
    /// `socket` + `connect`, blocking until the connection completes or
    /// fails.
    pub fn connect(addr: SocketAddr) -> Result<Self> {
        Ok(Self {
            fd: sysnet::tcp_connect(addr)?,
        })
    }

    /// Toggle `O_NONBLOCK` on the underlying fd.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.fd, nonblocking)
    }
}

impl TcpStream for BsdTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
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

impl AsFd for BsdTcpStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for BsdTcpStream {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl From<OwnedFd> for BsdTcpStream {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

/// A listening TCP socket backed by an `OwnedFd`.
pub struct BsdTcpListener {
    fd: OwnedFd,
}

impl BsdTcpListener {
    /// `socket` + `SO_REUSEADDR` + `bind` + `listen`.
    pub fn bind(addr: SocketAddr) -> Result<Self> {
        Ok(Self {
            fd: sysnet::tcp_listen(addr)?,
        })
    }

    /// `accept` (no `accept4` in this crate's libc surface), returning
    /// the concrete [`BsdTcpStream`] directly.
    pub fn accept(&self) -> Result<(BsdTcpStream, SocketAddr)> {
        let (fd, peer) = sysnet::tcp_accept(&self.fd)?;
        Ok((BsdTcpStream { fd }, peer))
    }

    /// Toggle `O_NONBLOCK` on the underlying fd.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.fd, nonblocking)
    }
}

impl TcpListener for BsdTcpListener {
    fn accept(&self) -> Result<(Box<dyn TcpStream>, SocketAddr)> {
        let (stream, peer) = BsdTcpListener::accept(self)?;
        Ok((Box::new(stream), peer))
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        sysnet::local_addr(&self.fd)
    }
}

impl AsFd for BsdTcpListener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for BsdTcpListener {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl From<OwnedFd> for BsdTcpListener {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

/// A connected Unix domain stream socket backed by an `OwnedFd`.
pub struct BsdUnixStream {
    fd: OwnedFd,
}

impl BsdUnixStream {
    /// `socket` + `connect`.
    pub fn connect(path: &Path) -> Result<Self> {
        Ok(Self {
            fd: sysnet::unix_connect(path)?,
        })
    }

    /// Toggle `O_NONBLOCK` on the underlying fd.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.fd, nonblocking)
    }
}

impl UnixStream for BsdUnixStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
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

impl AsFd for BsdUnixStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for BsdUnixStream {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl From<OwnedFd> for BsdUnixStream {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

/// A listening Unix domain socket backed by an `OwnedFd`.
pub struct BsdUnixListener {
    fd: OwnedFd,
}

impl BsdUnixListener {
    /// `socket` + `bind` (stale-cleanup retried once) + mode-`0600`
    /// `chmod` + `listen`.
    pub fn bind(path: &Path) -> Result<Self> {
        Ok(Self {
            fd: sysnet::unix_listen(path)?,
        })
    }

    /// `accept` (no `accept4` in this crate's libc surface), returning
    /// the concrete [`BsdUnixStream`] directly.
    pub fn accept(&self) -> Result<(BsdUnixStream, Option<PathBuf>)> {
        let (fd, peer) = sysnet::unix_accept(&self.fd)?;
        Ok((BsdUnixStream { fd }, peer))
    }

    /// Toggle `O_NONBLOCK` on the underlying fd.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.fd, nonblocking)
    }
}

impl UnixListener for BsdUnixListener {
    fn accept(&self) -> Result<(Box<dyn UnixStream>, Option<PathBuf>)> {
        let (stream, peer) = BsdUnixListener::accept(self)?;
        Ok((Box::new(stream), peer))
    }

    fn local_addr(&self) -> Result<Option<PathBuf>> {
        sysnet::unix_local_addr(&self.fd)
    }
}

impl AsFd for BsdUnixListener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for BsdUnixListener {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl From<OwnedFd> for BsdUnixListener {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

/// A UDP datagram socket backed by an `OwnedFd`.
pub struct BsdUdpSocket {
    fd: OwnedFd,
}

impl BsdUdpSocket {
    /// `socket` + `bind`.
    pub fn bind(addr: SocketAddr) -> Result<Self> {
        Ok(Self {
            fd: sysnet::udp_bind(addr)?,
        })
    }

    /// Toggle `O_NONBLOCK` on the underlying fd.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.fd, nonblocking)
    }
}

impl UdpSocket for BsdUdpSocket {
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

impl AsFd for BsdUdpSocket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for BsdUdpSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl From<OwnedFd> for BsdUdpSocket {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}
