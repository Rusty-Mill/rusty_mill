//! Sovereign Networking abstractions for rusty_std.

use crate::error::{Error, Result};
use crate::io::{Read, Write};

/// Socket address type representing IP and port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketAddr {
    /// IP address string/bytes representation.
    pub ip: [u8; 4],
    /// Port number.
    pub port: u16,
}

#[cfg(target_os = "linux")]
fn map_errno(op: &str, err: rusty_libc::Errno) -> Error {
    Error::Io(err.code(), alloc::format!("{op}: {err}"))
}

#[cfg(windows)]
fn map_win32(op: &str, err: rusty_win32::Win32Error) -> Error {
    Error::Io(err.code() as i32, alloc::format!("{op}: {err}"))
}

/// Sovereign TCP stream connection.
///
/// Backed by a real OS socket: `rusty_libc::socket` on Linux, Winsock
/// (`rusty_win32::net`) on Windows -- `connect` performs a genuine
/// `connect(2)`/`connect`, `read`/`write` genuinely `recv`/`send` on the
/// wire. On any other target (e.g. `wasm32`) there is no wired socket
/// implementation yet: `connect` stores the address without opening a real
/// socket, `write` unconditionally reports every byte as sent, and `read`
/// always reports immediate EOF -- callers on those targets must not treat
/// this as real network I/O.
pub struct TcpStream {
    addr: SocketAddr,
    #[cfg(target_os = "linux")]
    fd: i32,
    #[cfg(windows)]
    sock: rusty_win32::net::RawSocket,
}

impl TcpStream {
    /// Connects to a remote socket address, opening a real OS socket on
    /// Linux/Windows (see the type-level docs for the unwired-target
    /// fallback).
    pub fn connect(addr: SocketAddr) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let fd = rusty_libc::socket::socket(
                rusty_libc::socket::AF_INET,
                rusty_libc::socket::SOCK_STREAM,
                0,
            )
            .map_err(|e| map_errno("socket", e))?;
            let sock_addr = rusty_libc::socket::SockAddrIn::new(addr.ip, addr.port);
            rusty_libc::socket::connect_in(fd, &sock_addr).map_err(|e| {
                let _ = rusty_libc::fd::close(fd);
                map_errno("connect", e)
            })?;
            Ok(Self { addr, fd })
        }
        #[cfg(windows)]
        {
            rusty_win32::net::startup().map_err(|e| map_win32("WSAStartup", e))?;
            let sock = rusty_win32::net::socket(
                rusty_win32::net::AddressFamily::Inet,
                rusty_win32::net::SocketKind::Stream,
                rusty_win32::net::Protocol::Tcp,
            )
            .map_err(|e| {
                let _ = rusty_win32::net::cleanup();
                map_win32("socket", e)
            })?;
            let win_addr = rusty_win32::net::SocketAddr::V4 {
                ip: addr.ip,
                port: addr.port,
            };
            // SAFETY: `sock` was just opened above by `net::socket` and has
            // not yet been closed or handed to any other owner.
            unsafe { rusty_win32::net::connect(sock, &win_addr) }.map_err(|e| {
                // SAFETY: `sock` is still open and uniquely owned here.
                let _ = unsafe { rusty_win32::net::close_socket(sock) };
                let _ = rusty_win32::net::cleanup();
                map_win32("connect", e)
            })?;
            Ok(Self { addr, sock })
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Ok(Self { addr })
        }
    }

    /// Returns the remote peer address.
    pub fn peer_addr(&self) -> Result<SocketAddr> {
        Ok(self.addr)
    }
}

impl Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        #[cfg(target_os = "linux")]
        {
            rusty_libc::fd::read(self.fd, buf).map_err(|e| map_errno("read", e))
        }
        #[cfg(windows)]
        {
            // SAFETY: `self.sock` was opened by `TcpStream::connect` and is
            // only ever closed once, in `Drop`, after which this
            // `TcpStream` (and any `&mut` borrow of it) can no longer be
            // used.
            unsafe { rusty_win32::net::recv(self.sock, buf) }.map_err(|e| map_win32("recv", e))
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Ok(0)
        }
    }
}

impl Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        #[cfg(target_os = "linux")]
        {
            rusty_libc::fd::write(self.fd, buf).map_err(|e| map_errno("write", e))
        }
        #[cfg(windows)]
        {
            // SAFETY: `self.sock` was opened by `TcpStream::connect` and is
            // only ever closed once, in `Drop`, after which this
            // `TcpStream` (and any `&mut` borrow of it) can no longer be
            // used.
            unsafe { rusty_win32::net::send(self.sock, buf) }.map_err(|e| map_win32("send", e))
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            let _ = rusty_libc::fd::close(self.fd);
        }
        #[cfg(windows)]
        {
            // SAFETY: `self.sock` is a currently-open socket uniquely
            // owned by this `TcpStream`, closed exactly once here and
            // never used again afterward.
            let _ = unsafe { rusty_win32::net::close_socket(self.sock) };
            let _ = rusty_win32::net::cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real syscall-backed round-trip: proves `TcpStream` actually opens a
    // socket and exchanges bytes over the wire (rusty_libc on Linux,
    // rusty_win32 on Windows) rather than being the no-op stub this module
    // started as, which reported every write as fully sent and every read
    // as immediate EOF regardless of whether a peer was even listening.
    #[test]
    fn write_then_read_round_trips_over_a_real_loopback_socket() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind std listener");
        let port = listener.local_addr().expect("local_addr").port();

        let accept_thread = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 5];
            std::io::Read::read_exact(&mut sock, &mut buf).expect("std read");
            std::io::Write::write_all(&mut sock, &buf).expect("std echo");
        });

        let mut stream = TcpStream::connect(SocketAddr {
            ip: [127, 0, 0, 1],
            port,
        })
        .expect("TcpStream::connect should reach the real loopback listener");

        stream
            .write_all(b"hello")
            .expect("write_all should succeed");

        let mut echoed = [0u8; 5];
        let mut read_total = 0;
        while read_total < echoed.len() {
            let n = stream
                .read(&mut echoed[read_total..])
                .expect("read should succeed");
            assert!(n > 0, "read returned 0 before all bytes arrived");
            read_total += n;
        }
        assert_eq!(&echoed, b"hello");

        accept_thread
            .join()
            .expect("accept thread should not panic");
    }

    #[test]
    fn connect_to_a_closed_port_fails() {
        // Bind a socket to get a genuinely free ephemeral port, then drop
        // it immediately so nothing is listening there.
        let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
        let port = probe.local_addr().expect("local_addr").port();
        drop(probe);

        let result = TcpStream::connect(SocketAddr {
            ip: [127, 0, 0, 1],
            port,
        });
        assert!(
            result.is_err(),
            "connecting to a closed port should fail, not silently succeed"
        );
    }
}
