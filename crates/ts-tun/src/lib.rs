//! TUN device, addressing/routes, and a MagicDNS hosts stub.
//!
//! Linux-first (Phase 4). The device is created and configured with pure
//! ioctls (`sys`), then wrapped in tokio's `AsyncFd` for async packet I/O.
//! `ts-engine` bridges this to the WireGuard-over-DERP data plane.
//!
//! A `wintun` adapter (Windows) and the `TunDevice` trait's second impl land
//! in Phase 7; for now there is a single concrete Linux [`Tun`].

#![cfg(target_os = "linux")]

pub mod magicdns;
mod sys;

use std::io;
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

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
    fd: Arc<AsyncFd<TunFd>>,
    name: String,
    ipv4: Ipv4Addr,
}

/// Owns the TUN file descriptor and reports readiness to `AsyncFd`.
struct TunFd(OwnedFd);

impl AsRawFd for TunFd {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.0.as_raw_fd()
    }
}

impl Tun {
    /// Creates and configures a TUN device: assigns `ipv4`/`prefix_len`
    /// (which auto-installs the connected route), sets `mtu`, and brings it
    /// up. Requires `CAP_NET_ADMIN`.
    pub fn create(name: &str, ipv4: Ipv4Addr, prefix_len: u8, mtu: u32) -> io::Result<Self> {
        let fd = sys::create_tun(name)?;
        set_nonblocking(&fd)?;
        sys::configure(name, ipv4, prefix_len, mtu)?;
        Ok(Self {
            fd: Arc::new(AsyncFd::new(TunFd(fd))?),
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
            let raw = self.fd.get_ref().as_raw_fd();
            match guard.try_io(|_| read_fd(raw)) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }

    /// Writes an inbound IP packet into the tunnel for the local stack.
    pub async fn send(&self, packet: &[u8]) -> io::Result<()> {
        loop {
            let mut guard = self.fd.writable().await?;
            let raw = self.fd.get_ref().as_raw_fd();
            match guard.try_io(|_| write_fd(raw, packet)) {
                Ok(result) => return result.map(|_| ()),
                Err(_would_block) => continue,
            }
        }
    }
}

fn set_nonblocking(fd: &OwnedFd) -> io::Result<()> {
    let raw = fd.as_raw_fd();
    // SAFETY: raw is a valid open fd for the duration of these calls.
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn read_fd(fd: std::os::fd::RawFd) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; READ_BUF];
    // SAFETY: buf is valid for READ_BUF bytes; fd is a valid TUN fd.
    let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(n as usize);
    Ok(buf)
}

fn write_fd(fd: std::os::fd::RawFd, packet: &[u8]) -> io::Result<usize> {
    // SAFETY: packet is valid for its length; fd is a valid TUN fd.
    let n = unsafe { libc::write(fd, packet.as_ptr() as *const libc::c_void, packet.len()) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(n as usize)
}
