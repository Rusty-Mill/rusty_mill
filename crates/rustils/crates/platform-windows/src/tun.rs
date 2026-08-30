//! `Tun` stub (RFC v2 R5+, D14). `wintun` is `Unsupported` until a
//! Windows consumer names itself — rusty_tail's `ts-tun` (the only named
//! consumer for this surface) is `#![cfg(target_os = "linux")]` only, so
//! there is no donor evidence for the Windows shape yet. Reports
//! `Unsupported` explicitly rather than omitting the module, matching
//! every other Windows-`Unsupported` capability in this workspace
//! (`Sandbox`'s `SandboxStatus::Unsupported`, `Signal`'s
//! Windows-`Unsupported` variants) — a caller always gets an honest
//! answer, never a missing implementation to trip over.

use std::net::Ipv4Addr;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::tun::{Tun, TunDevice};

/// The Windows backend's [`Tun`] capability. Always `Unsupported` — see
/// this module's doc comment.
pub struct WindowsTun;

impl Tun for WindowsTun {
    fn create(
        &self,
        _name: &str,
        _ipv4: Ipv4Addr,
        _prefix_len: u8,
        _mtu: u32,
    ) -> Result<Box<dyn TunDevice>> {
        Err(PlatformError::new(
            ErrorKind::Unsupported,
            OsCode::None,
            "Tun::create (no wintun backend — no Windows consumer named yet)",
        ))
    }
}
