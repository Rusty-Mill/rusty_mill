//! The mock backend against the shared parity assertion sets.
//!
//! These used to be duplicated into every backend's suite — three
//! identical `mock_*_conforms` tests running the same assertions against
//! the same fake, once per backend crate. They belong here: the mock is
//! this crate's product, so its conformance is this crate's test, and it
//! runs once.
//!
//! The mock is what proves an assertion set is *satisfiable* at all, so
//! a red backend leg means the backend, not the spec. That value comes
//! from running it — not from running it three times.

use platform::error::ErrorKind;
use platform::security::TrustAnchors;
use platform_parity::net::{assert_net_behavior, assert_udp_behavior, assert_unix_behavior};
use platform_parity::security::{assert_credential_store_behavior, assert_csprng_behavior};

#[test]
fn mock_net_conforms() {
    assert_net_behavior(&platform_mock::MockNet);
}

#[test]
fn mock_unix_conforms() {
    assert_unix_behavior(&platform_mock::MockNet, "mock");
}

#[test]
fn mock_udp_conforms() {
    assert_udp_behavior(&platform_mock::MockNet);
}

#[test]
fn mock_csprng_conforms() {
    assert_csprng_behavior(&platform_mock::MockCsprng::new());
}

#[test]
fn mock_credential_store_conforms() {
    assert_credential_store_behavior(&platform_mock::MockCredentialStore::new());
}

#[test]
fn mock_trust_anchors_conforms() {
    platform_parity::security::assert_trust_anchors_behavior(
        &platform_mock::MockTrustAnchors::new(),
    );
}

/// The fail-closed rule, which only the mock can stage on demand: a
/// store holding nothing is an `Err`, never `Ok(vec![])`. A caller
/// handed an empty set would trust nothing and fail every connection
/// with a confusing per-connection error instead of the real one.
///
/// Not in the shared set: a real backend can't be made to hold zero
/// anchors on demand, so this is a mock-only assertion about a contract
/// every backend nonetheless owes.
#[test]
fn empty_trust_store_fails_closed_rather_than_returning_nothing() {
    let empty = platform_mock::MockTrustAnchors::empty();
    let err = empty
        .load_anchors()
        .expect_err("an empty store must fail, not return an empty Vec");
    assert_eq!(err.kind, ErrorKind::NotFound);
}

/// A consumer pinning its own anchor set gets exactly that back — the
/// hermetic-test service this slice exists to provide (`rusty_tls`
/// pinning a throwaway CA rather than depending on the developer's
/// machine trust store).
#[test]
fn mock_trust_anchors_returns_exactly_what_was_pinned() {
    let pinned = vec![vec![0x30, 0x01, 0xAA], vec![0x30, 0x02, 0xBB, 0xCC]];
    let anchors = platform_mock::MockTrustAnchors::with_anchors(pinned.clone());
    assert_eq!(anchors.load_anchors().unwrap(), pinned);

    anchors.set_anchors(vec![vec![0x30, 0x00]]);
    assert_eq!(anchors.load_anchors().unwrap(), vec![vec![0x30, 0x00]]);
}

/// The mock `Sandbox` has no in-memory equivalent of kernel confinement
/// to fake — see this crate's own doc comment — so its only contract is
/// honesty: report `Unsupported`, never silently claim enforcement.
#[test]
fn mock_sandbox_reports_unsupported() {
    use platform::security::{Sandbox, SandboxStatus};
    use std::path::Path;

    let sandbox = platform_mock::MockSandbox;
    let root: &Path = Path::new(".");
    assert_eq!(
        sandbox.confine_filesystem(&[root], &[]).unwrap(),
        SandboxStatus::Unsupported
    );
    assert_eq!(
        sandbox.block_inet_sockets().unwrap(),
        SandboxStatus::Unsupported
    );
}
