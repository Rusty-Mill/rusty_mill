//! Stand-in `Tun` for targets without a TUN adapter. Cannot be constructed.

use std::io;
use std::net::Ipv4Addr;

/// Tailscale's standard tunnel MTU.
pub const DEFAULT_MTU: u32 = 1280;

/// Uninhabited: `create` always fails, so no value exists.
#[derive(Clone)]
pub enum Tun {}

impl Tun {
    /// Always fails with `Unsupported`.
    pub fn create(_name: &str, _ipv4: Ipv4Addr, _prefix_len: u8, _mtu: u32) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "TUN is not available on this target"))
    }

    pub fn name(&self) -> &str {
        match *self {}
    }

    pub fn ipv4(&self) -> Ipv4Addr {
        match *self {}
    }

    pub async fn recv(&self) -> io::Result<Vec<u8>> {
        match *self {}
    }

    pub async fn send(&self, _packet: &[u8]) -> io::Result<()> {
        match *self {}
    }
}
