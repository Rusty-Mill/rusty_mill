//! `Net`/`TcpStream`/`TcpListener`/`UnixStream`/`UnixListener` trait
//! impls over the sys layer. No `unsafe` here.
//!
//! Each concrete socket type also gets (rustils#59, mirroring the
//! rustils#41/#42 Linux precedent — forced by rusty_tail's
//! `rusty_tokio` hand-rolled async runtime scoping a Windows/IOCP
//! reactor backend, `rusty_tokio#6`, the same consumer #41/#48 already
//! served on Linux/macOS):
//!
//! - `AsRawSocket`, delegating to the private `sysnet::OwnedSocket`.
//!   Raw-handle exposure only (no `AsSocket`/ownership-transfer
//!   interop) — `sysnet::OwnedSocket` is this crate's own newtype, not
//!   std's `std::os::windows::io::OwnedSocket`, and nothing has asked
//!   for adopting an externally-created socket on Windows the way
//!   `LinuxTcpStream`'s `From<OwnedFd>` does for Unix; that's a bigger
//!   step this issue's own text flags as "not required for this
//!   issue's core ask."
//! - `set_nonblocking`, backed by `ioctlsocket(FIONBIO, ...)`.
//! - A concrete constructor (`connect`/`bind`, matching `std::net`'s
//!   own naming) that calls the exact same `sysnet::` sockaddr-packing/
//!   error-mapping logic `Net`'s trait impl below uses — returning the
//!   concrete type directly instead of `Box<dyn Trait>`. This is the
//!   missing piece without which `AsRawSocket`/`set_nonblocking` would
//!   be unreachable: `Net::tcp_connect` and friends only ever hand out
//!   `Box<dyn TcpStream>` etc., which erases the concrete type with no
//!   safe way back (the traits are object-safe, not `Any`) —
//!   deliberately left that way rather than widening the object-safe
//!   traits themselves, per RFC v2 §5.1's instance/object-safe/
//!   std-interop split. Exactly the same reasoning `platform-linux`'s
//!   `net.rs` module doc gives for its own escape hatch.
//!
//! `Net`'s own trait methods are thin wrappers over these constructors
//! now, not a second copy of the same logic.

use std::net::SocketAddr;
use std::os::windows::io::{AsRawSocket, RawSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

use platform::error::Result;
use platform::net::{Net, TcpListener, TcpStream, UdpSocket, UnixListener, UnixStream};

use crate::sys::net as sysnet;

/// The Windows backend's [`Net`] capability. Stateless — every
/// operation is a fresh Winsock call, mirroring
/// [`crate::WindowsSpawner`].
pub struct WindowsNet;

impl Net for WindowsNet {
    fn tcp_connect(&self, addr: SocketAddr) -> Result<Box<dyn TcpStream>> {
        Ok(Box::new(WindowsTcpStream::connect(addr)?))
    }

    fn tcp_listen(&self, addr: SocketAddr) -> Result<Box<dyn TcpListener>> {
        Ok(Box::new(WindowsTcpListener::bind(addr)?))
    }

    fn unix_connect(&self, path: &Path) -> Result<Box<dyn UnixStream>> {
        Ok(Box::new(WindowsUnixStream::connect(path)?))
    }

    fn unix_listen(&self, path: &Path) -> Result<Box<dyn UnixListener>> {
        Ok(Box::new(WindowsUnixListener::bind(path)?))
    }

    fn udp_bind(&self, addr: SocketAddr) -> Result<Box<dyn UdpSocket>> {
        Ok(Box::new(WindowsUdpSocket::bind(addr)?))
    }
}

/// A connected TCP stream backed by an owned Winsock socket. Public for
/// std interop (RFC v2 §5.1), the same reasoning `LinuxTcpStream`
/// documents.
pub struct WindowsTcpStream {
    sock: sysnet::OwnedSocket,
}

impl WindowsTcpStream {
    /// `socket` + `connect`, blocking until the connection completes or
    /// fails — the same `sysnet::tcp_connect` [`Net::tcp_connect`]
    /// calls, returned as the concrete type instead of `Box<dyn
    /// TcpStream>`.
    pub fn connect(addr: SocketAddr) -> Result<Self> {
        Ok(Self {
            sock: sysnet::tcp_connect(addr)?,
        })
    }

    /// Toggle non-blocking mode on the underlying socket (rustils#59).
    /// Additive: existing blocking callers are unaffected unless they
    /// opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.sock, nonblocking)
    }
}

impl TcpStream for WindowsTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        sysnet::read(&self.sock, buf)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sysnet::write(&self.sock, buf)
    }

    fn set_nodelay(&self, nodelay: bool) -> Result<()> {
        sysnet::set_nodelay(&self.sock, nodelay)
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        sysnet::peer_addr(&self.sock)
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        sysnet::local_addr(&self.sock)
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        sysnet::set_read_timeout(&self.sock, timeout)
    }
}

impl AsRawSocket for WindowsTcpStream {
    fn as_raw_socket(&self) -> RawSocket {
        self.sock.raw() as RawSocket
    }
}

/// A listening TCP socket backed by an owned Winsock socket.
pub struct WindowsTcpListener {
    sock: sysnet::OwnedSocket,
}

impl WindowsTcpListener {
    /// `socket` + `SO_REUSEADDR` + `bind` + `listen`, the same
    /// `sysnet::tcp_listen` [`Net::tcp_listen`] calls, returned as the
    /// concrete type instead of `Box<dyn TcpListener>`.
    pub fn bind(addr: SocketAddr) -> Result<Self> {
        Ok(Self {
            sock: sysnet::tcp_listen(addr)?,
        })
    }

    /// `accept`, returning the concrete [`WindowsTcpStream`] directly —
    /// the concrete-typed counterpart of [`TcpListener::accept`], named
    /// to match `std::net::TcpListener::accept`.
    pub fn accept(&self) -> Result<(WindowsTcpStream, SocketAddr)> {
        let (sock, peer) = sysnet::tcp_accept(&self.sock)?;
        Ok((WindowsTcpStream { sock }, peer))
    }

    /// Toggle non-blocking mode on the underlying socket (rustils#59).
    /// Additive: existing blocking callers are unaffected unless they
    /// opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.sock, nonblocking)
    }
}

impl TcpListener for WindowsTcpListener {
    fn accept(&self) -> Result<(Box<dyn TcpStream>, SocketAddr)> {
        let (stream, peer) = WindowsTcpListener::accept(self)?;
        Ok((Box::new(stream), peer))
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        sysnet::local_addr(&self.sock)
    }
}

impl AsRawSocket for WindowsTcpListener {
    fn as_raw_socket(&self) -> RawSocket {
        self.sock.raw() as RawSocket
    }
}

/// A connected Unix domain stream socket backed by an owned Winsock
/// socket. Public for the same std-interop reasoning `WindowsTcpStream`
/// documents.
pub struct WindowsUnixStream {
    sock: sysnet::OwnedSocket,
}

impl WindowsUnixStream {
    /// `socket` + `connect`, the same `sysnet::unix_connect`
    /// [`Net::unix_connect`] calls, returned as the concrete type
    /// instead of `Box<dyn UnixStream>`.
    pub fn connect(path: &Path) -> Result<Self> {
        Ok(Self {
            sock: sysnet::unix_connect(path)?,
        })
    }

    /// Toggle non-blocking mode on the underlying socket (rustils#59).
    /// Additive: existing blocking callers are unaffected unless they
    /// opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.sock, nonblocking)
    }
}

impl UnixStream for WindowsUnixStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        sysnet::read(&self.sock, buf)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sysnet::write(&self.sock, buf)
    }

    fn peer_addr(&self) -> Result<Option<PathBuf>> {
        sysnet::unix_peer_addr(&self.sock)
    }

    fn local_addr(&self) -> Result<Option<PathBuf>> {
        sysnet::unix_local_addr(&self.sock)
    }
}

impl AsRawSocket for WindowsUnixStream {
    fn as_raw_socket(&self) -> RawSocket {
        self.sock.raw() as RawSocket
    }
}

/// A listening Unix domain socket backed by an owned Winsock socket.
pub struct WindowsUnixListener {
    sock: sysnet::OwnedSocket,
}

impl WindowsUnixListener {
    /// `socket` + `bind` (stale-cleanup retried once) + `listen`, the
    /// same `sysnet::unix_listen` [`Net::unix_listen`] calls, returned
    /// as the concrete type instead of `Box<dyn UnixListener>`.
    pub fn bind(path: &Path) -> Result<Self> {
        Ok(Self {
            sock: sysnet::unix_listen(path)?,
        })
    }

    /// `accept`, returning the concrete [`WindowsUnixStream`] directly
    /// — the concrete-typed counterpart of [`UnixListener::accept`],
    /// named to match `std::os::unix::net::UnixListener::accept`.
    pub fn accept(&self) -> Result<(WindowsUnixStream, Option<PathBuf>)> {
        let (sock, peer) = sysnet::unix_accept(&self.sock)?;
        Ok((WindowsUnixStream { sock }, peer))
    }

    /// Toggle non-blocking mode on the underlying socket (rustils#59).
    /// Additive: existing blocking callers are unaffected unless they
    /// opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.sock, nonblocking)
    }
}

impl UnixListener for WindowsUnixListener {
    fn accept(&self) -> Result<(Box<dyn UnixStream>, Option<PathBuf>)> {
        let (stream, peer) = WindowsUnixListener::accept(self)?;
        Ok((Box::new(stream), peer))
    }

    fn local_addr(&self) -> Result<Option<PathBuf>> {
        sysnet::unix_local_addr(&self.sock)
    }
}

impl AsRawSocket for WindowsUnixListener {
    fn as_raw_socket(&self) -> RawSocket {
        self.sock.raw() as RawSocket
    }
}

/// A UDP datagram socket backed by an owned Winsock socket.
pub struct WindowsUdpSocket {
    sock: sysnet::OwnedSocket,
}

impl WindowsUdpSocket {
    /// `socket` + `bind`, the same `sysnet::udp_bind` [`Net::udp_bind`]
    /// calls, returned as the concrete type instead of `Box<dyn
    /// UdpSocket>`.
    pub fn bind(addr: SocketAddr) -> Result<Self> {
        Ok(Self {
            sock: sysnet::udp_bind(addr)?,
        })
    }

    /// Toggle non-blocking mode on the underlying socket (rustils#59).
    /// Additive: existing blocking callers are unaffected unless they
    /// opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        sysnet::set_nonblocking(&self.sock, nonblocking)
    }
}

impl UdpSocket for WindowsUdpSocket {
    fn send_to(&self, buf: &[u8], addr: SocketAddr) -> Result<usize> {
        sysnet::udp_send_to(&self.sock, buf, addr)
    }

    fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        sysnet::udp_recv_from(&self.sock, buf)
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        sysnet::local_addr(&self.sock)
    }
}

impl AsRawSocket for WindowsUdpSocket {
    fn as_raw_socket(&self) -> RawSocket {
        self.sock.raw() as RawSocket
    }
}
