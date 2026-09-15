//! Security assertion sets (RFC v2 R5+, D15) — the behavior
//! `docs/behavior/security.md` specifies, asserted against any backend.
//!
//! `Sandbox` has no set here: its whole contract is a `SandboxStatus`
//! that legitimately differs per backend and per host kernel, so there
//! is no cross-backend behavior to assert. Its per-backend expectations
//! stay in the suites that own them.

use platform::security::{CredentialStore, CredentialStoreStatus, Csprng, TrustAnchors};

/// `fill_random` fills the whole buffer, and two consecutive calls don't
/// return the same bytes (the one property every named consumer — a
/// nonce, a confounder — actually relies on: real, non-repeating
/// randomness, not any particular distribution).
pub fn assert_csprng_behavior(csprng: &dyn Csprng) {
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

/// See the Linux copy of this test for the full contract. No cleanup
/// step: there is no `delete` in this slice's scope (rustils#76), and a
/// fresh CI runner's Credential Manager store doesn't persist between
/// runs anyway — a distinctive test-only service name is enough to avoid
/// colliding with anything real.
pub fn assert_credential_store_behavior(store: &dyn CredentialStore) {
    let svc = format!("rustils-test-svc-{}", std::process::id());
    assert_eq!(store.available(), CredentialStoreStatus::Available);
    assert_eq!(store.get(&svc, "alice").unwrap(), None);

    store.set(&svc, "alice", b"alice-secret").unwrap();
    store.set(&svc, "bob", b"bob-secret").unwrap();
    assert_eq!(
        store.get(&svc, "alice").unwrap(),
        Some(b"alice-secret".to_vec())
    );
    assert_eq!(
        store.get(&svc, "bob").unwrap(),
        Some(b"bob-secret".to_vec())
    );

    store.set(&svc, "alice", b"new-secret").unwrap();
    assert_eq!(
        store.get(&svc, "alice").unwrap(),
        Some(b"new-secret".to_vec())
    );
}

/// The three contract rules every backend owes, per
/// `platform::security::TrustAnchors` and `docs/behavior/security.md`.
///
/// Deliberately asserts nothing about *which* anchors come back, or how
/// many: that is whatever the host machine happens to trust, and pinning
/// it would make this suite a test of the CI image rather than of the
/// backend. What is universal is the shape of the answer.
pub fn assert_trust_anchors_behavior(anchors: &dyn TrustAnchors) {
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
