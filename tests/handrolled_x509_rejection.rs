//! Everything the DER reader and certificate parser must refuse.
//!
//! ADR-0002 makes a rejection suite a hard gate for every stage, and it
//! matters more here than it did for the record layer. A record that fails to
//! authenticate fails loudly and immediately; a certificate that parses
//! "successfully" into slightly the wrong values fails silently, later, in
//! someone else's favor.
//!
//! Two kinds of test:
//!
//! - **DER-level**, driving [`rusty_tls::handrolled::der`] directly with
//!   hand-written byte sequences. Every non-canonical encoding DER forbids
//!   gets its own case, because "one value, one encoding" is the property the
//!   whole parser leans on.
//! - **Certificate-level**, assembling certificates from parts so a single
//!   structural rule can be broken in isolation. Mutating a real certificate
//!   would be easier and much less precise — it is hard to be sure which rule
//!   a mutated byte actually broke.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::der::{DerError, Reader, Tag};
use rusty_tls::handrolled::x509::{Certificate, Version, X509Error};

// ---------------------------------------------------------------------------
// A minimal DER writer, so tests can say exactly what they mean
// ---------------------------------------------------------------------------

/// Encode one TLV with a minimal (canonical) length.
fn tlv(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = contents.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let first = bytes.iter().position(|&b| b != 0).expect("len is non-zero");
        out.push(0x80 | (bytes.len() - first) as u8);
        out.extend_from_slice(&bytes[first..]);
    }
    out.extend_from_slice(contents);
    out
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &concat(parts))
}

/// `ecdsa-with-SHA256`, 1.2.840.10045.4.3.2 — the algorithm identifier is
/// never interpreted by the parser, only compared, so any valid one will do.
fn algorithm_identifier() -> Vec<u8> {
    seq(&[&tlv(
        0x06,
        &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02],
    )])
}

/// A different algorithm identifier, for the mismatch test: `ecdsa-with-SHA384`.
fn other_algorithm_identifier() -> Vec<u8> {
    seq(&[&tlv(
        0x06,
        &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x03],
    )])
}

/// An `RDNSequence` with a single `commonName`.
fn name(common_name: &str) -> Vec<u8> {
    let attribute = seq(&[
        &tlv(0x06, &[0x55, 0x04, 0x03]), // id-at-commonName, 2.5.4.3
        &tlv(0x0c, common_name.as_bytes()),
    ]);
    seq(&[&tlv(0x31, &attribute)])
}

fn validity() -> Vec<u8> {
    seq(&[&tlv(0x17, b"200101000000Z"), &tlv(0x18, b"20991231235959Z")])
}

/// A structurally valid `SubjectPublicKeyInfo`. The key bits are nonsense —
/// nothing in this module looks at them.
fn spki() -> Vec<u8> {
    let algorithm = seq(&[
        &tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]), // id-ecPublicKey
        &tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]), // prime256v1
    ]);
    let mut key_bits = vec![0x00];
    key_bits.extend_from_slice(&[0x04; 65]);
    seq(&[&algorithm, &tlv(0x03, &key_bits)])
}

fn extension(oid: &[u8], critical: bool, value: &[u8]) -> Vec<u8> {
    let mut parts = vec![tlv(0x06, oid)];
    if critical {
        parts.push(tlv(0x01, &[0xff]));
    }
    parts.push(tlv(0x04, value));
    let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
    seq(&refs)
}

/// How a certificate is assembled, with each structural rule reachable
/// independently.
struct CertBuilder {
    /// `None` omits the `[0]` version wrapper entirely, which is v1.
    version: Option<u64>,
    serial: Vec<u8>,
    tbs_algorithm: Vec<u8>,
    outer_algorithm: Vec<u8>,
    unique_id: bool,
    /// Encoded `Extension` values; `None` omits the `[3]` wrapper.
    extensions: Option<Vec<Vec<u8>>>,
}

impl Default for CertBuilder {
    fn default() -> Self {
        Self {
            version: Some(2), // v3
            serial: vec![0x01],
            tbs_algorithm: algorithm_identifier(),
            outer_algorithm: algorithm_identifier(),
            unique_id: false,
            extensions: None,
        }
    }
}

impl CertBuilder {
    fn build(&self) -> Vec<u8> {
        let mut tbs: Vec<Vec<u8>> = Vec::new();
        if let Some(version) = self.version {
            tbs.push(tlv(0xa0, &tlv(0x02, &[version as u8])));
        }
        tbs.push(tlv(0x02, &self.serial));
        tbs.push(self.tbs_algorithm.clone());
        tbs.push(name("issuer"));
        tbs.push(validity());
        tbs.push(name("subject"));
        tbs.push(spki());
        if self.unique_id {
            // issuerUniqueID [1] IMPLICIT BIT STRING
            tbs.push(tlv(0x81, &[0x00, 0xab]));
        }
        if let Some(extensions) = &self.extensions {
            let refs: Vec<&[u8]> = extensions.iter().map(|e| e.as_slice()).collect();
            tbs.push(tlv(0xa3, &seq(&refs)));
        }

        let tbs_refs: Vec<&[u8]> = tbs.iter().map(|p| p.as_slice()).collect();
        let signature = tlv(0x03, &[0x00, 0xde, 0xad, 0xbe, 0xef]);
        seq(&[&seq(&tbs_refs), &self.outer_algorithm, &signature])
    }
}

/// The builder's default output must parse, or every rejection test below is
/// passing for the wrong reason.
#[test]
fn the_baseline_certificate_parses() {
    let der = CertBuilder::default().build();
    let cert = Certificate::parse(&der).expect("the baseline must be well-formed");
    assert_eq!(cert.version(), Version::V3);
    assert_eq!(cert.serial(), &[0x01]);
    // UTCTime `200101000000Z` is 2020-01-01, not 2000: RFC 5280 §4.1.2.5.1 maps
    // a two-digit year below 50 to 20YY.
    assert_eq!(cert.validity().not_before, 1_577_836_800);
}

// ---------------------------------------------------------------------------
// DER: non-canonical encodings
// ---------------------------------------------------------------------------

#[test]
fn indefinite_length_is_refused() {
    let mut reader = Reader::new(&[0x30, 0x80, 0x00, 0x00]);
    assert_eq!(reader.read_any(), Err(DerError::IndefiniteLength));
}

#[test]
fn non_minimal_lengths_are_refused() {
    // Long form for a length the short form could carry.
    let mut reader = Reader::new(&[0x04, 0x81, 0x01, 0xaa]);
    assert_eq!(reader.read_any(), Err(DerError::NonMinimalLength));

    // Long form with a leading zero octet.
    let mut reader = Reader::new(&[0x04, 0x82, 0x00, 0x81]);
    assert_eq!(reader.read_any(), Err(DerError::NonMinimalLength));

    // The reserved 0xff length octet.
    let mut reader = Reader::new(&[0x04, 0xff, 0x01]);
    assert_eq!(reader.read_any(), Err(DerError::NonMinimalLength));

    // ...but a genuine long form is accepted.
    let mut contents = vec![0x04, 0x81, 0x80];
    contents.extend(std::iter::repeat_n(0xaa, 0x80));
    let mut reader = Reader::new(&contents);
    assert_eq!(
        reader.read_any().expect("valid long form").contents.len(),
        0x80
    );
}

#[test]
fn the_high_tag_number_form_is_refused() {
    let mut reader = Reader::new(&[0x1f, 0x81, 0x00, 0x00]);
    assert_eq!(reader.read_any(), Err(DerError::HighTagNumberForm));
}

#[test]
fn truncated_input_is_refused() {
    for bytes in [
        &[][..],
        &[0x30][..],
        &[0x30, 0x05][..],
        &[0x30, 0x05, 0x01, 0x02][..],
        &[0x04, 0x82, 0xff][..],
    ] {
        let mut reader = Reader::new(bytes);
        assert!(
            matches!(
                reader.read_any(),
                Err(DerError::UnexpectedEnd | DerError::NonMinimalLength)
            ),
            "{bytes:?} was not refused"
        );
    }
}

#[test]
fn trailing_data_is_refused() {
    let mut reader = Reader::new(&[0x05, 0x00, 0xff]);
    reader.read_any().expect("first value reads");
    assert_eq!(
        reader.finish(),
        Err(DerError::TrailingData { remaining: 1 })
    );
}

#[test]
fn non_minimal_and_negative_integers_are_refused() {
    // Leading zero that is not clearing a sign bit.
    let mut reader = Reader::new(&[0x02, 0x02, 0x00, 0x01]);
    assert_eq!(
        reader.read_unsigned_integer(),
        Err(DerError::NonMinimalInteger)
    );

    // Negative, where the field is unsigned.
    let mut reader = Reader::new(&[0x02, 0x01, 0xff]);
    assert_eq!(
        reader.read_unsigned_integer(),
        Err(DerError::NegativeInteger)
    );

    // No content octets at all.
    let mut reader = Reader::new(&[0x02, 0x00]);
    assert_eq!(reader.read_unsigned_integer(), Err(DerError::EmptyInteger));

    // A leading zero IS required when the next octet has its high bit set.
    let mut reader = Reader::new(&[0x02, 0x02, 0x00, 0x80]);
    assert_eq!(reader.read_unsigned_integer(), Ok(&[0x80][..]));

    // Zero is a single zero octet, and stays that way.
    let mut reader = Reader::new(&[0x02, 0x01, 0x00]);
    assert_eq!(reader.read_unsigned_integer(), Ok(&[0x00][..]));
}

#[test]
fn an_integer_too_large_for_u64_is_refused() {
    // Eight significant octets, with the sign octet DER requires in front.
    let mut reader = Reader::new(&[0x02, 0x09, 0x00, 0x81, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(reader.read_u64(), Ok(0x8102_0304_0506_0708));

    // Nine significant octets does not fit, however it is encoded.
    let mut reader = Reader::new(&[0x02, 0x09, 0x01, 1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(reader.read_u64(), Err(DerError::IntegerTooLarge));
}

#[test]
fn a_non_canonical_boolean_is_refused() {
    for value in [0x01u8, 0x02, 0x7f, 0xfe] {
        let encoded = [0x01, 0x01, value];
        let mut reader = Reader::new(&encoded);
        assert_eq!(
            reader.read_bool(),
            Err(DerError::NonCanonicalBoolean(value)),
            "BOOLEAN 0x{value:02x} was accepted"
        );
    }
    assert_eq!(Reader::new(&[0x01, 0x01, 0x00]).read_bool(), Ok(false));
    assert_eq!(Reader::new(&[0x01, 0x01, 0xff]).read_bool(), Ok(true));
}

#[test]
fn malformed_bit_strings_are_refused() {
    // Empty: no room even for the unused-bits octet.
    let mut reader = Reader::new(&[0x03, 0x00]);
    assert_eq!(
        reader.read_bit_string_octets(),
        Err(DerError::MalformedBitString)
    );

    // Claims unused bits where the caller requires octet alignment.
    let mut reader = Reader::new(&[0x03, 0x02, 0x01, 0xfe]);
    assert_eq!(
        reader.read_bit_string_octets(),
        Err(DerError::MalformedBitString)
    );

    // More than seven unused bits is impossible.
    let mut reader = Reader::new(&[0x03, 0x02, 0x08, 0x00]);
    assert_eq!(
        reader.read_bit_string_flags(),
        Err(DerError::MalformedBitString)
    );

    // Unused bits claimed with no content octets to spare them from.
    let mut reader = Reader::new(&[0x03, 0x01, 0x03]);
    assert_eq!(
        reader.read_bit_string_flags(),
        Err(DerError::MalformedBitString)
    );
}

#[test]
fn malformed_object_identifiers_are_refused() {
    for bytes in [
        &[0x06, 0x00][..],                   // empty
        &[0x06, 0x01, 0x80][..],             // unterminated subidentifier
        &[0x06, 0x02, 0x80, 0x01][..],       // non-minimal leading 0x80
        &[0x06, 0x03, 0x55, 0x80, 0x01][..], // non-minimal in a later arc
        &[0x06, 0x02, 0x55, 0x81][..],       // trailing continuation bit
    ] {
        let mut reader = Reader::new(bytes);
        assert_eq!(
            reader.read_oid(),
            Err(DerError::MalformedOid),
            "{bytes:?} was accepted"
        );
    }

    // 2.5.29.19 still reads.
    let mut reader = Reader::new(&[0x06, 0x03, 0x55, 0x1d, 0x13]);
    assert_eq!(
        reader.read_oid().expect("valid OID").as_bytes(),
        &[0x55, 0x1d, 0x13]
    );
}

#[test]
fn a_wrong_tag_does_not_consume_the_value() {
    let input = [0x02, 0x01, 0x07];
    let mut reader = Reader::new(&input);

    assert_eq!(
        reader.read(Tag::SEQUENCE),
        Err(DerError::UnexpectedTag {
            expected: Tag::SEQUENCE,
            found: 0x02,
        })
    );
    // The cursor did not move, so an OPTIONAL field that is absent can be
    // followed by a successful read of what is actually there.
    assert_eq!(
        reader.read(Tag::INTEGER).expect("still readable").contents,
        &[0x07]
    );
}

// ---------------------------------------------------------------------------
// Certificate structure
// ---------------------------------------------------------------------------

#[test]
fn trailing_bytes_after_a_certificate_are_refused() {
    let mut der = CertBuilder::default().build();
    der.push(0x00);
    assert!(
        matches!(
            Certificate::parse(&der),
            Err(X509Error::Der(DerError::TrailingData { .. }))
        ),
        "a byte appended to a certificate was ignored"
    );

    // A whole second certificate is the case that actually gets attempted.
    let one = CertBuilder::default().build();
    let two = [one.clone(), one].concat();
    assert!(matches!(
        Certificate::parse(&two),
        Err(X509Error::Der(DerError::TrailingData { .. }))
    ));
}

#[test]
fn a_truncated_certificate_is_refused() {
    let der = CertBuilder::default().build();
    for cut in 1..der.len() {
        assert!(
            Certificate::parse(&der[..cut]).is_err(),
            "a certificate truncated to {cut} bytes parsed"
        );
    }
}

/// DER omits `DEFAULT` values, so `version` explicitly encoded as v1 is a
/// second spelling of "v1" — exactly the ambiguity DER exists to remove.
#[test]
fn an_explicitly_encoded_default_version_is_refused() {
    let der = CertBuilder {
        version: Some(0),
        ..Default::default()
    }
    .build();
    assert_eq!(
        Certificate::parse(&der).err(),
        Some(X509Error::ExplicitDefaultVersion)
    );
}

#[test]
fn an_unknown_version_is_refused() {
    for version in [3u64, 4, 127] {
        let der = CertBuilder {
            version: Some(version),
            ..Default::default()
        }
        .build();
        assert_eq!(
            Certificate::parse(&der).err(),
            Some(X509Error::UnsupportedVersion(version))
        );
    }
}

/// RFC 5280 §4.1.2.9: extensions are a v3 feature. A v1 or v2 certificate
/// carrying them is claiming constraints in a format its own version says it
/// cannot express.
#[test]
fn extensions_outside_a_v3_certificate_are_refused() {
    let basic_constraints = extension(&[0x55, 0x1d, 0x13], true, &seq(&[&tlv(0x01, &[0xff])]));

    for (version, expected) in [(None, Version::V1), (Some(1), Version::V2)] {
        let der = CertBuilder {
            version,
            extensions: Some(vec![basic_constraints.clone()]),
            ..Default::default()
        }
        .build();
        assert_eq!(
            Certificate::parse(&der).err(),
            Some(X509Error::ExtensionsBeforeV3(expected))
        );
    }

    // v3 with the same extension is fine, so the rule above is about the
    // version and not about the extension.
    let der = CertBuilder {
        version: Some(2),
        extensions: Some(vec![basic_constraints]),
        ..Default::default()
    }
    .build();
    let cert = Certificate::parse(&der).expect("v3 accepts extensions");
    assert!(
        cert.extensions()
            .basic_constraints()
            .expect("present")
            .is_ca
    );
}

#[test]
fn a_unique_identifier_in_a_v1_certificate_is_refused() {
    let der = CertBuilder {
        version: None,
        unique_id: true,
        ..Default::default()
    }
    .build();
    assert_eq!(
        Certificate::parse(&der).err(),
        Some(X509Error::UniqueIdentifierInV1)
    );

    // v2 and v3 may carry one.
    for version in [1u64, 2] {
        let der = CertBuilder {
            version: Some(version),
            unique_id: true,
            ..Default::default()
        }
        .build();
        assert!(
            Certificate::parse(&der).is_ok(),
            "v{} rejected a unique identifier",
            version + 1
        );
    }
}

#[test]
fn an_empty_extensions_sequence_is_refused() {
    let der = CertBuilder {
        extensions: Some(Vec::new()),
        ..Default::default()
    }
    .build();
    assert_eq!(
        Certificate::parse(&der).err(),
        Some(X509Error::EmptyExtensions)
    );
}

/// RFC 5280 §4.2: one instance of each extension. Two `basicConstraints`
/// disagreeing about `cA` is a certificate that says different things to
/// implementations that take the first versus the last.
#[test]
fn a_duplicated_extension_is_refused() {
    let ca_true = extension(&[0x55, 0x1d, 0x13], true, &seq(&[&tlv(0x01, &[0xff])]));
    let ca_false = extension(&[0x55, 0x1d, 0x13], true, &seq(&[]));

    let der = CertBuilder {
        extensions: Some(vec![ca_true.clone(), ca_false]),
        ..Default::default()
    }
    .build();
    assert_eq!(
        Certificate::parse(&der).err(),
        Some(X509Error::DuplicateExtension)
    );

    // Duplicates of an extension the parser does not understand are refused
    // too — "I ignore it" is not a reason to accept a contradiction.
    let unknown = extension(&[0x2b, 0x06, 0x01, 0x04, 0x01, 0x01], false, &[0x05, 0x00]);
    let der = CertBuilder {
        extensions: Some(vec![unknown.clone(), unknown]),
        ..Default::default()
    }
    .build();
    assert_eq!(
        Certificate::parse(&der).err(),
        Some(X509Error::DuplicateExtension)
    );
}

/// RFC 5280 §4.1.1.2: the algorithm is stated twice and the two MUST agree.
/// A certificate where they differ is asking a signed question and an
/// unsigned one, and an implementation that reads the wrong copy is being
/// steered by the unsigned one.
#[test]
fn a_signature_algorithm_mismatch_is_refused() {
    let der = CertBuilder {
        outer_algorithm: other_algorithm_identifier(),
        ..Default::default()
    }
    .build();
    assert_eq!(
        Certificate::parse(&der).err(),
        Some(X509Error::SignatureAlgorithmMismatch)
    );
}

#[test]
fn a_negative_serial_number_is_refused() {
    let der = CertBuilder {
        serial: vec![0xff],
        ..Default::default()
    }
    .build();
    assert_eq!(
        Certificate::parse(&der).err(),
        Some(X509Error::Der(DerError::NegativeInteger))
    );
}

/// RFC 5280 §4.2.1.9: `pathLenConstraint` "MUST NOT appear" unless `cA` is
/// true. A non-CA carrying one is constraining a chain it can never be in.
#[test]
fn a_path_length_on_a_non_ca_is_refused() {
    let bad = extension(
        &[0x55, 0x1d, 0x13],
        true,
        &seq(&[&tlv(0x02, &[0x03])]), // pathLenConstraint with no cA
    );
    let der = CertBuilder {
        extensions: Some(vec![bad]),
        ..Default::default()
    }
    .build();
    assert_eq!(
        Certificate::parse(&der).err(),
        Some(X509Error::MalformedBasicConstraints)
    );
}

#[test]
fn an_over_long_key_usage_is_refused() {
    let bad = extension(
        &[0x55, 0x1d, 0x0f],
        true,
        &tlv(0x03, &[0x00, 0xff, 0xff, 0xff]), // three octets, nine bits defined
    );
    let der = CertBuilder {
        extensions: Some(vec![bad]),
        ..Default::default()
    }
    .build();
    assert_eq!(
        Certificate::parse(&der).err(),
        Some(X509Error::MalformedKeyUsage)
    );
}

// ---------------------------------------------------------------------------
// Times
// ---------------------------------------------------------------------------

/// RFC 5280 §4.1.2.5 pins the time formats tightly: `Z`, seconds present, no
/// fractional part, and no local-offset forms. Everything else is a second
/// way to write a moment, which is how a certificate ends up expired for one
/// implementation and valid for another.
#[test]
fn out_of_profile_times_are_refused() {
    let cases: &[(&str, u8, &[u8])] = &[
        ("UTCTime without seconds", 0x17, b"2001010000Z"),
        ("UTCTime with an offset", 0x17, b"200101000000+0100"),
        ("UTCTime with no Z", 0x17, b"200101000000"),
        ("UTCTime with a bad month", 0x17, b"200001000000Z"),
        ("UTCTime with month 13", 0x17, b"201301000000Z"),
        ("UTCTime with day 32", 0x17, b"200132000000Z"),
        ("UTCTime with hour 24", 0x17, b"200101240000Z"),
        ("UTCTime with minute 60", 0x17, b"200101006000Z"),
        ("UTCTime with a non-digit", 0x17, b"20010100000ZZ"),
        ("GeneralizedTime, 2-digit year", 0x18, b"200101000000Z"),
        ("GeneralizedTime, fractional", 0x18, b"20000101000000.5Z"),
        ("GeneralizedTime with no Z", 0x18, b"20000101000000"),
        ("Feb 30", 0x17, b"200230000000Z"),
        ("Feb 29 in a non-leap year", 0x17, b"010229000000Z"),
    ];

    for (label, tag, bytes) in cases {
        let bad_validity = seq(&[&tlv(*tag, bytes), &tlv(0x18, b"20991231235959Z")]);
        let der = certificate_with_validity(&bad_validity);
        assert_eq!(
            Certificate::parse(&der).err(),
            Some(X509Error::MalformedTime),
            "{label} was accepted"
        );
    }
}

/// The other side of the rule above: the forms that *are* in profile parse,
/// including the leap day the case above rejects in a non-leap year.
#[test]
fn in_profile_times_are_accepted() {
    for (label, tag, bytes, expected) in [
        ("epoch", 0x17u8, &b"700101000000Z"[..], 0i64),
        ("2000 leap day", 0x17, b"000229000000Z", 951_782_400),
        ("last UTCTime year", 0x17, b"491231235959Z", 2_524_607_999),
        (
            "first GeneralizedTime year",
            0x18,
            b"20500101000000Z",
            2_524_608_000,
        ),
    ] {
        let good = seq(&[&tlv(tag, bytes), &tlv(0x18, b"20991231235959Z")]);
        let der = certificate_with_validity(&good);
        let cert = Certificate::parse(&der).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(cert.validity().not_before, expected, "{label}");
    }
}

/// The builder cannot vary `validity`, so this assembles one directly.
fn certificate_with_validity(validity: &[u8]) -> Vec<u8> {
    let tbs = seq(&[
        &tlv(0xa0, &tlv(0x02, &[0x02])),
        &tlv(0x02, &[0x01]),
        &algorithm_identifier(),
        &name("issuer"),
        validity,
        &name("subject"),
        &spki(),
    ]);
    seq(&[
        &tbs,
        &algorithm_identifier(),
        &tlv(0x03, &[0x00, 0xde, 0xad]),
    ])
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// A NUL inside a `dNSName` is the null-prefix attack (CVE-2009-2408). The
/// parser deliberately preserves it rather than dropping or rejecting it, so
/// this test exists to pin that behavior down: the name matcher that comes
/// next has to see the NUL in order to defend against it.
#[test]
fn an_embedded_nul_in_a_dns_name_survives_parsing() {
    let san = extension(
        &[0x55, 0x1d, 0x11],
        false,
        &seq(&[&tlv(0x82, b"evil.example.com\0good.example.com")]),
    );
    let der = CertBuilder {
        extensions: Some(vec![san]),
        ..Default::default()
    }
    .build();

    let cert = Certificate::parse(&der).expect("parses");
    let names: Vec<_> = cert
        .extensions()
        .subject_alt_names()
        .collect::<Result<Vec<_>, _>>()
        .expect("the name parses");

    assert_eq!(names.len(), 1);
    match names[0] {
        rusty_tls::handrolled::x509::GeneralName::DnsName(name) => {
            assert_eq!(name, "evil.example.com\0good.example.com");
            assert!(name.contains('\0'), "the NUL must not be dropped");
        }
        other => panic!("expected a dNSName, got {other:?}"),
    }
}

#[test]
fn a_non_ascii_ia5_name_is_refused() {
    let san = extension(
        &[0x55, 0x1d, 0x11],
        false,
        &seq(&[&tlv(0x82, &[b'e', b'x', 0xc3, 0xa9])]),
    );
    let der = CertBuilder {
        extensions: Some(vec![san]),
        ..Default::default()
    }
    .build();

    let cert = Certificate::parse(&der).expect("the certificate itself is well-formed");
    let first = cert
        .extensions()
        .subject_alt_names()
        .next()
        .expect("one name");
    assert_eq!(first, Err(X509Error::NonAsciiName));
}

#[test]
fn a_misshapen_ip_address_is_refused() {
    for len in [0usize, 1, 3, 5, 8, 15, 17] {
        let san = extension(
            &[0x55, 0x1d, 0x11],
            false,
            &seq(&[&tlv(0x87, &vec![0u8; len])]),
        );
        let der = CertBuilder {
            extensions: Some(vec![san]),
            ..Default::default()
        }
        .build();

        let cert = Certificate::parse(&der).expect("parses");
        let first = cert
            .extensions()
            .subject_alt_names()
            .next()
            .expect("one name");
        assert_eq!(
            first,
            Err(X509Error::MalformedIpAddress { len }),
            "a {len}-octet iPAddress was accepted"
        );
    }
}

/// One unparseable name must not make the others unreachable — a caller
/// looking for a DNS name should not be blocked by a malformed IP address.
#[test]
fn one_bad_name_does_not_hide_the_others() {
    let san = extension(
        &[0x55, 0x1d, 0x11],
        false,
        &seq(&[
            &tlv(0x82, b"good.example.com"),
            &tlv(0x87, &[0u8; 3]),
            &tlv(0x82, b"also-good.example.com"),
        ]),
    );
    let der = CertBuilder {
        extensions: Some(vec![san]),
        ..Default::default()
    }
    .build();

    let cert = Certificate::parse(&der).expect("parses");
    let results: Vec<_> = cert.extensions().subject_alt_names().collect();
    assert_eq!(results.len(), 3);
    assert!(results[0].is_ok());
    assert!(results[1].is_err());
    assert!(results[2].is_ok());
}
