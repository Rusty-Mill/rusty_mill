//! Security parity suite (RFC v2 R5+, D15) for the BSD backend — this
//! crate's only security surface today (`TrustAnchors`, rustils#88).
//!
//! The assertion set lives in `platform-parity`; this file records only
//! that it applies here. Mock conformance moved to
//! `platform-mock/tests/parity_conformance.rs` when the sets were
//! extracted.
//!
//! The `cfg` must stay textually identical to `lib.rs`'s — see
//! `net_parity.rs`'s doc comment for why.
#![cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]

use platform::error::ErrorKind;
use platform::security::TrustAnchors;
use platform_parity::security::assert_trust_anchors_behavior;

/// The real backend against this machine's actual trust store —
/// Security.framework on the macOS runner, PEM files on the FreeBSD and
/// OpenBSD VM legs. Skips rather than fails where no store exists: a
/// minimal VM image legitimately has none, and `NotFound` is the
/// backend's documented answer there, not a bug to fail CI over.
///
/// This is the leg that would catch a Core Foundation ownership mistake
/// in the Darwin path — the class of bug no static check finds, and the
/// reason rustils#48/#86 insisted on real-OS runners. The shared set's
/// two-loads-agree assertion is what makes it detectable: a
/// released-too-early array would not survive being run twice.
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
