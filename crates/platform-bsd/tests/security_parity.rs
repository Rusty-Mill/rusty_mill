//! Security parity suite for the BSD backend (rustils#88) — the first
//! security coverage on this crate, which was net-only until the
//! `TrustAnchors` slice landed.
//!
//! Kept textually identical to `platform-linux`/`platform-windows`'s
//! copies of `assert_trust_anchors_behavior`, the same convention the
//! net parity suites already follow. The recorded follow-up there — pull
//! the shared assertions into one crate once a third copy appears —
//! applies here too and is now due for this function specifically.
//!
//! The file-level `cfg` must stay textually identical to `lib.rs`'s:
//! integration tests don't inherit the library crate's gate, and without
//! it a Linux or Windows host tries to build this file against a `libc`
//! that isn't even a dependency there.
#![cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]

use platform::error::ErrorKind;
use platform::security::TrustAnchors;

/// The three contract rules every backend owes, per
/// `platform::security::TrustAnchors` and `docs/behavior/security.md`.
///
/// Asserts nothing about *which* anchors come back or how many — that is
/// whatever the host trusts, and pinning it would test the CI image
/// rather than the backend.
fn assert_trust_anchors_behavior(anchors: &dyn TrustAnchors) {
    let loaded = anchors.load_anchors().expect("load_anchors");

    // 1. Never `Ok(vec![])` — zero anchors is the error path.
    assert!(
        !loaded.is_empty(),
        "a successful load must never return an empty anchor set"
    );

    // 2. Every anchor is non-empty DER, starting with ASN.1's SEQUENCE
    //    tag — the one structural claim available without parsing.
    for (i, der) in loaded.iter().enumerate() {
        assert!(!der.is_empty(), "anchor {i} is empty");
        assert_eq!(
            der[0], 0x30,
            "anchor {i} does not begin with the DER SEQUENCE tag — not a certificate"
        );
    }

    // 3. Stateless: two loads agree. On Darwin this additionally catches
    //    a Core Foundation ownership mistake — a double `CFRelease` or a
    //    released-too-early array would not survive being run twice.
    let again = anchors.load_anchors().expect("second load_anchors");
    assert_eq!(loaded, again, "two loads disagreed");
}

#[test]
fn mock_trust_anchors_conforms() {
    assert_trust_anchors_behavior(&platform_mock::MockTrustAnchors::new());
}

#[test]
fn empty_trust_store_fails_closed_rather_than_returning_nothing() {
    let empty = platform_mock::MockTrustAnchors::empty();
    let err = empty
        .load_anchors()
        .expect_err("an empty store must fail, not return an empty Vec");
    assert_eq!(err.kind, ErrorKind::NotFound);
}

/// The real backend against this machine's actual trust store —
/// Security.framework on the macOS runner, PEM files on the FreeBSD and
/// OpenBSD VM legs. Skips rather than fails where no store exists: a
/// minimal VM image legitimately has none, and `NotFound` is the
/// backend's documented answer there, not a bug to fail CI over.
///
/// This is the leg that would catch a Core Foundation ownership mistake
/// in the Darwin path — the class of bug no static check finds, and the
/// reason rustils#48/#86 insisted on real-OS runners.
#[test]
fn bsd_trust_anchors_conforms() {
    let anchors = platform_bsd::BsdTrustAnchors;
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
