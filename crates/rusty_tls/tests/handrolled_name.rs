//! Server name matching and name constraints.
//!
//! This is the stage that turns "a trusted CA issued this" into "this
//! certificate is for the server I asked for", so the tests are organised
//! around the two things that must never happen:
//!
//! - a certificate authenticating a name it was not issued for,
//! - a name-constrained CA issuing outside its subtree and getting away with
//!   it.
//!
//! Both fail open. A too-permissive wildcard breaks nothing that anyone would
//! notice; it just means one certificate covers more of the internet than its
//! CA intended.
//!
//! # The historical cases have their own tests
//!
//! Name matching is unusually well supplied with real, named attacks, and
//! each gets a test rather than being folded into a generic case: the null
//! prefix (CVE-2009-2408), the Common Name fallback, `*.com`-style
//! whole-TLD wildcards, and label-boundary confusion in constraint matching
//! (`notexample.com` versus `example.com`).

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rcgen::{
    BasicConstraints, CertificateParams, CustomExtension, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType,
};
use rusty_tls::handrolled::name::{verify_server_name, NameError, ServerName};
use rusty_tls::handrolled::path::{verify_peer_certificate, PathError, PathOptions, TrustAnchor};
use rusty_tls::handrolled::x509::Certificate;

const NOW: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z

fn options() -> PathOptions {
    PathOptions {
        time: NOW,
        ..Default::default()
    }
}

/// A self-signed certificate carrying exactly the SANs given.
fn cert_with_sans(sans: Vec<SanType>) -> Vec<u8> {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
    params.subject_alt_names = sans;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let key = KeyPair::generate().expect("key");
    params.self_signed(&key).expect("signs").der().to_vec()
}

fn dns(name: &str) -> SanType {
    SanType::DnsName(name.try_into().expect("a valid rcgen DNS name"))
}

/// Assert whether a certificate with `sans` authenticates `wanted`.
fn matches(sans: Vec<SanType>, wanted: &str) -> bool {
    let der = cert_with_sans(sans);
    let certificate = Certificate::parse(&der).expect("parses");
    verify_server_name(&certificate, &ServerName::Dns(wanted)).is_ok()
}

// ---------------------------------------------------------------------------
// Exact names
// ---------------------------------------------------------------------------

#[test]
fn an_exact_name_matches_and_a_different_one_does_not() {
    assert!(matches(vec![dns("example.com")], "example.com"));
    assert!(!matches(vec![dns("example.com")], "example.org"));
    assert!(!matches(vec![dns("example.com")], "www.example.com"));
    assert!(!matches(vec![dns("www.example.com")], "example.com"));
}

/// DNS is case-insensitive, and a matcher that is not would refuse valid
/// certificates rather than accept invalid ones — the safe direction, but
/// still wrong.
#[test]
fn matching_is_case_insensitive() {
    assert!(matches(vec![dns("EXAMPLE.com")], "example.com"));
    assert!(matches(vec![dns("example.com")], "ExAmPlE.CoM"));
}

/// Any one of several SANs is enough.
#[test]
fn any_of_several_names_matches() {
    let sans = vec![dns("a.example.com"), dns("b.example.com"), dns("c.test")];
    assert!(matches(sans.clone(), "b.example.com"));
    assert!(matches(sans.clone(), "c.test"));
    assert!(!matches(sans, "d.example.com"));
}

/// A trailing dot is the root label. Accepting it would make `example.com`
/// and `example.com.` two spellings of one name, so both sides are refused
/// rather than normalised.
#[test]
fn trailing_dots_are_refused_rather_than_normalised() {
    let der = cert_with_sans(vec![dns("example.com")]);
    let certificate = Certificate::parse(&der).expect("parses");
    assert_eq!(
        verify_server_name(&certificate, &ServerName::Dns("example.com.")),
        Err(NameError::MalformedReferenceName)
    );
}

/// A nonsense reference name is a caller error, reported as such rather than
/// as a plain "no match" — the question was malformed, not answered.
#[test]
fn a_malformed_reference_name_is_distinguished_from_a_mismatch() {
    let der = cert_with_sans(vec![dns("example.com")]);
    let certificate = Certificate::parse(&der).expect("parses");

    for bad in ["", ".", "..", ".example.com", "exa mple.com", "a..b"] {
        assert_eq!(
            verify_server_name(&certificate, &ServerName::Dns(bad)),
            Err(NameError::MalformedReferenceName),
            "{bad:?} was not reported as malformed"
        );
    }
    // A well-formed name that simply is not present reports the other error.
    assert_eq!(
        verify_server_name(&certificate, &ServerName::Dns("other.test")),
        Err(NameError::NoMatchingSubjectAltName)
    );
}

// ---------------------------------------------------------------------------
// Wildcards
// ---------------------------------------------------------------------------

#[test]
fn a_wildcard_covers_exactly_one_label() {
    let sans = vec![dns("*.example.com")];
    assert!(matches(sans.clone(), "www.example.com"));
    assert!(matches(sans.clone(), "anything.example.com"));
    // Not the bare domain...
    assert!(!matches(sans.clone(), "example.com"));
    // ...and not two levels down.
    assert!(!matches(sans, "a.b.example.com"));
}

/// RFC 6125 permits partial wildcards; the CA/Browser Forum forbids them and
/// no browser accepts them. Refused here too.
#[test]
fn partial_wildcards_are_refused() {
    assert!(!matches(vec![dns("www*.example.com")], "www1.example.com"));
    assert!(!matches(vec![dns("*w.example.com")], "ww.example.com"));
}

#[test]
fn a_wildcard_outside_the_leftmost_label_is_refused() {
    assert!(!matches(vec![dns("a.*.example.com")], "a.b.example.com"));
    assert!(!matches(vec![dns("example.*")], "example.com"));
}

/// A certificate for an entire top-level domain. A policy judgment rather
/// than an RFC rule, and a crude stand-in for a public suffix list this crate
/// does not carry — it stops `*.com` and does not pretend to stop `*.co.uk`.
#[test]
fn a_whole_tld_wildcard_is_refused() {
    assert!(!matches(vec![dns("*.com")], "example.com"));
    assert!(!matches(vec![dns("*.test")], "example.test"));
    // Two labels behind the wildcard is fine.
    assert!(matches(vec![dns("*.example.com")], "www.example.com"));
}

// ---------------------------------------------------------------------------
// The named historical attacks
// ---------------------------------------------------------------------------

/// CVE-2009-2408. `evil.example.com\0good.example.com` reads as
/// `good.example.com` to anything with C string semantics.
///
/// `handrolled::x509` deliberately preserves the NUL rather than trimming at
/// it or rejecting the certificate — that decision was made for this test.
/// The name is malformed, so it matches nothing, including the prefix an
/// attacker wants it to be read as.
#[test]
fn a_null_prefix_name_matches_nothing() {
    // rcgen validates its input, so the SAN is built by hand.
    let der = certificate_with_raw_dns_san(b"evil.example.com\0good.example.com");
    let certificate = Certificate::parse(&der).expect("the certificate itself is well-formed");

    for wanted in ["good.example.com", "evil.example.com"] {
        assert_eq!(
            verify_server_name(&certificate, &ServerName::Dns(wanted)),
            Err(NameError::NoMatchingSubjectAltName),
            "a null-prefix name authenticated {wanted}"
        );
    }
}

/// RFC 2818's Common Name fallback, obsoleted by RFC 6125 and removed from
/// browsers. A CN is free text no CA validates as a domain.
#[test]
fn the_common_name_is_never_used_as_a_server_name() {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "example.com");
    params.distinguished_name = dn;
    params.subject_alt_names = Vec::new();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let key = KeyPair::generate().expect("key");
    let der = params.self_signed(&key).expect("signs").der().to_vec();

    let certificate = Certificate::parse(&der).expect("parses");
    assert_eq!(
        verify_server_name(&certificate, &ServerName::Dns("example.com")),
        Err(NameError::NoMatchingSubjectAltName),
        "a certificate with example.com only in its CN authenticated it"
    );
}

/// A certificate with no `subjectAltName` at all matches nothing. The
/// intended outcome of the rule above, asserted separately because it is the
/// case someone would be tempted to "fix".
#[test]
fn a_certificate_with_no_subject_alt_name_matches_nothing() {
    let der = cert_with_sans(vec![]);
    let certificate = Certificate::parse(&der).expect("parses");
    assert_eq!(
        verify_server_name(&certificate, &ServerName::Dns("example.com")),
        Err(NameError::NoMatchingSubjectAltName)
    );
}

// ---------------------------------------------------------------------------
// IP addresses
// ---------------------------------------------------------------------------

#[test]
fn ip_addresses_match_by_octets() {
    let sans = vec![
        SanType::IpAddress("192.0.2.1".parse().unwrap()),
        SanType::IpAddress("2001:db8::1".parse().unwrap()),
    ];
    let der = cert_with_sans(sans);
    let certificate = Certificate::parse(&der).expect("parses");

    for (label, wanted, expected) in [
        ("the v4 address", "192.0.2.1", true),
        ("a different v4 address", "192.0.2.2", false),
        ("the v6 address", "2001:db8::1", true),
        ("a different v6 address", "2001:db8::2", false),
    ] {
        let address: std::net::IpAddr = wanted.parse().unwrap();
        assert_eq!(
            verify_server_name(&certificate, &ServerName::Ip(address)).is_ok(),
            expected,
            "{label}"
        );
    }
}

/// A certificate for the *string* "192.0.2.1" does not authenticate the
/// *address* 192.0.2.1, and the reverse. They are different SAN types and
/// conflating them is how an attacker gets a DNS certificate to cover an IP.
#[test]
fn a_dns_name_that_looks_like_an_ip_does_not_authenticate_that_ip() {
    let der = cert_with_sans(vec![dns("192.0.2.1")]);
    let certificate = Certificate::parse(&der).expect("parses");
    let address: std::net::IpAddr = "192.0.2.1".parse().unwrap();

    assert_eq!(
        verify_server_name(&certificate, &ServerName::Ip(address)),
        Err(NameError::NoMatchingSubjectAltName)
    );

    // ...and an iPAddress SAN does not authenticate the DNS name either.
    let der = cert_with_sans(vec![SanType::IpAddress(address)]);
    let certificate = Certificate::parse(&der).expect("parses");
    assert_eq!(
        verify_server_name(&certificate, &ServerName::Dns("192.0.2.1")),
        Err(NameError::NoMatchingSubjectAltName)
    );
}

/// A wildcard is a DNS construct and must not apply to addresses.
#[test]
fn a_wildcard_never_matches_an_ip_address() {
    let der = cert_with_sans(vec![dns("*.example.com")]);
    let certificate = Certificate::parse(&der).expect("parses");
    let address: std::net::IpAddr = "192.0.2.1".parse().unwrap();
    assert!(verify_server_name(&certificate, &ServerName::Ip(address)).is_err());
}

// ---------------------------------------------------------------------------
// Name constraints, end to end through a real chain
// ---------------------------------------------------------------------------

/// dNSName `permittedSubtrees`, DER-encoded.
fn permitted_dns(bases: &[&str]) -> CustomExtension {
    let mut subtrees = Vec::new();
    for base in bases {
        let mut general = vec![0x82, base.len() as u8];
        general.extend_from_slice(base.as_bytes());
        let mut subtree = vec![0x30, general.len() as u8];
        subtree.extend_from_slice(&general);
        subtrees.extend_from_slice(&subtree);
    }
    let mut permitted = vec![0xa0, subtrees.len() as u8];
    permitted.extend_from_slice(&subtrees);
    let mut body = vec![0x30, permitted.len() as u8];
    body.extend_from_slice(&permitted);

    let mut extension = CustomExtension::from_oid_content(&[2, 5, 29, 30], body);
    extension.set_criticality(true);
    extension
}

/// dNSName `excludedSubtrees`, DER-encoded.
fn excluded_dns(bases: &[&str]) -> CustomExtension {
    let mut subtrees = Vec::new();
    for base in bases {
        let mut general = vec![0x82, base.len() as u8];
        general.extend_from_slice(base.as_bytes());
        let mut subtree = vec![0x30, general.len() as u8];
        subtree.extend_from_slice(&general);
        subtrees.extend_from_slice(&subtree);
    }
    let mut excluded = vec![0xa1, subtrees.len() as u8];
    excluded.extend_from_slice(&subtrees);
    let mut body = vec![0x30, excluded.len() as u8];
    body.extend_from_slice(&excluded);

    let mut extension = CustomExtension::from_oid_content(&[2, 5, 29, 30], body);
    extension.set_criticality(true);
    extension
}

struct ConstrainedChain {
    root: Vec<u8>,
    intermediate: Vec<u8>,
    leaf: Vec<u8>,
}

fn constrained_chain(constraint: Option<CustomExtension>, leaf_names: &[&str]) -> ConstrainedChain {
    let root_key = KeyPair::generate().expect("key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "constraint-test root");
    root_params.distinguished_name = dn;
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let root = root_params.self_signed(&root_key).expect("signs");

    let mid_key = KeyPair::generate().expect("key");
    let mut mid_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "constraint-test intermediate");
    mid_params.distinguished_name = dn;
    mid_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    mid_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    if let Some(extension) = constraint {
        mid_params.custom_extensions = vec![extension];
    }
    let intermediate = mid_params
        .signed_by(&mid_key, &root, &root_key)
        .expect("signs");

    let leaf_key = KeyPair::generate().expect("key");
    let mut leaf_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    leaf_params.subject_alt_names = leaf_names.iter().map(|n| dns(n)).collect();
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf = leaf_params
        .signed_by(&leaf_key, &intermediate, &mid_key)
        .expect("signs");

    ConstrainedChain {
        root: root.der().to_vec(),
        intermediate: intermediate.der().to_vec(),
        leaf: leaf.der().to_vec(),
    }
}

fn verify(chain: &ConstrainedChain, name: &str) -> Result<(), PathError> {
    let root = Certificate::parse(&chain.root).expect("parses");
    let intermediate = Certificate::parse(&chain.intermediate).expect("parses");
    let leaf = Certificate::parse(&chain.leaf).expect("parses");

    verify_peer_certificate(
        &leaf,
        &[intermediate],
        &[TrustAnchor::from_certificate(&root)],
        &ServerName::Dns(name),
        &options(),
    )
    .map(|_| ())
}

/// The headline: a name-constrained chain now validates, where before stage
/// 2b-iii it was refused wholesale as an unhandled critical extension.
#[test]
fn a_chain_within_its_name_constraints_validates() {
    let chain = constrained_chain(Some(permitted_dns(&["example.com"])), &["www.example.com"]);
    verify(&chain, "www.example.com").expect("a chain inside its constraint must validate");
}

/// And the reason the check above matters: outside the subtree, refused.
#[test]
fn a_chain_outside_its_name_constraints_is_refused() {
    let chain = constrained_chain(Some(permitted_dns(&["example.com"])), &["www.evil.test"]);
    assert!(
        matches!(verify(&chain, "www.evil.test"), Err(PathError::Name(_))),
        "a constrained CA issued outside its subtree and was accepted"
    );
}

/// Label boundaries. `notexample.com` is not inside `example.com`, and a
/// suffix comparison that forgot the dot would say it is.
#[test]
fn name_constraints_respect_label_boundaries() {
    let chain = constrained_chain(Some(permitted_dns(&["example.com"])), &["notexample.com"]);
    assert!(
        matches!(verify(&chain, "notexample.com"), Err(PathError::Name(_))),
        "notexample.com was treated as inside example.com"
    );

    // The base itself is inside its own subtree.
    let chain = constrained_chain(Some(permitted_dns(&["example.com"])), &["example.com"]);
    verify(&chain, "example.com").expect("the base is inside its own subtree");
}

/// Every name in the certificate must be permitted, not just the one being
/// matched — otherwise a constrained CA could smuggle an extra name in.
#[test]
fn every_name_must_be_permitted_not_just_the_matched_one() {
    let chain = constrained_chain(
        Some(permitted_dns(&["example.com"])),
        &["www.example.com", "www.evil.test"],
    );
    assert!(
        matches!(verify(&chain, "www.example.com"), Err(PathError::Name(_))),
        "a certificate smuggled an unpermitted name past a constrained CA"
    );
}

#[test]
fn excluded_subtrees_are_refused() {
    let chain = constrained_chain(Some(excluded_dns(&["evil.test"])), &["www.evil.test"]);
    assert!(matches!(
        verify(&chain, "www.evil.test"),
        Err(PathError::Name(_))
    ));

    // Outside the exclusion, the same chain is fine — an exclusion with no
    // permitted list constrains only what it names.
    let chain = constrained_chain(Some(excluded_dns(&["evil.test"])), &["www.example.com"]);
    verify(&chain, "www.example.com").expect("names outside an exclusion are unaffected");
}

/// Several permitted subtrees: any one is enough.
#[test]
fn any_permitted_subtree_suffices() {
    let constraint = permitted_dns(&["example.com", "example.org"]);
    let chain = constrained_chain(Some(constraint), &["www.example.org"]);
    verify(&chain, "www.example.org").expect("the second subtree permits it");
}

/// A constraint type this implementation cannot evaluate must refuse the
/// chain rather than be skipped. A constraint that is parsed and ignored is
/// worse than one that was never recognised, because recognising the
/// extension is what removed the blanket refusal.
#[test]
fn an_unevaluable_constraint_type_refuses_the_chain() {
    // permittedSubtrees with a directoryName [4] base, which is not
    // implemented. An empty RDNSequence keeps the encoding minimal.
    let body = vec![
        0x30, 0x08, // SEQUENCE (NameConstraints)
        0xa0, 0x06, // [0] permittedSubtrees
        0x30, 0x04, // SEQUENCE (GeneralSubtree)
        0xa4, 0x02, // [4] directoryName, constructed
        0x30, 0x00, // SEQUENCE (an empty RDNSequence)
    ];
    let mut extension = CustomExtension::from_oid_content(&[2, 5, 29, 30], body);
    extension.set_criticality(true);

    let chain = constrained_chain(Some(extension), &["www.example.com"]);
    assert_eq!(
        verify(&chain, "www.example.com"),
        Err(PathError::Name(NameError::UnsupportedNameConstraint {
            tag: 4
        })),
        "a directoryName constraint was silently ignored"
    );
}

/// An unconstrained chain is unaffected by any of this.
#[test]
fn an_unconstrained_chain_still_validates() {
    let chain = constrained_chain(None, &["www.example.com"]);
    verify(&chain, "www.example.com").expect("no constraint, no restriction");
}

/// `verify_peer_certificate` must apply the name check as well as the path
/// check — the whole reason it exists is that the two are easy to separate
/// and disastrous to separate.
#[test]
fn the_combined_entry_point_checks_the_name_too() {
    let chain = constrained_chain(None, &["www.example.com"]);
    assert!(
        matches!(
            verify(&chain, "www.other.test"),
            Err(PathError::Name(NameError::NoMatchingSubjectAltName))
        ),
        "a valid chain for the wrong name was accepted"
    );
}

// ---------------------------------------------------------------------------
// Hand-built certificate, for names rcgen will not produce
// ---------------------------------------------------------------------------

/// rcgen validates its inputs, so a certificate with a deliberately invalid
/// `dNSName` has to be assembled here.
fn certificate_with_raw_dns_san(name: &[u8]) -> Vec<u8> {
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
    fn seq(parts: &[&[u8]]) -> Vec<u8> {
        tlv(0x30, &parts.concat())
    }

    let algorithm = seq(&[&tlv(
        0x06,
        &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02],
    )]);
    let name_field = seq(&[&tlv(
        0x31,
        &seq(&[&tlv(0x06, &[0x55, 0x04, 0x03]), &tlv(0x0c, b"test")]),
    )]);
    let validity = seq(&[&tlv(0x17, b"200101000000Z"), &tlv(0x18, b"20991231235959Z")]);
    let spki = {
        let alg = seq(&[
            &tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]),
            &tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]),
        ]);
        let mut bits = vec![0x00];
        bits.extend_from_slice(&[0x04; 65]);
        seq(&[&alg, &tlv(0x03, &bits)])
    };
    let san = {
        let names = seq(&[&tlv(0x82, name)]);
        seq(&[&tlv(0x06, &[0x55, 0x1d, 0x11]), &tlv(0x04, &names)])
    };
    let extensions = tlv(0xa3, &seq(&[&san]));

    let tbs = seq(&[
        &tlv(0xa0, &tlv(0x02, &[0x02])),
        &tlv(0x02, &[0x01]),
        &algorithm,
        &name_field,
        &validity,
        &name_field,
        &spki,
        &extensions,
    ]);
    seq(&[&tbs, &algorithm, &tlv(0x03, &[0x00, 0xde, 0xad])])
}
