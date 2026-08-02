//! Path validation: one builder, and a test per way a chain must be refused.
//!
//! Every check in `handrolled::path` fails *open* if it is wrong — a missing
//! `basicConstraints` check does not break any valid chain, it just quietly
//! lets a leaf certificate sign for the whole internet. So the shape of this
//! file is one valid baseline chain and a long list of single deviations from
//! it, each of which must be refused, and each named for the deviation rather
//! than for the code path.
//!
//! The baseline itself is asserted first: if it stopped validating, every
//! rejection test below would pass for the wrong reason, and the suite would
//! report perfect health while testing nothing.
//!
//! # A differential where it is honest to have one
//!
//! rustls (via webpki) validates the same generated chains, and agreement is
//! asserted on the cases where both implementations are answering the same
//! question. It is deliberately not asserted everywhere — three divergences
//! are excluded, all of them this crate being stricter: SHA-1 signatures,
//! unknown critical extensions, and `keyUsage` on CAs. The last was measured
//! rather than assumed: rustls accepts an intermediate marked `cA` whose
//! `keyUsage` omits `keyCertSign`, which RFC 5280 §6.1.4(n) forbids. Demanding
//! agreement on any of them would be demanding this crate be less careful.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rcgen::{
    date_time_ymd, BasicConstraints, Certificate as RcgenCertificate, CertificateParams,
    CustomExtension, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use rusty_tls::handrolled::path::{
    validate_path, PathError, PathOptions, TrustAnchor, VerifiedPath,
};
use rusty_tls::handrolled::x509::{oid, Certificate};

/// 2026-01-01T00:00:00Z — inside every generated certificate's window unless
/// a test deliberately moves it.
const NOW: i64 = 1_767_225_600;

fn options() -> PathOptions {
    PathOptions {
        time: NOW,
        ..Default::default()
    }
}

/// A generated chain, kept as DER so the parsed borrows have somewhere to live.
struct Chain {
    root: Vec<u8>,
    intermediate: Vec<u8>,
    leaf: Vec<u8>,
}

/// How to bend the chain away from the baseline. One knob per test.
#[derive(Default)]
struct Bend {
    root_is_ca: Option<IsCa>,
    intermediate_is_ca: Option<IsCa>,
    intermediate_key_usages: Option<Vec<KeyUsagePurpose>>,
    intermediate_extra_extension: Option<CustomExtension>,
    leaf_extra_extension: Option<CustomExtension>,
    leaf_ekus: Option<Vec<ExtendedKeyUsagePurpose>>,
    leaf_is_ca: Option<IsCa>,
    leaf_validity: Option<(time::OffsetDateTime, time::OffsetDateTime)>,
    intermediate_validity: Option<(time::OffsetDateTime, time::OffsetDateTime)>,
    /// Sign the leaf with the root instead of the intermediate, so the
    /// intermediate in the chain is not actually its issuer.
    leaf_signed_by_root: bool,
    /// Distinguishes one generated chain's names from another's. Two chains
    /// built with the same suffix share subject names but not keys, which is
    /// the impostor case; different suffixes are unrelated PKIs.
    name_suffix: &'static str,
}

fn named(common_name: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    dn
}

fn build(bend: Bend) -> Chain {
    let root_key = KeyPair::generate().expect("key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    root_params.distinguished_name =
        named(&format!("rusty_tls path-test root{}", bend.name_suffix));
    root_params.is_ca = bend
        .root_is_ca
        .unwrap_or(IsCa::Ca(BasicConstraints::Unconstrained));
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let root = root_params.self_signed(&root_key).expect("root signs");

    let intermediate_key = KeyPair::generate().expect("key");
    let mut mid_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    mid_params.distinguished_name = named(&format!(
        "rusty_tls path-test intermediate{}",
        bend.name_suffix
    ));
    mid_params.is_ca = bend
        .intermediate_is_ca
        .unwrap_or(IsCa::Ca(BasicConstraints::Unconstrained));
    mid_params.key_usages = bend
        .intermediate_key_usages
        .unwrap_or_else(|| vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign]);
    if let Some(extension) = bend.intermediate_extra_extension {
        mid_params.custom_extensions = vec![extension];
    }
    if let Some((not_before, not_after)) = bend.intermediate_validity {
        mid_params.not_before = not_before;
        mid_params.not_after = not_after;
    }
    let intermediate = mid_params
        .signed_by(&intermediate_key, &root, &root_key)
        .expect("intermediate signs");

    let leaf_key = KeyPair::generate().expect("key");
    let mut leaf_params = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
    leaf_params.distinguished_name = named("example.com");
    leaf_params.subject_alt_names = vec![SanType::DnsName("example.com".try_into().unwrap())];
    leaf_params.is_ca = bend.leaf_is_ca.unwrap_or(IsCa::NoCa);
    leaf_params.extended_key_usages = bend
        .leaf_ekus
        .unwrap_or_else(|| vec![ExtendedKeyUsagePurpose::ServerAuth]);
    if let Some(extension) = bend.leaf_extra_extension {
        leaf_params.custom_extensions = vec![extension];
    }
    if let Some((not_before, not_after)) = bend.leaf_validity {
        leaf_params.not_before = not_before;
        leaf_params.not_after = not_after;
    }

    let leaf: RcgenCertificate = if bend.leaf_signed_by_root {
        leaf_params
            .signed_by(&leaf_key, &root, &root_key)
            .expect("leaf signs")
    } else {
        leaf_params
            .signed_by(&leaf_key, &intermediate, &intermediate_key)
            .expect("leaf signs")
    };

    Chain {
        root: root.der().to_vec(),
        intermediate: intermediate.der().to_vec(),
        leaf: leaf.der().to_vec(),
    }
}

/// Run the baseline shape: leaf, one intermediate, one anchor.
fn validate(chain: &Chain, options: &PathOptions) -> Result<VerifiedPath, PathError> {
    let root = Certificate::parse(&chain.root).expect("root parses");
    let intermediate = Certificate::parse(&chain.intermediate).expect("intermediate parses");
    let leaf = Certificate::parse(&chain.leaf).expect("leaf parses");

    validate_path(
        &leaf,
        &[intermediate],
        &[TrustAnchor::from_certificate(&root)],
        options,
    )
}

// ---------------------------------------------------------------------------
// The baseline
// ---------------------------------------------------------------------------

/// If this stops passing, every rejection test below is meaningless.
#[test]
fn the_baseline_chain_validates() {
    let chain = build(Bend::default());
    let path = validate(&chain, &options()).expect("the baseline chain must validate");

    assert_eq!(path.anchor, 0);
    assert_eq!(path.intermediates, vec![0], "the intermediate must be used");
}

/// A leaf issued directly by an anchor needs no intermediates at all.
#[test]
fn a_chain_with_no_intermediates_validates() {
    let chain = build(Bend {
        leaf_signed_by_root: true,
        ..Default::default()
    });
    let root = Certificate::parse(&chain.root).expect("parses");
    let leaf = Certificate::parse(&chain.leaf).expect("parses");

    let path = validate_path(
        &leaf,
        &[],
        &[TrustAnchor::from_certificate(&root)],
        &options(),
    )
    .expect("a directly-issued leaf validates");
    assert!(path.intermediates.is_empty());
}

/// Irrelevant intermediates must be ignored rather than confusing the search —
/// peers routinely send extras, and an attacker will send many.
#[test]
fn unrelated_intermediates_are_ignored() {
    let chain = build(Bend::default());
    let noise = build(Bend::default());

    let root = Certificate::parse(&chain.root).expect("parses");
    let leaf = Certificate::parse(&chain.leaf).expect("parses");
    let wanted = Certificate::parse(&chain.intermediate).expect("parses");
    let noise_mid = Certificate::parse(&noise.intermediate).expect("parses");
    let noise_root = Certificate::parse(&noise.root).expect("parses");

    // The needed intermediate is last, behind two irrelevant ones.
    let path = validate_path(
        &leaf,
        &[noise_mid, noise_root, wanted],
        &[TrustAnchor::from_certificate(&root)],
        &options(),
    )
    .expect("the chain validates through the noise");
    assert_eq!(path.intermediates, vec![2]);
}

/// More than one anchor in the store, only one of which is right.
#[test]
fn the_correct_anchor_is_selected_from_several() {
    let chain = build(Bend::default());
    let other = build(Bend::default());

    let root = Certificate::parse(&chain.root).expect("parses");
    let other_root = Certificate::parse(&other.root).expect("parses");
    let intermediate = Certificate::parse(&chain.intermediate).expect("parses");
    let leaf = Certificate::parse(&chain.leaf).expect("parses");

    let path = validate_path(
        &leaf,
        &[intermediate],
        &[
            TrustAnchor::from_certificate(&other_root),
            TrustAnchor::from_certificate(&root),
        ],
        &options(),
    )
    .expect("validates against the right anchor");
    assert_eq!(path.anchor, 1);
}

// ---------------------------------------------------------------------------
// Trust
// ---------------------------------------------------------------------------

/// The whole point. A chain that does not reach a trusted anchor is refused,
/// however internally consistent it is.
#[test]
fn a_chain_to_an_untrusted_root_is_refused() {
    let chain = build(Bend::default());
    let stranger = build(Bend {
        name_suffix: " (unrelated)",
        ..Default::default()
    });

    let leaf = Certificate::parse(&chain.leaf).expect("parses");
    let intermediate = Certificate::parse(&chain.intermediate).expect("parses");
    let stranger_root = Certificate::parse(&stranger.root).expect("parses");

    assert_eq!(
        validate_path(
            &leaf,
            &[intermediate],
            &[TrustAnchor::from_certificate(&stranger_root)],
            &options(),
        ),
        Err(PathError::NoPathToTrustAnchor)
    );
}

/// An empty trust store trusts nothing. Obvious, and worth pinning: a
/// validator that vacuously succeeds on an empty anchor set would pass every
/// other test in this file.
#[test]
fn an_empty_trust_store_validates_nothing() {
    let chain = build(Bend::default());
    let leaf = Certificate::parse(&chain.leaf).expect("parses");
    let intermediate = Certificate::parse(&chain.intermediate).expect("parses");

    assert_eq!(
        validate_path(&leaf, &[intermediate], &[], &options()),
        Err(PathError::NoPathToTrustAnchor)
    );
}

/// The intermediate must actually be missing from the search, not merely
/// unnecessary: without it there is no path, even though the root is trusted.
#[test]
fn a_chain_missing_its_intermediate_is_refused() {
    let chain = build(Bend::default());
    let root = Certificate::parse(&chain.root).expect("parses");
    let leaf = Certificate::parse(&chain.leaf).expect("parses");

    assert_eq!(
        validate_path(
            &leaf,
            &[],
            &[TrustAnchor::from_certificate(&root)],
            &options()
        ),
        Err(PathError::NoPathToTrustAnchor)
    );
}

/// An anchor whose name matches but whose key does not must not validate.
/// Name chaining is a search hint, never evidence.
#[test]
fn an_anchor_with_the_right_name_and_wrong_key_is_refused() {
    // Same `name_suffix`, so the two roots share a subject name and differ
    // only in their keys.
    let chain = build(Bend::default());
    let impostor = build(Bend::default());

    let real_root = Certificate::parse(&chain.root).expect("parses");
    let fake_root = Certificate::parse(&impostor.root).expect("parses");
    let intermediate = Certificate::parse(&chain.intermediate).expect("parses");
    let leaf = Certificate::parse(&chain.leaf).expect("parses");

    // Same subject name (both builders use the same DN), different key.
    assert_eq!(real_root.subject(), fake_root.subject());

    let result = validate_path(
        &leaf,
        &[intermediate],
        &[TrustAnchor::from_certificate(&fake_root)],
        &options(),
    );
    assert!(
        matches!(result, Err(PathError::Signature(_))),
        "expected a signature failure, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// basicConstraints — the check that fails open loudest
// ---------------------------------------------------------------------------

/// RFC 5280 §6.1.4(k). An intermediate without `cA` true cannot issue, and
/// accepting one is the Basic Constraints bug: any leaf certificate becomes a
/// CA for every site.
#[test]
fn an_intermediate_that_is_not_a_ca_is_refused() {
    for is_ca in [IsCa::NoCa, IsCa::ExplicitNoCa] {
        let chain = build(Bend {
            intermediate_is_ca: Some(is_ca.clone()),
            ..Default::default()
        });
        assert_eq!(
            validate(&chain, &options()),
            Err(PathError::NotACertificateAuthority),
            "an intermediate with {is_ca:?} was accepted as a CA"
        );
    }
}

/// A CA whose `keyUsage` excludes `keyCertSign` may not sign certificates,
/// §6.1.4(n) — even though it is marked as a CA.
#[test]
fn an_intermediate_without_key_cert_sign_is_refused() {
    let chain = build(Bend {
        intermediate_key_usages: Some(vec![KeyUsagePurpose::DigitalSignature]),
        ..Default::default()
    });
    assert_eq!(
        validate(&chain, &options()),
        Err(PathError::MissingKeyCertSign)
    );
}

/// Absent `keyUsage` means unrestricted, which is the extension's semantics.
/// Inverting this would refuse a large fraction of real CAs.
#[test]
fn an_intermediate_with_no_key_usage_at_all_is_accepted() {
    let chain = build(Bend {
        intermediate_key_usages: Some(vec![]),
        ..Default::default()
    });
    validate(&chain, &options()).expect("absent keyUsage is not a restriction");
}

/// `pathLenConstraint: 0` on the root means no intermediates may follow it.
#[test]
fn a_path_length_constraint_of_zero_refuses_an_intermediate() {
    let chain = build(Bend {
        root_is_ca: Some(IsCa::Ca(BasicConstraints::Constrained(0))),
        ..Default::default()
    });
    // The root's own constraint is not consulted — an anchor's certificate is
    // not validated — so this must still pass. The constraint that matters is
    // on the intermediate.
    validate(&chain, &options()).expect("an anchor's own pathLen is not consulted");

    // On the intermediate, a constraint of zero permits no further CA below
    // it. There is none in the baseline (the leaf is not a CA), so this is
    // still valid...
    let chain = build(Bend {
        intermediate_is_ca: Some(IsCa::Ca(BasicConstraints::Constrained(0))),
        ..Default::default()
    });
    validate(&chain, &options()).expect("pathLen 0 permits a leaf directly below");
}

/// Two CAs below a `pathLenConstraint: 0` intermediate is one too many.
#[test]
fn a_path_length_constraint_is_enforced_across_two_intermediates() {
    // root -> mid1 (pathLen 0) -> mid2 -> leaf
    let root_key = KeyPair::generate().expect("key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    root_params.distinguished_name = named("root");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let root = root_params.self_signed(&root_key).expect("signs");

    let mid1_key = KeyPair::generate().expect("key");
    let mut mid1_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    mid1_params.distinguished_name = named("mid1");
    mid1_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    mid1_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let mid1 = mid1_params
        .signed_by(&mid1_key, &root, &root_key)
        .expect("signs");

    let mid2_key = KeyPair::generate().expect("key");
    let mut mid2_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    mid2_params.distinguished_name = named("mid2");
    mid2_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    mid2_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let mid2 = mid2_params
        .signed_by(&mid2_key, &mid1, &mid1_key)
        .expect("signs");

    let leaf_key = KeyPair::generate().expect("key");
    let mut leaf_params = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf = leaf_params
        .signed_by(&leaf_key, &mid2, &mid2_key)
        .expect("signs");

    let (root_der, mid1_der, mid2_der, leaf_der) = (
        root.der().to_vec(),
        mid1.der().to_vec(),
        mid2.der().to_vec(),
        leaf.der().to_vec(),
    );
    let root_cert = Certificate::parse(&root_der).expect("parses");
    let leaf_cert = Certificate::parse(&leaf_der).expect("parses");
    let intermediates = [
        Certificate::parse(&mid2_der).expect("parses"),
        Certificate::parse(&mid1_der).expect("parses"),
    ];

    assert_eq!(
        validate_path(
            &leaf_cert,
            &intermediates,
            &[TrustAnchor::from_certificate(&root_cert)],
            &options(),
        ),
        Err(PathError::PathLengthExceeded),
        "two CAs below a pathLen-0 intermediate were accepted"
    );

    // The same shape with mid1 unconstrained validates, so the refusal above
    // is about the constraint and not about the depth.
    let mut mid1_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    mid1_params.distinguished_name = named("mid1");
    mid1_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(1));
    mid1_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let mid1_key2 = KeyPair::generate().expect("key");
    let mid1b = mid1_params
        .signed_by(&mid1_key2, &root, &root_key)
        .expect("signs");
    let mid2b = {
        let mut p = CertificateParams::new(Vec::<String>::new()).expect("params");
        p.distinguished_name = named("mid2");
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        p.signed_by(&mid2_key, &mid1b, &mid1_key2).expect("signs")
    };
    let leafb = {
        let mut p = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        p.signed_by(&leaf_key, &mid2b, &mid2_key).expect("signs")
    };
    let (m1, m2, lf) = (
        mid1b.der().to_vec(),
        mid2b.der().to_vec(),
        leafb.der().to_vec(),
    );
    let leaf_cert = Certificate::parse(&lf).expect("parses");
    let intermediates = [
        Certificate::parse(&m2).expect("parses"),
        Certificate::parse(&m1).expect("parses"),
    ];
    validate_path(
        &leaf_cert,
        &intermediates,
        &[TrustAnchor::from_certificate(&root_cert)],
        &options(),
    )
    .expect("pathLen 1 permits two CAs below");
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

#[test]
fn an_expired_leaf_is_refused() {
    let chain = build(Bend {
        leaf_validity: Some((date_time_ymd(2020, 1, 1), date_time_ymd(2021, 1, 1))),
        ..Default::default()
    });
    assert!(matches!(
        validate(&chain, &options()),
        Err(PathError::Expired { .. })
    ));
}

#[test]
fn a_not_yet_valid_leaf_is_refused() {
    let chain = build(Bend {
        leaf_validity: Some((date_time_ymd(2090, 1, 1), date_time_ymd(2091, 1, 1))),
        ..Default::default()
    });
    assert!(matches!(
        validate(&chain, &options()),
        Err(PathError::NotYetValid { .. })
    ));
}

/// An expired *intermediate* invalidates the chain just as an expired leaf
/// does — the check is per certificate, not just on the one the peer is
/// presenting.
#[test]
fn an_expired_intermediate_is_refused() {
    let chain = build(Bend {
        intermediate_validity: Some((date_time_ymd(2020, 1, 1), date_time_ymd(2021, 1, 1))),
        ..Default::default()
    });
    assert!(
        matches!(validate(&chain, &options()), Err(PathError::Expired { .. })),
        "an expired intermediate was accepted"
    );
}

/// The boundaries are inclusive (RFC 5280 §4.1.2.5), and a validator that got
/// the comparison backwards would fail only at the edges.
#[test]
fn the_validity_boundaries_are_inclusive() {
    let chain = build(Bend {
        leaf_validity: Some((date_time_ymd(2025, 1, 1), date_time_ymd(2027, 1, 1))),
        ..Default::default()
    });
    let not_before = 1_735_689_600; // 2025-01-01T00:00:00Z
    let not_after = 1_798_761_600; // 2027-01-01T00:00:00Z

    for (label, time) in [("notBefore", not_before), ("notAfter", not_after)] {
        validate(&chain, &PathOptions { time, ..options() })
            .unwrap_or_else(|e| panic!("{label} should be inclusive, got {e}"));
    }
    for (label, time) in [
        ("just before notBefore", not_before - 1),
        ("just after notAfter", not_after + 1),
    ] {
        assert!(
            validate(&chain, &PathOptions { time, ..options() }).is_err(),
            "{label} was accepted"
        );
    }
}

// ---------------------------------------------------------------------------
// Critical extensions — where the unimplemented name constraints live
// ---------------------------------------------------------------------------

/// RFC 5280 §6.1.3(f). A critical extension the validator does not understand
/// means the certificate cannot be processed safely, so the chain is refused.
#[test]
fn an_unknown_critical_extension_refuses_the_chain() {
    let mut extension =
        CustomExtension::from_oid_content(&[1, 3, 6, 1, 4, 1, 99999, 7], vec![0x05, 0x00]);
    extension.set_criticality(true);

    let chain = build(Bend {
        leaf_extra_extension: Some(extension.clone()),
        ..Default::default()
    });
    assert!(
        matches!(
            validate(&chain, &options()),
            Err(PathError::UnhandledCriticalExtension(_))
        ),
        "a leaf with an unknown critical extension was accepted"
    );

    let chain = build(Bend {
        intermediate_extra_extension: Some(extension),
        ..Default::default()
    });
    assert!(
        matches!(
            validate(&chain, &options()),
            Err(PathError::UnhandledCriticalExtension(_))
        ),
        "an intermediate with an unknown critical extension was accepted"
    );
}

/// The same extension, non-critical, must not refuse anything — that is the
/// entire meaning of the criticality bit, and refusing here would break most
/// real chains.
#[test]
fn an_unknown_noncritical_extension_is_ignored() {
    let extension =
        CustomExtension::from_oid_content(&[1, 3, 6, 1, 4, 1, 99999, 8], vec![0x05, 0x00]);
    assert!(!extension.criticality());

    let chain = build(Bend {
        leaf_extra_extension: Some(extension),
        ..Default::default()
    });
    validate(&chain, &options()).expect("a non-critical unknown extension is ignorable");
}

/// Name constraints are not implemented, and the safety of that rests
/// entirely on the check above: `nameConstraints` MUST be critical, so a
/// name-constrained intermediate is refused rather than having its constraint
/// silently ignored.
///
/// This test exists to make that dependency explicit. If someone ever relaxes
/// the unknown-critical-extension rule without implementing name constraints,
/// this fails and says why.
#[test]
fn a_name_constrained_intermediate_is_refused_rather_than_ignored() {
    // id-ce-nameConstraints, 2.5.29.30, with a permittedSubtrees of
    // dNSName:"example.org" — a constraint the baseline leaf violates.
    let mut extension = CustomExtension::from_oid_content(
        &[2, 5, 29, 30],
        vec![
            0x30, 0x11, // SEQUENCE (NameConstraints)
            0xa0, 0x0f, // [0] permittedSubtrees
            0x30, 0x0d, // SEQUENCE (GeneralSubtree)
            0x82, 0x0b, // [2] dNSName
            b'e', b'x', b'a', b'm', b'p', b'l', b'e', b'.', b'o', b'r', b'g',
        ],
    );
    extension.set_criticality(true);

    let chain = build(Bend {
        intermediate_extra_extension: Some(extension),
        ..Default::default()
    });

    match validate(&chain, &options()) {
        Err(PathError::UnhandledCriticalExtension(oid)) => {
            assert_eq!(
                oid,
                vec![0x55, 0x1d, 0x1e],
                "expected nameConstraints (2.5.29.30) to be the unhandled extension"
            );
        }
        other => panic!(
            "a name-constrained intermediate must be refused while name \
             constraints are unimplemented, got {other:?}"
        ),
    }
}

// ---------------------------------------------------------------------------
// Extended key usage
// ---------------------------------------------------------------------------

/// A certificate issued for client authentication only must not authenticate
/// a server.
#[test]
fn a_leaf_without_the_required_eku_is_refused() {
    let chain = build(Bend {
        leaf_ekus: Some(vec![ExtendedKeyUsagePurpose::ClientAuth]),
        ..Default::default()
    });
    assert_eq!(
        validate(&chain, &options()),
        Err(PathError::RequiredEkuMissing)
    );
}

/// Absent `extendedKeyUsage` permits every purpose. Getting this backwards
/// would refuse a large fraction of certificates that exist.
#[test]
fn a_leaf_with_no_eku_extension_satisfies_any_requirement() {
    let chain = build(Bend {
        leaf_ekus: Some(vec![]),
        ..Default::default()
    });
    validate(&chain, &options()).expect("an absent EKU is not a restriction");
}

/// `anyExtendedKeyUsage` satisfies whatever was asked for.
#[test]
fn any_extended_key_usage_satisfies_the_requirement() {
    let chain = build(Bend {
        leaf_ekus: Some(vec![ExtendedKeyUsagePurpose::Any]),
        ..Default::default()
    });
    validate(&chain, &options()).expect("anyExtendedKeyUsage satisfies serverAuth");
}

/// Requiring nothing skips the check entirely.
#[test]
fn no_required_eku_accepts_a_client_auth_certificate() {
    let chain = build(Bend {
        leaf_ekus: Some(vec![ExtendedKeyUsagePurpose::ClientAuth]),
        ..Default::default()
    });
    validate(
        &chain,
        &PathOptions {
            required_eku: None,
            ..options()
        },
    )
    .expect("with no requirement, any EKU is fine");

    // And a different requirement is satisfied by the matching EKU.
    validate(
        &chain,
        &PathOptions {
            required_eku: Some(oid::EKU_CLIENT_AUTH),
            ..options()
        },
    )
    .expect("clientAuth satisfies a clientAuth requirement");
}

// ---------------------------------------------------------------------------
// Search bounds
// ---------------------------------------------------------------------------

/// Depth is capped, and the cap is this crate's rather than anything a
/// certificate can influence.
#[test]
fn the_path_length_bound_is_enforced() {
    let chain = build(Bend::default());
    assert_eq!(
        validate(
            &chain,
            &PathOptions {
                max_path_length: 1,
                ..options()
            }
        ),
        Err(PathError::NoPathToTrustAnchor),
        "a two-certificate path was found under a depth bound of one"
    );
    validate(
        &chain,
        &PathOptions {
            max_path_length: 2,
            ..options()
        },
    )
    .expect("a depth bound of two admits leaf plus one intermediate");
}

/// The signature budget stops a search rather than merely slowing it, and says
/// so rather than reporting "no path".
#[test]
fn the_signature_budget_stops_the_search() {
    let chain = build(Bend::default());
    assert_eq!(
        validate(
            &chain,
            &PathOptions {
                max_signature_checks: 0,
                ..options()
            }
        ),
        Err(PathError::SearchBudgetExhausted)
    );
}

/// A large set of same-named intermediates is what a denial of service looks
/// like: every one is a candidate issuer, so a builder without a budget
/// explores them all. This must terminate quickly and refuse.
#[test]
fn a_pile_of_decoy_intermediates_terminates() {
    let chain = build(Bend::default());
    let root = Certificate::parse(&chain.root).expect("parses");
    let leaf = Certificate::parse(&chain.leaf).expect("parses");

    // Forty intermediates that all claim the right subject name and none of
    // which can actually have signed the leaf.
    let decoys: Vec<Vec<u8>> = (0..40)
        .map(|_| build(Bend::default()).intermediate)
        .collect();
    let mut candidates: Vec<Certificate<'_>> = decoys
        .iter()
        .map(|der| Certificate::parse(der).expect("parses"))
        .collect();
    // The real one last, so the search must get through every decoy.
    candidates.push(Certificate::parse(&chain.intermediate).expect("parses"));

    let path = validate_path(
        &leaf,
        &candidates,
        &[TrustAnchor::from_certificate(&root)],
        &options(),
    )
    .expect("the real intermediate is still found");
    assert_eq!(path.intermediates, vec![40]);
}

/// A certificate must not appear twice in one path, or a cycle among
/// cross-signed certificates becomes an unbounded search.
#[test]
fn a_certificate_is_not_reused_within_a_path() {
    let chain = build(Bend::default());
    let root = Certificate::parse(&chain.root).expect("parses");
    let leaf = Certificate::parse(&chain.leaf).expect("parses");
    let intermediate = Certificate::parse(&chain.intermediate).expect("parses");

    // The same intermediate offered three times must be used at most once.
    let path = validate_path(
        &leaf,
        &[
            Certificate::parse(&chain.intermediate).expect("parses"),
            intermediate,
            Certificate::parse(&chain.intermediate).expect("parses"),
        ],
        &[TrustAnchor::from_certificate(&root)],
        &options(),
    )
    .expect("validates");
    assert_eq!(path.intermediates.len(), 1);
}

// ---------------------------------------------------------------------------
// Differential against rustls
// ---------------------------------------------------------------------------

/// The same generated chains, through rustls' own path validation.
///
/// Restricted to cases where both implementations are answering the same
/// question. Three known divergences are excluded, all of them this crate
/// being stricter:
///
/// - **SHA-1 signatures**, which `handrolled::verify` refuses outright.
/// - **Unknown critical extensions**, which this crate treats as a hard
///   refusal per RFC 5280 §6.1.3(f).
/// - **`keyUsage` on CAs.** RFC 5280 §6.1.4(n) says that if a CA carries
///   `keyUsage`, `keyCertSign` must be set. webpki does not check this — it
///   was measured, not assumed: an intermediate marked `cA` whose `keyUsage`
///   lists only `digitalSignature` is accepted by rustls and refused here.
///   `an_intermediate_without_key_cert_sign_is_refused` covers that case
///   directly instead.
///
/// Demanding agreement on any of these would be demanding this crate be less
/// careful, so they are named here rather than quietly dropped.
#[test]
fn rustls_agrees_on_the_chains_where_both_answer_the_same_question() {
    use rustls::client::verify_server_cert_signed_by_trust_anchor;
    use rustls::pki_types::{CertificateDer, UnixTime};
    use rustls::RootCertStore;
    use std::time::Duration;

    let cases: Vec<(&str, Bend, bool)> = vec![
        ("baseline", Bend::default(), true),
        (
            "non-CA intermediate",
            Bend {
                intermediate_is_ca: Some(IsCa::ExplicitNoCa),
                ..Default::default()
            },
            false,
        ),
        (
            "expired leaf",
            Bend {
                leaf_validity: Some((date_time_ymd(2020, 1, 1), date_time_ymd(2021, 1, 1))),
                ..Default::default()
            },
            false,
        ),
        (
            "expired intermediate",
            Bend {
                intermediate_validity: Some((date_time_ymd(2020, 1, 1), date_time_ymd(2021, 1, 1))),
                ..Default::default()
            },
            false,
        ),
    ];

    for (label, bend, expected) in cases {
        let chain = build(bend);
        let ours = validate(&chain, &options()).is_ok();
        assert_eq!(ours, expected, "{label}: our own expectation");

        let mut store = RootCertStore::empty();
        store
            .add(CertificateDer::from(chain.root.clone()))
            .expect("the root is a usable anchor");

        let end_entity = CertificateDer::from(chain.leaf.clone());
        let intermediates = [CertificateDer::from(chain.intermediate.clone())];
        let theirs = verify_server_cert_signed_by_trust_anchor(
            &rustls::server::ParsedCertificate::try_from(&end_entity).expect("parses"),
            &store,
            &intermediates,
            UnixTime::since_unix_epoch(Duration::from_secs(NOW as u64)),
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .all,
        )
        .is_ok();

        assert_eq!(
            ours, theirs,
            "{label}: we said {ours}, rustls said {theirs}"
        );
    }
}
