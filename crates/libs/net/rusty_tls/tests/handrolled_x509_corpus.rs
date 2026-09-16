//! The hand-rolled parser against certificates nobody here wrote.
//!
//! Every other test of this parser uses certificates generated for the
//! occasion — by `rcgen`, or byte by byte in the rejection suite. Those prove
//! the parser handles what its author thought of. This one is the opposite
//! test, and it is the important one: a deliberately strict parser's real
//! failure mode is being strict about the *wrong* thing, and no hand-written
//! case can find that, because the whole problem is that the author did not
//! think of it.
//!
//! So the corpus is the machine's own trust store, read exactly the way
//! `TrustPolicy::System` reads it. Public root CAs are a genuinely adversarial
//! sample for a new parser: they span three decades of issuance practice, and
//! carry RSA keys, v1 structures with no extensions at all, `GeneralizedTime`
//! dates, twenty-octet serial numbers, and extensions this parser has never
//! heard of.
//!
//! # The differential, and its direction
//!
//! rustls (via webpki) parses each anchor too, and the assertion is
//! one-directional on purpose:
//!
//! > **Every certificate rustls accepts, this parser must also accept.**
//!
//! The converse is deliberately not asserted. rustls and this parser enforce
//! overlapping but not identical rules — this one rejects some things rustls
//! is relaxed about, which is a defensible choice and not a bug. But a
//! certificate a production TLS stack is happy to build a chain from, that
//! this parser cannot even read, is a straightforward defect. That is the
//! direction worth failing on.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use platform::security::TrustAnchors;
use rustls::pki_types::CertificateDer;
use rustls::RootCertStore;
use rusty_tls::handrolled::x509::Certificate;

/// The same backend selection `src/trust.rs` makes, for the same reason.
fn load_anchors() -> Vec<Vec<u8>> {
    #[cfg(target_os = "linux")]
    let backend = platform_linux::LinuxTrustAnchors;
    #[cfg(windows)]
    let backend = platform_windows::WindowsTrustAnchors;
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    let backend = platform_bsd::BsdTrustAnchors;

    backend.load_anchors().unwrap_or_default()
}

/// True if rustls will accept this certificate as a trust anchor.
fn rustls_accepts(der: &[u8]) -> bool {
    let mut store = RootCertStore::empty();
    store.add(CertificateDer::from(der.to_vec())).is_ok()
}

/// A corpus of zero certificates would make every assertion below vacuously
/// true, which is the failure mode this whole file exists to avoid. So the
/// corpus size is itself asserted.
#[test]
fn the_trust_store_yields_a_corpus_worth_testing_against() {
    let anchors = load_anchors();
    assert!(
        anchors.len() >= 10,
        "only {} trust anchors available — this environment cannot exercise the \
         corpus test, and a silent pass would misreport that as coverage",
        anchors.len()
    );
}

/// The headline assertion. See the module docs on why it is one-directional.
#[test]
fn every_anchor_rustls_accepts_is_one_this_parser_can_read() {
    let anchors = load_anchors();
    let mut compared = 0usize;
    let mut failures = Vec::new();

    for der in &anchors {
        if !rustls_accepts(der) {
            continue;
        }
        compared += 1;
        if let Err(err) = Certificate::parse(der) {
            failures.push(format!("  {err}"));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {compared} anchors that rustls accepts failed to parse:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        compared >= 10,
        "only {compared} anchors were comparable; the corpus is too small to mean anything"
    );
    println!("parsed {compared} real trust anchors that rustls also accepts");
}

/// Whatever parses must also be *coherent*, not merely non-erroring. A parser
/// can return `Ok` with fields pointing at the wrong bytes, and every one of
/// these would still be `Ok`.
#[test]
fn every_parsed_anchor_has_self_consistent_fields() {
    let anchors = load_anchors();
    let mut checked = 0usize;

    for der in &anchors {
        let Ok(cert) = Certificate::parse(der) else {
            continue;
        };
        checked += 1;

        assert!(
            !cert.serial().is_empty(),
            "a parsed certificate has an empty serial number"
        );
        assert!(
            cert.validity().not_before < cert.validity().not_after,
            "a root's validity window runs backwards: {:?}",
            cert.validity()
        );
        assert!(!cert.issuer().is_empty(), "empty issuer");
        assert!(!cert.subject().is_empty(), "empty subject");
        assert!(!cert.tbs_der().is_empty(), "empty tbsCertificate");
        assert!(!cert.signature().is_empty(), "empty signature");
        assert!(
            !cert.subject_public_key_info().key.is_empty(),
            "empty public key"
        );

        // Every borrowed field must actually be a borrow of the input. A
        // field pointing outside it would mean the parser copied or
        // synthesized bytes somewhere it claims not to.
        let range = der.as_ptr_range();
        for (name, field) in [
            ("tbs_der", cert.tbs_der()),
            ("issuer", cert.issuer()),
            ("subject", cert.subject()),
            ("signature", cert.signature()),
            ("spki", cert.subject_public_key_info().encoded),
        ] {
            let start = field.as_ptr();
            assert!(
                start >= range.start && start < range.end,
                "{name} does not point into the certificate it was parsed from"
            );
        }

        // Roots are self-issued by definition: a root's issuer is itself.
        assert!(
            cert.is_self_issued(),
            "a trust anchor is not self-issued — which is possible in a store \
             that carries intermediates, but worth knowing about"
        );
    }

    assert!(checked >= 10, "only {checked} anchors parsed");
    println!("checked field consistency across {checked} real trust anchors");
}

/// Real trust stores contain certificates from before the extensions they now
/// rely on existed. If none of them are v1, this corpus is less varied than
/// the parser needs, and the numbers are worth printing either way.
#[test]
fn the_corpus_actually_covers_a_range_of_certificate_shapes() {
    let anchors = load_anchors();
    let (mut v1, mut v3, mut rsa, mut ec, mut with_eku, mut with_path_len) = (0, 0, 0, 0, 0, 0);
    let mut unhandled_critical = 0;

    for der in &anchors {
        let Ok(cert) = Certificate::parse(der) else {
            continue;
        };
        use rusty_tls::handrolled::x509::{oid, Version};
        match cert.version() {
            Version::V1 => v1 += 1,
            Version::V3 => v3 += 1,
            Version::V2 => {}
        }
        match cert.subject_public_key_info().algorithm.oid {
            oid::RSA_ENCRYPTION => rsa += 1,
            oid::EC_PUBLIC_KEY => ec += 1,
            _ => {}
        }
        if cert.extensions().has_extended_key_usage() {
            with_eku += 1;
        }
        if cert
            .extensions()
            .basic_constraints()
            .and_then(|bc| bc.path_len_constraint)
            .is_some()
        {
            with_path_len += 1;
        }
        if !cert.extensions().unhandled_critical().is_empty() {
            unhandled_critical += 1;
        }
    }

    println!(
        "corpus shape: v1={v1} v3={v3} rsa={rsa} ec={ec} eku={with_eku} \
         path_len={with_path_len} unhandled_critical={unhandled_critical}"
    );

    // RSA roots are universal in every real store; their absence would mean
    // the corpus is not what this test thinks it is.
    assert!(
        rsa > 0,
        "no RSA roots in the corpus — is this a real trust store?"
    );
    assert!(
        v1 + v3 >= 10,
        "too few parsed certificates to be meaningful"
    );
}
