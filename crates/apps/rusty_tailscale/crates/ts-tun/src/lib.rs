//! TUN device, addressing/routes, and a MagicDNS hosts stub.
//!
//! Linux-first (Phase 4). The device is created and configured through
//! rustils' `platform_linux::LinuxTunDevice` (rustils#56, decision D14),
//! then wrapped in `rusty_tokio`'s `AsyncFd` for async packet I/O — the same
//! "rustils-backed fd on the runtime's reactor" shape `ts-magicsock` already
//! established for `LinuxUdpSocket`. `ts-engine` bridges this to the
//! WireGuard-over-DERP data plane.
//!
//! A `wintun` adapter (Windows) and the `TunDevice` trait's second impl land
//! in Phase 7; for now there is a single concrete Linux [`Tun`].

pub mod magicdns;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

/// Non-Linux targets have no TUN adapter yet (`wintun` is deferred). The
/// embedded node (`ts-net`) needs none; `Tun::create` fails with `Unsupported`.
#[cfg(not(target_os = "linux"))]
mod unsupported;
#[cfg(not(target_os = "linux"))]
pub use unsupported::*;
