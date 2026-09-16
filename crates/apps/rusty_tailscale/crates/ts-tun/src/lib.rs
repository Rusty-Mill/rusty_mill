//! TUN device, addressing/routes, and a MagicDNS hosts stub.
//!
//! Linux-first (Phase 4). The device is created and configured through
//! rustils' `platform_linux::LinuxTunDevice` (rustils#56, decision D14),
//! then wrapped in tokio's `AsyncFd` for async packet I/O — the same
//! "rustils-backed fd on tokio's reactor" shape `ts-magicsock` already
//! established for `LinuxUdpSocket`. `ts-engine` bridges this to the
//! WireGuard-over-DERP data plane.
//!
//! A `wintun` adapter (Windows) and the `TunDevice` trait's second impl land
//! in Phase 7; for now there is a single concrete Linux [`Tun`].

#![cfg(target_os = "linux")]

pub mod magicdns;

use std::io;
use std::net::Ipv4Addr;
use std::sync::Arc;

use platform::error::{ErrorKind as PlatformErrorKind, PlatformError};
use platform::tun::TunDevice as PlatformTunDevice;
use platform_linux::LinuxTunDevice;
use tokio::io::unix::AsyncFd;

/// Tailscale's standard tunnel MTU.
pub const DEFAULT_MTU: u32 = 1280;
/// Largest packet we read from the device in one call.
const READ_BUF: usize = 65_536;

/// An async Linux TUN device carrying raw IPv4/IPv6 packets (no packet-info
/// prefix). Cloneable: the clone shares the same underlying fd, so one task
/// can `recv` while another `send`s.
#[derive(Clone)]
pub struct Tun {
    fd: Arc<AsyncFd<LinuxTunDevice>>,
    name: String,
    ipv4: Ipv4Addr,
}

impl Tun {
    /// Creates and configures a TUN device: assigns `ipv4`/`prefix_len`
    /// (which auto-installs the connected route), sets `mtu`, and brings it
    /// up. Requires `CAP_NET_ADMIN`.
    pub fn create(name: &str, ipv4: Ipv4Addr, prefix_len: u8, mtu: u32) -> io::Result<Self> {
        let dev = LinuxTunDevice::create(name, ipv4, prefix_len, mtu).map_err(from_platform_err)?;
        dev.set_nonblocking(true).map_err(from_platform_err)?;
        Ok(Self {
            fd: Arc::new(AsyncFd::new(dev)?),
            name: name.to_string(),
            ipv4,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn ipv4(&self) -> Ipv4Addr {
        self.ipv4
    }

    /// Reads the next outbound IP packet the kernel routed into the tunnel.
    pub async fn recv(&self) -> io::Result<Vec<u8>> {
        loop {
            let mut guard = self.fd.readable().await?;
            match guard.try_io(|inner| {
                let mut buf = vec![0u8; READ_BUF];
                let n = inner.get_ref().read(&mut buf).map_err(from_platform_err)?;
                buf.truncate(n);
                Ok(buf)
            }) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }

    /// Writes an inbound IP packet into the tunnel for the local stack.
    pub async fn send(&self, packet: &[u8]) -> io::Result<()> {
        loop {
            let mut guard = self.fd.writable().await?;
            match guard.try_io(|inner| inner.get_ref().write(packet).map_err(from_platform_err)) {
                Ok(result) => return result.map(|_| ()),
                Err(_would_block) => continue,
            }
        }
    }
}

/// Maps a rustils [`PlatformError`] to [`io::Error`], keeping the
/// operation/path context in the error's `Display` (via `source`) while
/// giving `AsyncFd::try_io` an accurate [`io::ErrorKind`] to recognize
/// `WouldBlock` by — `platform-linux` already maps `EAGAIN` to
/// `ErrorKind::WouldBlock`, so this just carries that through.
fn from_platform_err(e: PlatformError) -> io::Error {
    let kind = match e.kind {
        PlatformErrorKind::NotFound => io::ErrorKind::NotFound,
        PlatformErrorKind::PermissionDenied => io::ErrorKind::PermissionDenied,
        PlatformErrorKind::AlreadyExists => io::ErrorKind::AlreadyExists,
        PlatformErrorKind::WouldBlock => io::ErrorKind::WouldBlock,
        PlatformErrorKind::Interrupted => io::ErrorKind::Interrupted,
        PlatformErrorKind::BrokenPipe => io::ErrorKind::BrokenPipe,
        PlatformErrorKind::ConnectionRefused => io::ErrorKind::ConnectionRefused,
        PlatformErrorKind::ConnectionReset => io::ErrorKind::ConnectionReset,
        PlatformErrorKind::ConnectionAborted => io::ErrorKind::ConnectionAborted,
        PlatformErrorKind::NotConnected => io::ErrorKind::NotConnected,
        PlatformErrorKind::AddrInUse => io::ErrorKind::AddrInUse,
        PlatformErrorKind::AddrNotAvailable => io::ErrorKind::AddrNotAvailable,
        PlatformErrorKind::TimedOut => io::ErrorKind::TimedOut,
        PlatformErrorKind::InvalidInput => io::ErrorKind::InvalidInput,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, e)
}
