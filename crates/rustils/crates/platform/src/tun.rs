//! Tun / virtual-link surface (RFC v2 R5+, decision D14) — single named
//! consumer, the same single-consumer precedent that already justified
//! `process`/`fs`/`term` (RFC v2 §3).
//!
//! rusty_tail's `ts-tun` hand-rolls `/dev/net/tun` + `TUNSETIFF` + the
//! `SIOCSIFADDR`/`SIOCSIFNETMASK`/`SIOCSIFMTU`/`SIOCGIFFLAGS`/
//! `SIOCSIFFLAGS` configuration dance directly against `libc`, then wraps
//! the resulting fd in tokio's own `AsyncFd` for async packet I/O
//! (`ts-tun/src/{lib,sys}.rs`). This trait mirrors that exact shape —
//! creation, IPv4/prefix/MTU configuration, and bring-up bundled into one
//! call, since that's the whole of what the named consumer's own
//! `Tun::create(name, ipv4, prefix_len, mtu)` already does — not
//! decomposed into finer-grained ioctls (a bare create without
//! addressing, address-only reconfiguration of an existing device, etc.)
//! no one has asked for.
//!
//! Like the Net surface's raw-fd escape hatch (rustils#41/#42), the
//! concrete Linux backend type ships `AsFd`/`AsRawFd`/`set_nonblocking`/a
//! concrete `create` constructor from day one rather than as a follow-up
//! issue: `ts-tun`, exactly like `ts-magicsock` before it (converged onto
//! `platform_linux::LinuxUdpSocket`), needs to register the device's fd
//! with tokio's own reactor, not drive I/O through this trait's blocking
//! calls directly.
//!
//! Windows (`wintun`) is `Unsupported` until a Windows consumer names
//! itself — `ts-tun` itself is `#![cfg(target_os = "linux")]` only today,
//! so there is no donor evidence for the Windows shape yet, the same
//! judgment call every other Windows-`Unsupported` capability in this
//! crate already makes rather than guessing at an unvalidated design.

use std::net::Ipv4Addr;

use crate::error::Result;

/// A created, configured TUN device. Object-safe.
pub trait TunDevice {
    /// Read the next outbound IP packet the kernel routed into the
    /// tunnel. Blocking.
    fn read(&self, buf: &mut [u8]) -> Result<usize>;

    /// Write an inbound IP packet into the tunnel, for the local network
    /// stack to receive as if it arrived over the wire. Blocking.
    fn write(&self, buf: &[u8]) -> Result<usize>;

    /// The interface name the OS actually assigned — may differ from
    /// what was requested (e.g. truncated to fit the platform's own
    /// interface-name length limit).
    fn name(&self) -> &str;
}

/// A backend capable of creating TUN devices. Object-safe.
pub trait Tun {
    /// Create a TUN device, assign `ipv4`/`prefix_len` (installing the
    /// connected route for that subnet, so no explicit route command is
    /// needed), set `mtu`, and bring the interface up. Requires elevated
    /// privilege (`CAP_NET_ADMIN` on Linux).
    fn create(
        &self,
        name: &str,
        ipv4: Ipv4Addr,
        prefix_len: u8,
        mtu: u32,
    ) -> Result<Box<dyn TunDevice>>;
}
