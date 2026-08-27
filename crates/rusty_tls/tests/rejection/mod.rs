//! The rejection cases both engines must answer — one table, two drivers.
//!
//! # Why this exists
//!
//! `rusty_tls#25` asked for the hermetic rejection suite to pass *identically*
//! on both backends. What existed instead was the same three rejections written
//! twice: once against `rustls` in `handshake.rs`, once against the hand-rolled
//! client in `handrolled_client.rs`. Two suites that happen to agree is a
//! weaker claim than one suite run twice, and it degrades in a specific way —
//! a case added to one is not added to the other, and forgetting is silent.
//!
//! So this module owns the cases, and each engine owns only a driver that
//! translates a case into its own API. A new row runs on both engines by
//! construction rather than by someone remembering.
//!
//! # Why the expectation is per engine
//!
//! There are places where the hand-rolled engine is deliberately *stricter*
//! than `rustls` — `webpki` does not enforce RFC 5280 §6.1.4(n), for one. A
//! table with a single shared expectation would force those to be excluded or
//! papered over, and they are the most interesting rows there are.
//!
//! So [`Case`] carries an [`Outcome`] per engine plus a written reason, and
//! [`assert_table_is_coherent`] refuses a divergence that has no reason
//! attached. Today every row agrees; the structure is what stops the first
//! disagreement from being resolved by deleting the row.
//!
//! # The oracle problem
//!
//! `rustls` is an oracle for *agreement*, never a specification. These rows are
//! this crate's stated behaviour, with `rustls`' behaviour recorded beside it —
//! not "whatever `rustls` does". That is why both columns are written out
//! explicitly instead of one being derived from the other.

#![allow(dead_code)] // each driver uses a different subset

use rcgen::{
    BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

/// Which root the client is told to trust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// The CA that actually issued the leaf.
    TheIssuingCa,
    /// A well-formed CA that had nothing to do with this chain.
    AnUnrelatedCa,
}

/// The leaf's validity window.
///
/// Expiry is expressed in the **certificate**, not in the client's clock,
/// because only one of the two engines has an injectable clock. `rustls` here
/// runs on real system time; the hand-rolled path validator takes an explicit
/// instant. Encoding expiry in the certificate is the only form of the case
/// that means the same thing to both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    /// Spans [`EVALUATED_AT`].
    Current,
    /// Ended well before [`EVALUATED_AT`].
    Expired,
}

/// What an engine is expected to do with a case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The handshake completes.
    Accepted,
    /// The handshake fails.
    Rejected,
}

/// Why the hand-rolled engine refuses a case.
///
/// Only the hand-rolled side carries this, and the asymmetry is deliberate
/// rather than an oversight. Its `ClientError` is this crate's own type, so
/// naming the variant is a claim this repo can keep; `rustls`' failure arrives
/// as an opaque `io::Error` through `TlsStream`, and asserting on its text
/// would be pinning another project's wording.
///
/// It exists because the tests this table replaced asserted
/// `ClientError::Path(_)`, not merely "an error". Consolidating them must not
/// quietly weaken that — "it was refused" and "it was refused *for the right
/// reason*" are different claims, and a client that rejected a good chain for
/// an unrelated reason would satisfy the weaker one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// `ClientError::Path` — chain building, validity, or name matching.
    PathValidation,
}

/// One case, and what each engine should make of it.
pub struct Case {
    /// Appears in assertion messages; must be unique.
    pub name: &'static str,
    /// The DNS name the leaf is issued for.
    pub certificate_name: &'static str,
    /// The DNS name the client asks to verify against.
    pub requested_name: &'static str,
    pub anchor: Anchor,
    pub validity: Validity,
    pub rustls: Outcome,
    pub handrolled: Outcome,
    /// Required exactly when `handrolled` is [`Outcome::Rejected`].
    pub handrolled_reason: Option<Reason>,
    /// Required when `rustls != handrolled`; must be `None` when they agree.
    pub divergence: Option<&'static str>,
}

/// A window comfortably around [`EVALUATED_AT`], and short of the year 2050 so
/// the dates stay `UTCTime` — the same encoding the rest of the suite's
/// certificates use.
const CURRENT: (i64, i64) = (1_577_836_800, 2_366_841_600); // 2020-01-01 .. 2045-01-01
/// Long over by [`EVALUATED_AT`].
const EXPIRED: (i64, i64) = (946_684_800, 978_307_200); // 2000-01-01 .. 2001-01-01

/// The instant the hand-rolled driver validates at: inside [`CURRENT`], well
/// past [`EXPIRED`]. Fixed so a result never depends on how long the suite took
/// to run.
pub const EVALUATED_AT: i64 = 1_800_000_000; // 2027-01-15

/// The cases.
///
/// `accepts_a_good_chain` is not decoration. A rejection table with no
/// accepting row is passed by a driver that fails everything — including one
/// broken so badly it never reaches the certificate. #25 hit that class of
/// vacuous pass four separate times, so the control row is load-bearing.
pub const CASES: &[Case] = &[
    Case {
        name: "accepts_a_good_chain",
        certificate_name: "rejection.example",
        requested_name: "rejection.example",
        anchor: Anchor::TheIssuingCa,
        validity: Validity::Current,
        rustls: Outcome::Accepted,
        handrolled: Outcome::Accepted,
        handrolled_reason: None,
        divergence: None,
    },
    Case {
        name: "refuses_a_certificate_for_another_name",
        certificate_name: "somewhere.else.example",
        requested_name: "rejection.example",
        anchor: Anchor::TheIssuingCa,
        validity: Validity::Current,
        rustls: Outcome::Rejected,
        handrolled: Outcome::Rejected,
        handrolled_reason: Some(Reason::PathValidation),
        divergence: None,
    },
    Case {
        name: "refuses_an_expired_certificate",
        certificate_name: "rejection.example",
        requested_name: "rejection.example",
        anchor: Anchor::TheIssuingCa,
        validity: Validity::Expired,
        rustls: Outcome::Rejected,
        handrolled: Outcome::Rejected,
        handrolled_reason: Some(Reason::PathValidation),
        divergence: None,
    },
    Case {
        name: "refuses_a_chain_to_an_untrusted_root",
        certificate_name: "rejection.example",
        requested_name: "rejection.example",
        anchor: Anchor::AnUnrelatedCa,
        validity: Validity::Current,
        rustls: Outcome::Rejected,
        handrolled: Outcome::Rejected,
        handrolled_reason: Some(Reason::PathValidation),
        divergence: None,
    },
];

/// Invariants of the table itself, asserted by every driver.
///
/// A table is a claim about coverage, and these are the ways that claim can
/// quietly stop being true.
pub fn assert_table_is_coherent() {
    assert!(
        CASES
            .iter()
            .any(|c| c.rustls == Outcome::Accepted && c.handrolled == Outcome::Accepted),
        "the table has no accepting case, so a driver that fails everything passes it"
    );
    assert!(
        CASES.iter().any(|c| c.rustls == Outcome::Rejected),
        "the table has no rejecting case, which is the entire point of it"
    );

    for case in CASES {
        let agree = case.rustls == case.handrolled;
        assert_eq!(
            agree,
            case.divergence.is_none(),
            "{}: a divergence between the engines needs a written reason, and \
             agreement must not carry one",
            case.name,
        );
        assert_eq!(
            case.handrolled == Outcome::Rejected,
            case.handrolled_reason.is_some(),
            "{}: a hand-rolled rejection must name why, and an acceptance must not",
            case.name,
        );

        let duplicates = CASES.iter().filter(|c| c.name == case.name).count();
        assert_eq!(duplicates, 1, "{}: duplicated case name", case.name);
    }
}

/// Everything an engine needs to run one case.
pub struct Fixture {
    /// The chain the server presents, leaf first.
    pub chain: Vec<CertificateDer<'static>>,
    /// The leaf's private key, for the server to sign with.
    pub key: PrivateKeyDer<'static>,
    /// The same key as a PKCS#8 document, for a server that wants the bytes.
    pub key_pkcs8: Vec<u8>,
    /// The single root the client is configured to trust — the issuer's for
    /// [`Anchor::TheIssuingCa`], an unrelated CA's otherwise.
    pub trusted_root_der: Vec<u8>,
}

/// Build the certificates for a case.
///
/// Both engines get the identical chain and the identical trusted root; the
/// only thing that differs downstream is the API each is driven through. That
/// is the property the two-parallel-suites arrangement could not offer.
pub fn fixture(case: &Case) -> Fixture {
    let (issuer_root_der, issuer_cert, issuer_key) = ca("rusty_tls rejection-table CA");

    let window = match case.validity {
        Validity::Current => CURRENT,
        Validity::Expired => EXPIRED,
    };

    let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("leaf key");
    let mut leaf_params =
        CertificateParams::new(vec![case.certificate_name.to_string()]).expect("leaf params");
    dates(&mut leaf_params, window);
    let leaf = leaf_params
        .signed_by(&leaf_key, &issuer_cert, &issuer_key)
        .expect("leaf");

    let trusted_root_der = match case.anchor {
        Anchor::TheIssuingCa => issuer_root_der,
        // A second, entirely valid CA. The chain is well-formed and the leaf's
        // signature verifies; it simply does not lead here.
        Anchor::AnUnrelatedCa => ca("rusty_tls unrelated CA").0,
    };

    Fixture {
        chain: vec![
            CertificateDer::from(leaf.der().to_vec()),
            CertificateDer::from(issuer_cert.der().to_vec()),
        ],
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
        key_pkcs8: leaf_key.serialize_der(),
        trusted_root_der,
    }
}

/// A self-signed CA.
///
/// `keyCertSign` is set because the hand-rolled path validator enforces it —
/// RFC 5280 §4.2.1.3 requires it of a certificate that signs others, and this
/// engine checks. `rustls` is not troubled by its presence, so the stricter
/// shape is what both engines see.
fn ca(common_name: &str) -> (Vec<u8>, rcgen::Certificate, KeyPair) {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("ca key");
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String(common_name.to_string()),
    );
    dates(&mut params, CURRENT);
    let cert = params.self_signed(&key).expect("ca");
    (cert.der().to_vec(), cert, key)
}

/// An explicit validity window on every certificate.
///
/// `rcgen` defaults `not_after` to the year 4096, so a fixture relying on the
/// default cannot express "expired" — a mistake this suite has already made
/// once and which produced a green test for a certificate that was still
/// perfectly valid.
fn dates(params: &mut CertificateParams, (not_before, not_after): (i64, i64)) {
    params.not_before = OffsetDateTime::from_unix_timestamp(not_before).expect("not_before");
    params.not_after = OffsetDateTime::from_unix_timestamp(not_after).expect("not_after");
}
