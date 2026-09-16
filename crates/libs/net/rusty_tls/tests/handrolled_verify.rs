//! Signature verification, against real certificates and generated chains.
//!
//! Stage 2b-i is the first hand-rolled code whose answer has a security
//! consequence, so the tests are organised around the two ways it can be
//! wrong, which are not equally bad:
//!
//! - **Refusing a good signature** is a bug. Annoying, visible, and it fails
//!   closed.
//! - **Accepting a bad one** is the failure this whole issue exists to worry
//!   about. It fails open, silently, in an attacker's favour.
//!
//! So every positive test here has a negative twin, and the negative side is
//! where the volume is: the corpus test does not merely check that 124 real
//! roots self-verify, it checks that all 124 *stop* verifying when a single
//! bit anywhere in the signed material is flipped.
//!
//! # On self-signed roots as an oracle
//!
//! A root verifying against its own key says nothing about trust — anyone can
//! generate a self-signed certificate. It is used here purely as a
//! correctness check on this code: 124 certificates signed by real CAs, with
//! real RSA and ECDSA keys at three hash sizes, all of which must verify.
//! That is coverage no generated certificate can provide, because `rcgen`
//! (via `ring`) cannot even produce an RSA key.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use platform::security::TrustAnchors;
use rcgen::{
    BasicConstraints, CertificateParams, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256,
    PKCS_ECDSA_P384_SHA384, PKCS_ED25519,
};
use rusty_tls::handrolled::verify::{verify_signature, SignatureAlgorithm, VerifyError};
use rusty_tls::handrolled::x509::Certificate;

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

// ---------------------------------------------------------------------------
// Real certificates
// ---------------------------------------------------------------------------

/// Every real root using a supported algorithm must verify against its own
/// key — RSA at three hash sizes and ECDSA on two curves, signed by actual
/// CAs rather than by this test.
#[test]
fn real_roots_verify_against_their_own_keys() {
    let anchors = load_anchors();
    let (mut verified, mut weak, mut unsupported) = (0usize, 0usize, 0usize);
    let mut failures = Vec::new();

    for der in &anchors {
        let Ok(cert) = Certificate::parse(der) else {
            continue;
        };
        match verify_signature(&cert, &cert.subject_public_key_info()) {
            Ok(()) => verified += 1,
            Err(VerifyError::WeakSignatureAlgorithm(_)) => weak += 1,
            Err(VerifyError::UnsupportedSignatureAlgorithm) => unsupported += 1,
            Err(err) => failures.push(format!("  {err}")),
        }
    }

    println!(
        "of {} anchors: {verified} verified, {weak} refused as weak, {unsupported} unsupported",
        anchors.len()
    );

    assert!(
        failures.is_empty(),
        "{} real roots failed to verify for a reason other than algorithm support:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        verified >= 100,
        "only {verified} roots verified — that is too few to have exercised \
         RSA and ECDSA at several hash sizes"
    );
}

/// The negative twin, and the one that matters. Flip one bit anywhere in a
/// real root and its signature must stop verifying — every time, for every
/// root, at every bit position sampled.
///
/// A verifier that ignored the message, or hashed the wrong bytes, or
/// verified a re-encoding rather than the parsed input, would sail through
/// the test above and die here.
#[test]
fn a_single_flipped_bit_breaks_every_real_root() {
    let anchors = load_anchors();
    let (mut tested, mut still_parsed) = (0usize, 0usize);

    for der in &anchors {
        let Ok(cert) = Certificate::parse(der) else {
            continue;
        };
        if verify_signature(&cert, &cert.subject_public_key_info()).is_err() {
            continue; // weak or unsupported algorithm; nothing to break
        }
        tested += 1;

        // Sample bit positions across the whole certificate rather than
        // testing every one: exhaustive would be ~10^6 verifications.
        for offset in (0..der.len()).step_by(97) {
            for bit in [0u8, 3, 7] {
                let mut mutated = der.clone();
                mutated[offset] ^= 1 << bit;

                let Ok(mutant) = Certificate::parse(&mutated) else {
                    continue; // the mutation broke parsing, which is also fine
                };
                still_parsed += 1;
                assert!(
                    verify_signature(&mutant, &mutant.subject_public_key_info()).is_err(),
                    "a certificate with bit {bit} of byte {offset} flipped still verified"
                );
            }
        }
    }

    assert!(tested >= 100, "only {tested} roots were mutated");
    assert!(
        still_parsed >= 1_000,
        "only {still_parsed} mutants survived parsing to reach the verifier — \
         this test is not exercising signature verification"
    );
    println!(
        "{tested} roots mutated; {still_parsed} mutants reached the verifier and all were refused"
    );
}

/// One root's key must not verify another root's certificate. Obvious, and
/// exactly what a verifier that ignored the key would get wrong.
#[test]
fn a_root_does_not_verify_under_a_different_roots_key() {
    let anchors = load_anchors();
    let parsed: Vec<_> = anchors
        .iter()
        .filter_map(|der| Certificate::parse(der).ok().map(|c| (der, c)))
        .filter(|(_, c)| verify_signature(c, &c.subject_public_key_info()).is_ok())
        .collect();

    assert!(
        parsed.len() >= 20,
        "too few verifiable roots to cross-check"
    );

    let mut checked = 0usize;
    for window in parsed.windows(2) {
        let (_, first) = &window[0];
        let (_, second) = &window[1];
        assert!(
            verify_signature(first, &second.subject_public_key_info()).is_err(),
            "a root verified under a different root's key"
        );
        checked += 1;
    }
    println!("cross-checked {checked} adjacent root pairs");
}

// ---------------------------------------------------------------------------
// Generated chains
// ---------------------------------------------------------------------------

/// A leaf must verify under the key of the CA that signed it, and must not
/// verify under any other.
#[test]
fn a_leaf_verifies_under_its_issuer_and_nothing_else() {
    for (name, algorithm) in [
        ("ECDSA P-256", &PKCS_ECDSA_P256_SHA256),
        ("ECDSA P-384", &PKCS_ECDSA_P384_SHA384),
        ("Ed25519", &PKCS_ED25519),
    ] {
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("params");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate_for(algorithm).expect("key");
        let ca = ca_params.self_signed(&ca_key).expect("CA signs");

        let leaf_params = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
        let leaf_key = KeyPair::generate_for(algorithm).expect("key");
        let leaf = leaf_params
            .signed_by(&leaf_key, &ca, &ca_key)
            .expect("leaf signs");

        // An unrelated CA, to check the key is actually consulted.
        let mut other_params = CertificateParams::new(Vec::<String>::new()).expect("params");
        other_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let other_key = KeyPair::generate_for(algorithm).expect("key");
        let other = other_params.self_signed(&other_key).expect("signs");

        let (ca_der, leaf_der, other_der) =
            (ca.der().to_vec(), leaf.der().to_vec(), other.der().to_vec());
        let ca_cert = Certificate::parse(&ca_der).expect("CA parses");
        let leaf_cert = Certificate::parse(&leaf_der).expect("leaf parses");
        let other_cert = Certificate::parse(&other_der).expect("parses");

        verify_signature(&leaf_cert, &ca_cert.subject_public_key_info())
            .unwrap_or_else(|e| panic!("{name}: leaf did not verify under its issuer: {e}"));
        assert_eq!(
            verify_signature(&leaf_cert, &other_cert.subject_public_key_info()),
            Err(VerifyError::BadSignature),
            "{name}: leaf verified under an unrelated key"
        );
        // And the leaf's own key is not its issuer's key.
        assert!(
            verify_signature(&leaf_cert, &leaf_cert.subject_public_key_info()).is_err(),
            "{name}: a CA-signed leaf verified against its own key"
        );
    }
}

/// Every byte of a generated certificate is covered by the signature.
/// Exhaustive here, because these certificates are small.
#[test]
fn every_byte_of_a_generated_certificate_is_signed() {
    let params = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("key");
    let der = params.self_signed(&key).expect("signs").der().to_vec();

    let cert = Certificate::parse(&der).expect("parses");
    verify_signature(&cert, &cert.subject_public_key_info()).expect("the baseline verifies");

    let mut reached = 0usize;
    for offset in 0..der.len() {
        let mut mutated = der.clone();
        mutated[offset] ^= 0x01;
        let Ok(mutant) = Certificate::parse(&mutated) else {
            continue;
        };
        reached += 1;
        assert!(
            verify_signature(&mutant, &mutant.subject_public_key_info()).is_err(),
            "flipping byte {offset} left the signature valid"
        );
    }
    assert!(reached > 50, "only {reached} mutants parsed");
}

// ---------------------------------------------------------------------------
// Algorithm policy
// ---------------------------------------------------------------------------

/// SHA-1 and MD5 are refused, and refused *distinctly* — a caller that wants
/// to report "this CA is using obsolete crypto" can, and one that treats all
/// errors alike still fails closed.
#[test]
fn weak_algorithms_are_refused_rather_than_verified() {
    let anchors = load_anchors();
    let mut weak = 0usize;

    for der in &anchors {
        let Ok(cert) = Certificate::parse(der) else {
            continue;
        };
        match SignatureAlgorithm::from_identifier(&cert.signature_algorithm()) {
            Err(VerifyError::WeakSignatureAlgorithm(name)) => {
                weak += 1;
                assert!(
                    name.contains("sha1") || name.contains("SHA1") || name.contains("md5"),
                    "unexpected weak algorithm {name}"
                );
                // And verification refuses too, rather than only the lookup.
                assert!(matches!(
                    verify_signature(&cert, &cert.subject_public_key_info()),
                    Err(VerifyError::WeakSignatureAlgorithm(_))
                ));
            }
            Err(VerifyError::UnsupportedSignatureAlgorithm) => {}
            Err(other) => panic!("unexpected error identifying a real root: {other}"),
            Ok(_) => {}
        }
    }

    // This machine's store had 28 SHA-1 roots when this was written. The
    // assertion is deliberately loose about the count and strict about the
    // fact: if a store has none, this test proved nothing and says so.
    assert!(
        weak > 0,
        "no SHA-1 roots in this trust store, so the weak-algorithm path was \
         never exercised against a real certificate"
    );
    println!("{weak} real roots refused for weak signature algorithms");
}

/// RSASSA-PSS is refused as unsupported rather than mis-verified. Its
/// parameters carry the hash, MGF, salt length, and trailer field, and
/// getting any of them wrong verifies something other than what was signed.
#[test]
fn rsa_pss_is_refused_as_unsupported() {
    use rusty_tls::handrolled::der::ObjectIdentifier;
    use rusty_tls::handrolled::x509::AlgorithmIdentifier;

    let identifier = AlgorithmIdentifier {
        oid: ObjectIdentifier(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0a]),
        parameters: None,
        encoded: &[],
    };
    assert_eq!(
        SignatureAlgorithm::from_identifier(&identifier),
        Err(VerifyError::UnsupportedSignatureAlgorithm)
    );
}

/// An unknown OID is refused, not guessed at.
#[test]
fn an_unknown_signature_algorithm_is_refused() {
    use rusty_tls::handrolled::der::ObjectIdentifier;
    use rusty_tls::handrolled::x509::AlgorithmIdentifier;

    for bytes in [
        &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x63][..],
        &[0x55, 0x1d, 0x13][..],
        &[0x2b, 0x06, 0x01, 0x04, 0x01][..],
    ] {
        let identifier = AlgorithmIdentifier {
            oid: ObjectIdentifier(bytes),
            parameters: None,
            encoded: &[],
        };
        assert_eq!(
            SignatureAlgorithm::from_identifier(&identifier),
            Err(VerifyError::UnsupportedSignatureAlgorithm),
            "{bytes:02x?} was not refused"
        );
    }
}

/// RFC 5758 §3.2: ECDSA signature algorithms take no parameters. A
/// certificate that supplies some is malformed, and guessing what they meant
/// is how a verifier ends up checking the wrong thing.
#[test]
fn ecdsa_parameters_are_refused() {
    use rusty_tls::handrolled::der::ObjectIdentifier;
    use rusty_tls::handrolled::x509::AlgorithmIdentifier;

    let identifier = AlgorithmIdentifier {
        oid: ObjectIdentifier(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]),
        parameters: Some(&[0x05, 0x00]), // an explicit NULL, which is wrong here
        encoded: &[],
    };
    assert_eq!(
        SignatureAlgorithm::from_identifier(&identifier),
        Err(VerifyError::MalformedParameters)
    );

    // Absent is correct.
    let identifier = AlgorithmIdentifier {
        parameters: None,
        ..identifier
    };
    assert_eq!(
        SignatureAlgorithm::from_identifier(&identifier),
        Ok(SignatureAlgorithm::EcdsaSha256)
    );
}

/// RSA PKCS#1 v1.5 takes NULL parameters, or none. Anything else is refused.
#[test]
fn rsa_parameters_must_be_null_or_absent() {
    use rusty_tls::handrolled::der::ObjectIdentifier;
    use rusty_tls::handrolled::x509::AlgorithmIdentifier;

    let sha256_rsa = ObjectIdentifier(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b]);

    for (label, parameters, expected) in [
        (
            "NULL",
            Some(&[0x05u8, 0x00][..]),
            Ok(SignatureAlgorithm::RsaPkcs1Sha256),
        ),
        ("absent", None, Ok(SignatureAlgorithm::RsaPkcs1Sha256)),
        (
            "an INTEGER",
            Some(&[0x02, 0x01, 0x00][..]),
            Err(VerifyError::MalformedParameters),
        ),
        (
            "a non-empty NULL",
            Some(&[0x05, 0x01, 0x00][..]),
            Err(VerifyError::MalformedParameters),
        ),
        (
            "trailing data after NULL",
            Some(&[0x05, 0x00, 0x00][..]),
            Err(VerifyError::MalformedParameters),
        ),
    ] {
        let identifier = AlgorithmIdentifier {
            oid: sha256_rsa,
            parameters,
            encoded: &[],
        };
        assert_eq!(
            SignatureAlgorithm::from_identifier(&identifier),
            expected,
            "RSA parameters: {label}"
        );
    }
}

// ---------------------------------------------------------------------------
// Key/signature agreement
// ---------------------------------------------------------------------------

/// An RSA signature must not be checked against an EC key, or the reverse.
/// `ring` would refuse anyway; this asserts that the refusal is this module's
/// requirement rather than a library's accident, because both sides of the
/// comparison are attacker-supplied.
#[test]
fn a_signature_is_refused_against_a_key_of_the_wrong_kind() {
    let anchors = load_anchors();

    let rsa = anchors.iter().find_map(|der| {
        let cert = Certificate::parse(der).ok()?;
        (cert.subject_public_key_info().algorithm.oid
            == rusty_tls::handrolled::x509::oid::RSA_ENCRYPTION
            && verify_signature(&cert, &cert.subject_public_key_info()).is_ok())
        .then_some(der.clone())
    });
    let ec = anchors.iter().find_map(|der| {
        let cert = Certificate::parse(der).ok()?;
        (cert.subject_public_key_info().algorithm.oid
            == rusty_tls::handrolled::x509::oid::EC_PUBLIC_KEY
            && verify_signature(&cert, &cert.subject_public_key_info()).is_ok())
        .then_some(der.clone())
    });

    let (rsa, ec) = match (rsa, ec) {
        (Some(rsa), Some(ec)) => (rsa, ec),
        _ => panic!("the trust store needs both an RSA and an EC root for this test"),
    };
    let rsa = Certificate::parse(&rsa).expect("parses");
    let ec = Certificate::parse(&ec).expect("parses");

    assert_eq!(
        verify_signature(&rsa, &ec.subject_public_key_info()),
        Err(VerifyError::KeyAlgorithmMismatch),
        "an RSA signature was checked against an EC key"
    );
    assert_eq!(
        verify_signature(&ec, &rsa.subject_public_key_info()),
        Err(VerifyError::KeyAlgorithmMismatch),
        "an EC signature was checked against an RSA key"
    );
}

/// Cross-curve verification must fail. Note what this is *not* asserting:
/// `ecdsa-with-SHA384` against a P-256 key is a perfectly legal combination
/// (the algorithm names the hash, not the curve), so the refusal here comes
/// from the signature genuinely not verifying, not from a policy check.
#[test]
fn an_ecdsa_signature_does_not_verify_across_curves() {
    let p256_params = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
    let p256_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("key");
    let p256_der = p256_params
        .self_signed(&p256_key)
        .expect("signs")
        .der()
        .to_vec();

    let p384_params = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
    let p384_key = KeyPair::generate_for(&PKCS_ECDSA_P384_SHA384).expect("key");
    let p384_der = p384_params
        .self_signed(&p384_key)
        .expect("signs")
        .der()
        .to_vec();

    let p256 = Certificate::parse(&p256_der).expect("parses");
    let p384 = Certificate::parse(&p384_der).expect("parses");

    // Each verifies under its own key.
    verify_signature(&p256, &p256.subject_public_key_info()).expect("P-256 self-verifies");
    verify_signature(&p384, &p384.subject_public_key_info()).expect("P-384 self-verifies");

    // Each fails against the other's key: a real verification failure, since
    // both (curve, hash) pairs are ones this module supports.
    assert_eq!(
        verify_signature(&p384, &p256.subject_public_key_info()),
        Err(VerifyError::BadSignature)
    );
    assert_eq!(
        verify_signature(&p256, &p384.subject_public_key_info()),
        Err(VerifyError::BadSignature)
    );
}

/// A curve this module does not implement is refused outright rather than
/// approximated with the nearest one it does.
#[test]
fn an_unsupported_curve_is_refused() {
    use rusty_tls::handrolled::der::ObjectIdentifier;
    use rusty_tls::handrolled::verify::verify_signed_data;
    use rusty_tls::handrolled::x509::{AlgorithmIdentifier, SubjectPublicKeyInfo};

    for (label, curve) in [
        // secp521r1, 1.3.132.0.35
        ("P-521", &[0x06u8, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x23][..]),
        // secp256k1, 1.3.132.0.10 — Bitcoin's curve, never valid in web PKI
        ("secp256k1", &[0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x0a][..]),
    ] {
        let key = SubjectPublicKeyInfo {
            algorithm: AlgorithmIdentifier {
                oid: ObjectIdentifier(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]),
                parameters: Some(curve),
                encoded: &[],
            },
            key: &[0x04; 65],
            encoded: &[],
        };
        let signature_algorithm = AlgorithmIdentifier {
            oid: ObjectIdentifier(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]),
            parameters: None,
            encoded: &[],
        };
        assert_eq!(
            verify_signed_data(&signature_algorithm, &key, b"message", b"signature"),
            Err(VerifyError::UnsupportedCurve),
            "{label} was not refused"
        );
    }
}

/// Ed25519 keys and signatures carry no parameters anywhere, and supplying
/// them is refused.
#[test]
fn ed25519_round_trips_and_rejects_parameters() {
    let params = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
    let key = KeyPair::generate_for(&PKCS_ED25519).expect("key");
    let der = params.self_signed(&key).expect("signs").der().to_vec();

    let cert = Certificate::parse(&der).expect("parses");
    verify_signature(&cert, &cert.subject_public_key_info()).expect("Ed25519 self-verifies");
    assert_eq!(cert.subject_public_key_info().algorithm.parameters, None);

    use rusty_tls::handrolled::der::ObjectIdentifier;
    use rusty_tls::handrolled::x509::AlgorithmIdentifier;
    let identifier = AlgorithmIdentifier {
        oid: ObjectIdentifier(&[0x2b, 0x65, 0x70]),
        parameters: Some(&[0x05, 0x00]),
        encoded: &[],
    };
    assert_eq!(
        SignatureAlgorithm::from_identifier(&identifier),
        Err(VerifyError::MalformedParameters)
    );
}

/// An empty or truncated signature must be refused, not treated as a
/// degenerate success.
#[test]
fn a_truncated_signature_is_refused() {
    use rusty_tls::handrolled::verify::verify_signed_data;

    let params = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("key");
    let der = params.self_signed(&key).expect("signs").der().to_vec();
    let cert = Certificate::parse(&der).expect("parses");

    let full = cert.signature();
    for cut in [0usize, 1, full.len() / 2, full.len() - 1] {
        assert_eq!(
            verify_signed_data(
                &cert.signature_algorithm(),
                &cert.subject_public_key_info(),
                cert.tbs_der(),
                &full[..cut],
            ),
            Err(VerifyError::BadSignature),
            "a signature truncated to {cut} bytes was accepted"
        );
    }
    // The whole thing still works, so the loop above is meaningful.
    verify_signed_data(
        &cert.signature_algorithm(),
        &cert.subject_public_key_info(),
        cert.tbs_der(),
        full,
    )
    .expect("the untruncated signature verifies");
}

/// An empty message must not verify against a signature over real content.
#[test]
fn the_message_is_actually_consulted() {
    use rusty_tls::handrolled::verify::verify_signed_data;

    let params = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("key");
    let der = params.self_signed(&key).expect("signs").der().to_vec();
    let cert = Certificate::parse(&der).expect("parses");

    for message in [&b""[..], b"x", &cert.tbs_der()[1..], &cert.tbs_der()[..10]] {
        assert_eq!(
            verify_signed_data(
                &cert.signature_algorithm(),
                &cert.subject_public_key_info(),
                message,
                cert.signature(),
            ),
            Err(VerifyError::BadSignature),
            "a {}-byte message other than the tbsCertificate verified",
            message.len()
        );
    }
}
