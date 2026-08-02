//! Net parity suite (RFC v2 R5+, D16) for the Linux backend.
//!
//! The assertion sets live in `platform-parity`; this file records only
//! *which* of them apply here. That split landed once `platform-bsd`
//! made this the third copy — the trigger the previous version of this file's
//! own doc comment named. See `platform_parity`'s crate doc for what the
//! copies had already drifted into by then.

#![cfg(target_os = "linux")]

use platform_parity::net::{assert_net_behavior, assert_udp_behavior, assert_unix_behavior};

#[test]
fn linux_net_conforms() {
    assert_net_behavior(&platform_linux::LinuxNet);
}

#[test]
fn linux_unix_conforms() {
    assert_unix_behavior(&platform_linux::LinuxNet, "linux");
}

#[test]
fn linux_udp_conforms() {
    assert_udp_behavior(&platform_linux::LinuxNet);
}
