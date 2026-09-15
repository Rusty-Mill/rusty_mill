//! Net parity suite (RFC v2 R5+, D16) for the BSD backend.
//!
//! The assertion sets live in `platform-parity`; this file records only
//! which of them apply here. This backend being the *third* net
//! implementation is what triggered that extraction — see
//! `platform_parity`'s crate doc, including the comment drift this
//! file's own previous copy had already accumulated.
//!
//! The `cfg` must stay textually identical to `lib.rs`'s: integration
//! tests don't inherit the library crate's gate, and without it a Linux
//! or Windows host tries to build this file against a `libc` that isn't
//! a dependency there.
#![cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]

use platform_parity::net::{assert_net_behavior, assert_udp_behavior, assert_unix_behavior};

#[test]
fn bsd_net_conforms() {
    assert_net_behavior(&platform_bsd::BsdNet);
}

#[test]
fn bsd_unix_conforms() {
    assert_unix_behavior(&platform_bsd::BsdNet, "bsd");
}

#[test]
fn bsd_udp_conforms() {
    assert_udp_behavior(&platform_bsd::BsdNet);
}
