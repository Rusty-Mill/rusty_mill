//! Security parity suite (RFC v2 R5+, D15) for the Windows backend.
//!
//! The assertion sets live in `platform-parity`; this file records only
//! which of them apply here. Mock conformance moved to
//! `platform-mock/tests/parity_conformance.rs` when the sets were
//! extracted.

#![cfg(windows)]

use platform::security::CredentialStore;
use platform_parity::security::{
    assert_credential_store_behavior, assert_csprng_behavior, assert_trust_anchors_behavior,
};

#[test]
fn windows_csprng_conforms() {
    assert_csprng_behavior(&platform_windows::WindowsCsprng);
}

/// Live-verified against the real Credential Manager (`CredWriteW`/
/// `CredReadW`) — not a mock, actual OS state on the CI runner. The
/// shared set scopes its service name by process id precisely so this
/// leg can't collide with a concurrently-running test binary.
#[test]
fn windows_credential_store_conforms() {
    assert_credential_store_behavior(&platform_windows::WindowsCredentialStore);
}

/// Regression test (finding 48, CODEX-MONOREPO-REVIEW-2026-09-12): an
/// empty stored secret produces `CredentialBlob == NULL` with
/// `CredentialBlobSize == 0` per `CREDENTIALW`'s own docs, which
/// `std::slice::from_raw_parts` cannot accept even for a zero-length
/// slice. `credential_get`'s non-`track-w` arm must special-case that
/// shape instead of handing the null pointer straight to
/// `from_raw_parts`.
#[test]
fn windows_credential_store_round_trips_an_empty_secret() {
    let svc = format!("rustils-test-empty-svc-{}", std::process::id());
    let store = platform_windows::WindowsCredentialStore;
    store.set(&svc, "alice", &[]).unwrap();
    assert_eq!(store.get(&svc, "alice").unwrap(), Some(Vec::new()));
}

/// Live-verified against the real ROOT certificate store
/// (`CertOpenSystemStoreW`/`CertEnumCertificatesInStore`) — actual OS
/// state on the CI runner, not a mock.
#[test]
fn windows_trust_anchors_conforms() {
    assert_trust_anchors_behavior(&platform_windows::WindowsTrustAnchors);
}
