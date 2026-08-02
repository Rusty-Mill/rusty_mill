//! Security parity suite (RFC v2 R5+, D15): behavior-spec-derived
//! assertion set run against every backend, the same shape the Fs/Net
//! suites established.

use std::path::Path;

use platform::error::ErrorKind;
use platform::security::{
    CredentialStore, CredentialStoreStatus, Csprng, Sandbox, SandboxStatus, TrustAnchors,
};

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

#[cfg(target_os = "linux")]
#[test]
fn linux_security_conforms() {
    assert_security_behavior(&platform_linux::LinuxCsprng);
}

/// The mock `Sandbox` has no in-memory equivalent of kernel confinement
/// to fake — see `platform-mock`'s own doc comment — so the only
/// contract this backend has is honesty: report `Unsupported`, never
/// silently claim enforcement. Real Landlock/seccomp enforcement is
/// exercised separately, in `tests/security_sandbox.rs` (irreversible
/// for the calling thread, so it needs subprocess isolation this shared
/// parity-suite binary doesn't give it).
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

/// A faithful `CredentialStore` fake (mock, or a real-and-reachable
/// native backend): round-trips a stored secret, distinguishes accounts
/// under the same service, and reports a clean miss for nothing stored.
fn assert_credential_store_behavior(store: &dyn CredentialStore) {
    assert_eq!(store.available(), CredentialStoreStatus::Available);
    assert_eq!(store.get("rustils-test-svc", "alice").unwrap(), None);

    store
        .set("rustils-test-svc", "alice", b"alice-secret")
        .unwrap();
    store.set("rustils-test-svc", "bob", b"bob-secret").unwrap();
    assert_eq!(
        store.get("rustils-test-svc", "alice").unwrap(),
        Some(b"alice-secret".to_vec())
    );
    assert_eq!(
        store.get("rustils-test-svc", "bob").unwrap(),
        Some(b"bob-secret".to_vec())
    );

    store
        .set("rustils-test-svc", "alice", b"new-secret")
        .unwrap();
    assert_eq!(
        store.get("rustils-test-svc", "alice").unwrap(),
        Some(b"new-secret".to_vec())
    );
}

#[test]
fn mock_credential_store_conforms() {
    assert_credential_store_behavior(&platform_mock::MockCredentialStore::new());
}

/// rustils#78's real Linux backend (Secret Service over D-Bus), run in
/// this suite's own environment where no D-Bus session bus is reachable
/// (no `DBUS_SESSION_BUS_ADDRESS`, no daemon running): `available()`
/// reports `Unavailable` — a real mechanism exists on this OS, it's just
/// not reachable right now — and `get`/`set` surface that as a real
/// `Err` rather than a silent `Ok(None)`/`Ok(())`, per the trait's own
/// contract (a clean miss and "the store isn't reachable" are different
/// claims). Live round-trip coverage against a real, reachable Secret
/// Service lives in `tests/secret_service.rs`, which spawns its own
/// `dbus-daemon`/`gnome-keyring-daemon` pair.
#[cfg(target_os = "linux")]
#[test]
fn linux_credential_store_reports_unavailable_with_no_bus_reachable() {
    let store = platform_linux::LinuxCredentialStore;
    assert_eq!(store.available(), CredentialStoreStatus::Unavailable);
    assert!(store.get("svc", "acct").is_err());
    assert!(store.set("svc", "acct", b"secret").is_err());
}

// --- TrustAnchors (rustils#88) -----------------------------------------

/// The three contract rules every backend owes, per
/// `platform::security::TrustAnchors` and `docs/behavior/security.md`.
///
/// Deliberately asserts nothing about *which* anchors come back, or how
/// many: that is whatever the host machine happens to trust, and pinning
/// it would make this suite a test of the CI image rather than of the
/// backend. What is universal is the shape of the answer.
fn assert_trust_anchors_behavior(anchors: &dyn TrustAnchors) {
    let loaded = anchors.load_anchors().expect("load_anchors");

    // 1. Never `Ok(vec![])` — zero anchors is the error path, so a
    //    successful load always carries at least one.
    assert!(
        !loaded.is_empty(),
        "a successful load must never return an empty anchor set"
    );

    // 2. Every anchor is non-empty DER. `0x30` is ASN.1's SEQUENCE tag,
    //    which every X.509 certificate starts with — the one structural
    //    claim this crate can make without parsing anything, and enough
    //    to catch a backend handing back PEM text or an empty blob.
    for (i, der) in loaded.iter().enumerate() {
        assert!(!der.is_empty(), "anchor {i} is empty");
        assert_eq!(
            der[0], 0x30,
            "anchor {i} does not begin with the DER SEQUENCE tag — not a certificate"
        );
    }

    // 3. Stateless: loading twice gives the same answer. Every backend
    //    re-reads rather than caching, so this also catches a backend
    //    that consumed an iterator or freed its store on first use.
    let again = anchors.load_anchors().expect("second load_anchors");
    assert_eq!(loaded, again, "two loads disagreed");
}

#[test]
fn mock_trust_anchors_conforms() {
    assert_trust_anchors_behavior(&platform_mock::MockTrustAnchors::new());
}

/// The fail-closed rule, which only the mock can stage on demand: a
/// store holding nothing is an `Err`, never `Ok(vec![])`. A caller handed
/// an empty set would trust nothing and fail every connection with a
/// confusing per-connection error instead of the real one.
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

/// rustils#88's real Linux backend against this machine's actual trust
/// store. Skips rather than fails where no store exists — a minimal
/// container image legitimately has none, and the backend's documented
/// answer there is `NotFound`, which is not a bug to fail CI over.
#[cfg(target_os = "linux")]
#[test]
fn linux_trust_anchors_conforms() {
    let anchors = platform_linux::LinuxTrustAnchors;
    match anchors.load_anchors() {
        Ok(_) => assert_trust_anchors_behavior(&anchors),
        Err(e) => {
            assert_eq!(
                e.kind,
                ErrorKind::NotFound,
                "a host with no trust store must report NotFound, not {:?}",
                e.kind
            );
            eprintln!("skipping: no OS trust store on this host");
        }
    }
}
