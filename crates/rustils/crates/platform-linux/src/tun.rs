//! `Tun`/`TunDevice` trait impls over the sys layer (RFC v2 R5+, D14). No
//! `unsafe` here.
//!
//! `LinuxTunDevice` also gets `AsFd`/`AsRawFd`, `From<OwnedFd>`, and an
//! inherent `set_nonblocking` (rustils#41's precedent) plus a concrete
//! `create` constructor returning the concrete type directly instead of
//! `Box<dyn TunDevice>` — the same reasoning `net.rs`'s own module doc
//! documents: a consumer building its own reactor (rusty_tail's `ts-tun`,
//! wrapping this in tokio's `AsyncFd`, exactly like `ts-magicsock`
//! already does for `LinuxUdpSocket`) needs the concrete type to reach
//! them, since `Tun::create` only ever hands back the object-safe,
//! type-erased `Box<dyn TunDevice>`.

use std::net::Ipv4Addr;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};

use platform::error::Result;
use platform::tun::{Tun, TunDevice};

use crate::sys::fdio;
use crate::sys::tun as systun;

/// The Linux backend's [`Tun`] capability. Stateless, mirroring
/// [`crate::LinuxNet`].
pub struct LinuxTun;

impl Tun for LinuxTun {
    fn create(
        &self,
        name: &str,
        ipv4: Ipv4Addr,
        prefix_len: u8,
        mtu: u32,
    ) -> Result<Box<dyn TunDevice>> {
        Ok(Box::new(LinuxTunDevice::create(
            name, ipv4, prefix_len, mtu,
        )?))
    }
}

/// A created, configured TUN device backed by an `OwnedFd`. Public for
/// std/reactor interop (RFC v2 §5.1), the same reasoning `LinuxTcpStream`
/// documents.
pub struct LinuxTunDevice {
    fd: OwnedFd,
    name: String,
}

impl LinuxTunDevice {
    /// `open("/dev/net/tun")` + `TUNSETIFF` + the `SIOCSIF*` addressing/
    /// MTU/bring-up dance, the same `systun::create_tun`/`configure`
    /// [`Tun::create`] calls, returned as the concrete type instead of
    /// `Box<dyn TunDevice>`.
    pub fn create(name: &str, ipv4: Ipv4Addr, prefix_len: u8, mtu: u32) -> Result<Self> {
        let fd = systun::create_tun(name)?;
        systun::configure(name, ipv4, prefix_len, mtu)?;
        Ok(Self {
            fd,
            name: name.to_string(),
        })
    }

    /// Toggle `O_NONBLOCK` on the underlying fd (rustils#41's Net
    /// precedent, applied here). Additive: existing blocking callers are
    /// unaffected unless they opt in.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        crate::sys::net::set_nonblocking(&self.fd, nonblocking)
    }
}

impl TunDevice for LinuxTunDevice {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        fdio::read(&self.fd, buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        fdio::write(&self.fd, buf)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl AsFd for LinuxTunDevice {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for LinuxTunDevice {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// Any already-open TUN fd works as a [`LinuxTunDevice`] — the same
/// "any fd works" shape `LinuxFile`'s own `From<OwnedFd>` documents.
/// `name` is reported empty since an adopted fd carries no name of its
/// own; callers that need it should track it themselves.
impl From<OwnedFd> for LinuxTunDevice {
    fn from(fd: OwnedFd) -> Self {
        Self {
            fd,
            name: String::new(),
        }
    }
}
