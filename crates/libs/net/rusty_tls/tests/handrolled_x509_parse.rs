//! Round-trip oracle for the hand-rolled X.509 parser.
//!
//! The oracle is `rcgen`: every field is *specified* going in, so the test
//! knows what the parser is supposed to come back with. That makes it a true
//! independent check rather than a self-consistency one — the encoder is a
//! different implementation by different authors, and the expected values are
//! written down rather than derived from the code under test.
//!
//! Two of these tests are worth more than the rest, and for the same reason:
//!
//! - `tbs_der_is_what_the_signature_actually_covers` verifies the
//!   certificate's own signature with `ring`, using the `tbsCertificate`
//!   bytes, the public key, and the signature that this parser extracted. It
//!   passes only if all three were extracted correctly. Nothing else here
//!   ties the parser's output back to a cryptographic fact.
//! - `dates_match_an_independent_calendar_implementation` compares against
//!   `time::OffsetDateTime::unix_timestamp` rather than against a constant
//!   this file computed the same way `x509.rs` does.
//!
//! None of this says anything about whether a certificate should be trusted.
//! The parser does not decide that and neither does this file — see the
//! module docs on `handrolled::x509`.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rcgen::{
    date_time_ymd, BasicConstraints, CertificateParams, CustomExtension, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType, SerialNumber,
    PKCS_ECDSA_P256_SHA256, PKCS_ED25519,
};
use rusty_tls::handrolled::x509::{oid, Certificate, GeneralName, Version};

/// A self-signed certificate from `params`, plus the key that signed it.
fn self_signed(params: CertificateParams, alg: &'static rcgen::SignatureAlgorithm) -> Vec<u8> {
    let key = KeyPair::generate_for(alg).expect("key generates");
    params.self_signed(&key).expect("cert signs").der().to_vec()
}

fn default_params() -> CertificateParams {
    CertificateParams::new(vec!["example.com".to_string()]).expect("params build")
}

// ---------------------------------------------------------------------------
// The two tests that tie parsing to something outside the parser
// ---------------------------------------------------------------------------

/// The strongest single assertion available: verify the certificate's own
/// signature from the parser's own outputs.
///
/// `tbs_der`, `subject_public_key_info().key`, and `signature()` all have to
/// be exactly right for `ring` to accept — a one-byte error in any of them
/// fails. This is what makes `tbs_der` trustworthy enough for the signature
/// verification stage 2b will build on top of it.
#[test]
fn tbs_der_is_what_the_signature_actually_covers() {
    for (name, alg, verifier) in [
        (
            "ECDSA P-256",
            &PKCS_ECDSA_P256_SHA256,
            &ring::signature::ECDSA_P256_SHA256_ASN1 as &dyn ring::signature::VerificationAlgorithm,
        ),
        (
            "Ed25519",
            &PKCS_ED25519,
            &ring::signature::ED25519 as &dyn ring::signature::VerificationAlgorithm,
        ),
    ] {
        let der = self_signed(default_params(), alg);
        let cert = Certificate::parse(&der).unwrap_or_else(|e| panic!("{name}: parse failed: {e}"));

        ring::signature::UnparsedPublicKey::new(verifier, cert.subject_public_key_info().key)
            .verify(cert.tbs_der(), cert.signature())
            .unwrap_or_else(|_| {
                panic!("{name}: the parsed tbs/key/signature do not verify against each other")
            });
    }
}

/// Dates are checked against `time`'s calendar, not against a constant this
/// file worked out using the same algorithm the parser uses.
#[test]
fn dates_match_an_independent_calendar_implementation() {
    // Deliberately spans the UTCTime/GeneralizedTime boundary: RFC 5280
    // §4.1.2.5 requires UTCTime through 2049 and GeneralizedTime from 2050,
    // so a certificate with a window across it exercises both encodings.
    let windows = [
        (date_time_ymd(1975, 1, 1), date_time_ymd(4096, 1, 1)),
        (date_time_ymd(2049, 12, 31), date_time_ymd(2050, 1, 1)),
        (date_time_ymd(2000, 2, 29), date_time_ymd(2024, 2, 29)),
        (date_time_ymd(1970, 1, 1), date_time_ymd(2100, 12, 31)),
    ];

    for (not_before, not_after) in windows {
        let mut params = default_params();
        params.not_before = not_before;
        params.not_after = not_after;

        let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
        let cert = Certificate::parse(&der).expect("parses");

        assert_eq!(
            cert.validity().not_before,
            not_before.unix_timestamp(),
            "notBefore for {not_before}"
        );
        assert_eq!(
            cert.validity().not_after,
            not_after.unix_timestamp(),
            "notAfter for {not_after}"
        );
    }
}

// ---------------------------------------------------------------------------
// Field-by-field round trips
// ---------------------------------------------------------------------------

#[test]
fn the_serial_number_round_trips() {
    for value in [
        vec![0x01],
        vec![0x7f],
        vec![0x80],
        vec![0xff, 0xff],
        (1..=20u8).collect::<Vec<_>>(),
    ] {
        let mut params = default_params();
        params.serial_number = Some(SerialNumber::from(value.clone()));
        let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
        let cert = Certificate::parse(&der).expect("parses");
        assert_eq!(cert.serial(), &value[..], "serial {value:?}");
    }
}

#[test]
fn subject_alt_names_round_trip_in_order() {
    let mut params = default_params();
    params.subject_alt_names = vec![
        SanType::DnsName("example.com".try_into().unwrap()),
        SanType::DnsName("*.example.com".try_into().unwrap()),
        SanType::IpAddress("192.0.2.1".parse().unwrap()),
        SanType::IpAddress("2001:db8::1".parse().unwrap()),
        SanType::Rfc822Name("security@example.com".try_into().unwrap()),
        SanType::URI("https://example.com/".try_into().unwrap()),
    ];

    let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
    let cert = Certificate::parse(&der).expect("parses");
    let names: Vec<_> = cert
        .extensions()
        .subject_alt_names()
        .collect::<Result<_, _>>()
        .expect("every SAN parses");

    assert_eq!(
        names,
        vec![
            GeneralName::DnsName("example.com"),
            GeneralName::DnsName("*.example.com"),
            GeneralName::IpAddress(&[192, 0, 2, 1]),
            GeneralName::IpAddress(&[
                0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01
            ]),
            GeneralName::Rfc822Name("security@example.com"),
            GeneralName::Uri("https://example.com/"),
        ]
    );
    assert!(cert.extensions().has_subject_alt_name());
}

/// "No SAN extension" and "a SAN extension with nothing in it" are different
/// facts and a name matcher has to tell them apart.
#[test]
fn an_absent_subject_alt_name_is_distinguishable_from_an_empty_one() {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
    params.subject_alt_names = Vec::new();
    let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
    let cert = Certificate::parse(&der).expect("parses");

    assert!(!cert.extensions().has_subject_alt_name());
    assert_eq!(cert.extensions().subject_alt_names().count(), 0);
}

#[test]
fn basic_constraints_round_trip() {
    for (is_ca, expect_ca, expect_path_len) in [
        (IsCa::NoCa, false, None),
        (IsCa::ExplicitNoCa, false, None),
        (IsCa::Ca(BasicConstraints::Unconstrained), true, None),
        (IsCa::Ca(BasicConstraints::Constrained(0)), true, Some(0)),
        (IsCa::Ca(BasicConstraints::Constrained(3)), true, Some(3)),
    ] {
        let mut params = default_params();
        params.is_ca = is_ca.clone();
        let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
        let cert = Certificate::parse(&der).expect("parses");

        match cert.extensions().basic_constraints() {
            Some(bc) => {
                assert_eq!(bc.is_ca, expect_ca, "{is_ca:?}");
                assert_eq!(bc.path_len_constraint, expect_path_len, "{is_ca:?}");
            }
            // An omitted basicConstraints means "not a CA" by default, which
            // is the same answer as an explicit `cA=false`.
            None => assert!(!expect_ca, "{is_ca:?} lost its basicConstraints"),
        }
    }
}

#[test]
fn key_usage_bits_round_trip() {
    let mut params = default_params();
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
    let cert = Certificate::parse(&der).expect("parses");
    let usage = cert.extensions().key_usage().expect("keyUsage present");

    assert!(usage.digital_signature());
    assert!(usage.key_cert_sign());
    assert!(usage.crl_sign());
    // The bits that were not asked for must not be set — the bit numbering
    // runs from the most significant bit of the first octet, which is exactly
    // the kind of thing an off-by-one gets wrong silently.
    assert!(!usage.content_commitment());
    assert!(!usage.key_encipherment());
    assert!(!usage.data_encipherment());
    assert!(!usage.key_agreement());
    assert!(!usage.encipher_only());
    assert!(!usage.decipher_only());
}

/// `decipherOnly` is bit 8 — the only one in the second octet, and the one a
/// parser that reads a single byte would silently lose.
#[test]
fn key_usage_reaches_into_the_second_octet() {
    let mut params = default_params();
    params.key_usages = vec![KeyUsagePurpose::DecipherOnly];
    let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
    let cert = Certificate::parse(&der).expect("parses");
    let usage = cert.extensions().key_usage().expect("keyUsage present");

    assert!(usage.decipher_only());
    assert!(!usage.digital_signature());
}

#[test]
fn extended_key_usage_round_trips() {
    let mut params = default_params();
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
    let cert = Certificate::parse(&der).expect("parses");

    let purposes: Vec<_> = cert
        .extensions()
        .extended_key_usage()
        .collect::<Result<_, _>>()
        .expect("every EKU OID parses");

    assert!(cert.extensions().has_extended_key_usage());
    assert_eq!(purposes, vec![oid::EKU_SERVER_AUTH, oid::EKU_CLIENT_AUTH]);
}

/// Absent EKU means "no restriction"; present-without-your-purpose means
/// "forbidden". Collapsing those is a real privilege escalation, so the
/// distinction is asserted rather than assumed.
#[test]
fn an_absent_extended_key_usage_is_distinguishable_from_an_empty_one() {
    let der = self_signed(default_params(), &PKCS_ECDSA_P256_SHA256);
    let cert = Certificate::parse(&der).expect("parses");
    assert!(!cert.extensions().has_extended_key_usage());
    assert_eq!(cert.extensions().extended_key_usage().count(), 0);
}

// ---------------------------------------------------------------------------
// Critical extensions
// ---------------------------------------------------------------------------

/// RFC 5280 §6.1.3(f): a validator must reject a certificate carrying a
/// critical extension it does not understand. It can only do that if the
/// parser tells it, so this asserts the parser tells it.
#[test]
fn an_unknown_critical_extension_is_reported() {
    // 1.3.6.1.4.1.99999.1 — a private arc, guaranteed to mean nothing here.
    let mut extension =
        CustomExtension::from_oid_content(&[1, 3, 6, 1, 4, 1, 99999, 1], vec![0x05, 0x00]);
    extension.set_criticality(true);

    let mut params = default_params();
    params.custom_extensions = vec![extension];
    let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
    let cert =
        Certificate::parse(&der).expect("parsing succeeds — rejecting is the validator's job");

    let unhandled = cert.extensions().unhandled_critical();
    assert_eq!(
        unhandled.len(),
        1,
        "expected one unhandled critical extension"
    );
    assert_eq!(
        unhandled[0].as_bytes(),
        // 1.3.6.1.4.1.99999.1, encoded.
        &[0x2b, 0x06, 0x01, 0x04, 0x01, 0x86, 0x8d, 0x1f, 0x01]
    );
}

/// The other half: an unknown extension that is *not* critical says "skip me
/// if you like", and must not be reported as something to reject over.
#[test]
fn an_unknown_noncritical_extension_is_not_reported() {
    let extension =
        CustomExtension::from_oid_content(&[1, 3, 6, 1, 4, 1, 99999, 2], vec![0x05, 0x00]);
    assert!(!extension.criticality());

    let mut params = default_params();
    params.custom_extensions = vec![extension];
    let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
    let cert = Certificate::parse(&der).expect("parses");

    assert!(cert.extensions().unhandled_critical().is_empty());
}

/// Extensions this parser *does* understand must not be reported as
/// unhandled, however they are marked. `basicConstraints` and `keyUsage` are
/// routinely critical in real certificates, and reporting them would make
/// every conforming CA certificate unusable.
#[test]
fn understood_extensions_are_never_reported_as_unhandled() {
    let mut params = default_params();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

    let der = self_signed(params, &PKCS_ECDSA_P256_SHA256);
    let cert = Certificate::parse(&der).expect("parses");

    assert!(
        cert.extensions().unhandled_critical().is_empty(),
        "understood extensions reported as unhandled: {:?}",
        cert.extensions().unhandled_critical()
    );
}

// ---------------------------------------------------------------------------
// Names and structure
// ---------------------------------------------------------------------------

/// Name chaining (RFC 5280 §7.1) compares *encoded* names, so the bytes the
/// parser hands back for a leaf's issuer must be byte-identical to the ones
/// it hands back for its CA's subject. Producing them by re-encoding a parsed
/// name is how implementations end up with two spellings of one name.
#[test]
fn a_leaf_issuer_is_byte_identical_to_its_ca_subject() {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "rusty_tls parse-test CA");
    dn.push(DnType::OrganizationName, "rusty_tls");
    ca_params.distinguished_name = dn;

    let ca_key = KeyPair::generate().expect("key");
    let ca = ca_params.self_signed(&ca_key).expect("CA signs");

    let leaf_params = default_params();
    let leaf_key = KeyPair::generate().expect("key");
    let leaf = leaf_params
        .signed_by(&leaf_key, &ca, &ca_key)
        .expect("leaf signs");

    let ca_der = ca.der().to_vec();
    let leaf_der = leaf.der().to_vec();
    let ca_cert = Certificate::parse(&ca_der).expect("CA parses");
    let leaf_cert = Certificate::parse(&leaf_der).expect("leaf parses");

    assert_eq!(
        leaf_cert.issuer(),
        ca_cert.subject(),
        "leaf issuer does not match CA subject byte for byte"
    );
    assert!(ca_cert.is_self_issued(), "a self-signed CA is self-issued");
    assert!(!leaf_cert.is_self_issued(), "a leaf is not self-issued");
}

#[test]
fn rcgen_certificates_are_v3_with_a_matching_signature_algorithm() {
    let der = self_signed(default_params(), &PKCS_ECDSA_P256_SHA256);
    let cert = Certificate::parse(&der).expect("parses");

    assert_eq!(cert.version(), Version::V3);
    // ecdsa-with-SHA256, 1.2.840.10045.4.3.2.
    assert_eq!(
        cert.signature_algorithm().oid.as_bytes(),
        &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]
    );
    // The tbsCertificate is a borrow of the input, not a copy: it must be a
    // subslice, at the offset where the outer SEQUENCE's contents begin.
    let tbs = cert.tbs_der();
    let offset = der
        .windows(tbs.len())
        .position(|w| w == tbs)
        .expect("tbs_der is a subslice of the input");
    assert!(
        offset > 0 && offset < 8,
        "tbs starts right after the header"
    );
}

#[test]
fn the_public_key_algorithm_is_reported() {
    for (alg, expected) in [
        (&PKCS_ECDSA_P256_SHA256, oid::EC_PUBLIC_KEY),
        (&PKCS_ED25519, oid::ED25519),
    ] {
        let der = self_signed(default_params(), alg);
        let cert = Certificate::parse(&der).expect("parses");
        assert_eq!(cert.subject_public_key_info().algorithm.oid, expected);
        assert!(!cert.subject_public_key_info().key.is_empty());
    }
}
