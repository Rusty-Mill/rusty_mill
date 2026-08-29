//! Sovereign Networking abstractions for rusty_std.

use crate::error::Result;
use crate::io::{Read, Write};

/// Socket address type representing IP and port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketAddr {
    /// IP address string/bytes representation.
    pub ip: [u8; 4],
    /// Port number.
    pub port: u16,
}

/// Sovereign TCP stream connection.
pub struct TcpStream {
    addr: SocketAddr,
}

impl TcpStream {
    /// Connects to a remote socket address.
    pub fn connect(addr: SocketAddr) -> Result<Self> {
        Ok(Self { addr })
    }

    /// Returns the remote peer address.
    pub fn peer_addr(&self) -> Result<SocketAddr> {
        Ok(self.addr)
    }
}

impl Read for TcpStream {
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

impl Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
