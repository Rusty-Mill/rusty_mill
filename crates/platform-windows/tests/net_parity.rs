//! Net parity suite (RFC v2 R5+, D16) for the Windows backend.
//!
//! The assertion sets live in `platform-parity`; this file records only
//! which of them apply here. See `platform_parity`'s crate doc for why
//! the extraction happened when it did.

#![cfg(windows)]

use platform_parity::net::{assert_net_behavior, assert_udp_behavior, assert_unix_behavior};

#[test]
fn windows_net_conforms() {
    assert_net_behavior(&platform_windows::WindowsNet);
}

/// Windows has had `AF_UNIX` since 1803; the shared set applies
/// unchanged, which is the whole point of it being shared.
#[test]
fn windows_unix_conforms() {
    assert_unix_behavior(&platform_windows::WindowsNet, "windows");
}

#[test]
fn windows_udp_conforms() {
    assert_udp_behavior(&platform_windows::WindowsNet);
}
