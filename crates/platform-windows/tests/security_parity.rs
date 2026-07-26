//! Security parity suite (RFC v2 R5+, D15): behavior-spec-derived
//! assertion set run against every backend, the same shape the Fs/Net
//! suites established.

#![cfg(windows)]

use std::path::Path;

use platform::security::{CredentialStore, CredentialStoreStatus, Csprng, Sandbox, SandboxStatus};

/// `fill_random` fills the whole buffer, and two consecutive calls don't
/// return the same bytes (the one property every named consumer — a
/// nonce, a confounder — actually relies on: real, non-repeating
/// randomness, not any particular distribution).
fn assert_security_behavior(csprng: &dyn Csprng) {
    let mut a = [0u8; 32];
    csprng.fill_random(&mut a).expect("fill_random");
    assert!(
        a.iter().any(|&b| b != 0),
        "buffer was never actually written"
    );

    let mut b = [0u8; 32];
    csprng.fill_random(&mut b).expect("fill_random");
    assert_ne!(a, b, "two consecutive fills returned identical bytes");

    // A zero-length request is a valid no-op, not an error.
    csprng.fill_random(&mut []).expect("empty fill_random");

    // A request larger than a single getrandom(2)/BCryptGenRandom call
    // reliably fills in one go (the >256-byte chunking case).
    let mut large = [0u8; 4096];
    csprng.fill_random(&mut large).expect("large fill_random");
    assert!(large.iter().any(|&b| b != 0));
}

#[test]
fn mock_security_conforms() {
    assert_security_behavior(&platform_mock::MockCsprng::new());
}

#[test]
fn windows_security_conforms() {
    assert_security_behavior(&platform_windows::WindowsCsprng);
}

/// See the Linux copy of this test for why `Unsupported`, not
/// enforcement, is what both mock and Windows are expected to report.
#[test]
fn mock_sandbox_reports_unsupported() {
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

/// See the Linux copy of this test for the full contract. No cleanup
/// step: there is no `delete` in this slice's scope (rustils#76), and a
/// fresh CI runner's Credential Manager store doesn't persist between
/// runs anyway — a distinctive test-only service name is enough to avoid
/// colliding with anything real.
fn assert_credential_store_behavior(store: &dyn CredentialStore) {
    let svc = format!("rustils-test-svc-{}", std::process::id());
    assert_eq!(store.available(), CredentialStoreStatus::Available);
    assert_eq!(store.get(&svc, "alice").unwrap(), None);

    store
        .set(&svc, "alice", b"alice-secret")
        .unwrap();
    store.set(&svc, "bob", b"bob-secret").unwrap();
    assert_eq!(
        store.get(&svc, "alice").unwrap(),
        Some(b"alice-secret".to_vec())
    );
    assert_eq!(
        store.get(&svc, "bob").unwrap(),
        Some(b"bob-secret".to_vec())
    );

    store
        .set(&svc, "alice", b"new-secret")
        .unwrap();
    assert_eq!(
        store.get(&svc, "alice").unwrap(),
        Some(b"new-secret".to_vec())
    );
}

#[test]
fn mock_credential_store_conforms() {
    assert_credential_store_behavior(&platform_mock::MockCredentialStore::new());
}

/// Live-verified against the real Credential Manager (`CredWriteW`/
/// `CredReadW`) — not a mock, actual OS state on the CI runner.
#[test]
fn windows_credential_store_conforms() {
    assert_credential_store_behavior(&platform_windows::WindowsCredentialStore);
}
