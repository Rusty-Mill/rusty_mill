//! TLS 1.2 handshake signatures: `verify_tls12_signature`.
//!
//! Known-answer vectors from an implementation unrelated to this crate (Python
//! `cryptography`, over OpenSSL; see `tests/data/tls12_signature_vectors.py`),
//! and a table pinning where the TLS 1.2 verifier differs from the TLS 1.3 one.
//!
//! The differences are the dangerous part. They are two, both deliberate, and
//! each should be possible only on a TLS 1.2 connection:
//!
//! - `rsa_pkcs1_*` verifies here and is refused for TLS 1.3 (RFC 8446 §4.4.3).
//! - An ECDSA scheme here names only the hash, so a key on either curve verifies
//!   under either scheme. TLS 1.3 binds the curve. The cross-curve vectors exist
//!   because a live OpenSSL server produced exactly that signature and a first
//!   version of this verifier, which copied TLS 1.3's rule, refused it.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::verify::{
    verify_tls12_signature, verify_tls13_signature, SignatureScheme, VerifyError,
};
use rusty_tls::handrolled::x509::Certificate;

struct SigVector {
    name: &'static str,
    scheme: u16,
    certificate: &'static str,
    signature: &'static str,
}

include!("data/tls12_signature_vectors.rs");

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn verify12(v: &SigVector, message: &[u8], signature: &[u8]) -> Result<(), VerifyError> {
    let der = unhex(v.certificate);
    let cert = Certificate::parse(&der).expect("the vector's certificate parses");
    verify_tls12_signature(
        SignatureScheme(v.scheme),
        &cert.subject_public_key_info(),
        message,
        signature,
    )
}

fn verify13(v: &SigVector) -> Result<(), VerifyError> {
    let der = unhex(v.certificate);
    let cert = Certificate::parse(&der).expect("the vector's certificate parses");
    verify_tls13_signature(
        SignatureScheme(v.scheme),
        &cert.subject_public_key_info(),
        &unhex(MESSAGE),
        &unhex(v.signature),
    )
}

#[test]
fn every_independent_vector_verifies() {
    assert_eq!(VECTORS.len(), 11, "the vector table changed");
    for v in VECTORS {
        verify12(v, &unhex(MESSAGE), &unhex(v.signature))
            .unwrap_or_else(|e| panic!("{}: {e}", v.name));
    }
}

#[test]
fn every_vector_is_refused_when_the_message_or_signature_changes() {
    for v in VECTORS {
        let message = unhex(MESSAGE);
        let signature = unhex(v.signature);

        let mut bad_message = message.clone();
        bad_message[0] ^= 1;
        assert_eq!(
            verify12(v, &bad_message, &signature),
            Err(VerifyError::BadSignature),
            "{}: message",
            v.name
        );

        // Every bit of the signature (ECDSA signatures are DER, so some flips
        // are malformed rather than merely wrong; all must be refused).
        for i in 0..signature.len() {
            let mut bad = signature.clone();
            bad[i] ^= 0x80;
            assert!(verify12(v, &message, &bad).is_err(), "{}: byte {i}", v.name);
        }
        assert!(verify12(v, &message, &[]).is_err(), "{}: empty", v.name);
        assert!(
            verify12(v, &message, &signature[..signature.len() - 1]).is_err(),
            "{}: truncated",
            v.name
        );
    }
}

/// Where the two verifiers must disagree, and where they must agree.
#[test]
fn the_tls12_and_tls13_verifiers_differ_exactly_where_intended() {
    for v in VECTORS {
        let tls13 = verify13(v);
        match v.name {
            // Accepted by both: the schemes mean the same in both versions.
            "ecdsa P-256 key, sha256 scheme"
            | "ecdsa P-384 key, sha384 scheme"
            | "ed25519"
            | "rsa pss rsae sha256"
            | "rsa pss rsae sha384"
            | "rsa pss rsae sha512" => assert_eq!(tls13, Ok(()), "{}", v.name),
            // TLS 1.3 binds the curve to the scheme.
            "ecdsa P-384 key, sha256 scheme (cross-curve)"
            | "ecdsa P-256 key, sha384 scheme (cross-curve)" => {
                assert_eq!(tls13, Err(VerifyError::CurveMismatch), "{}", v.name)
            }
            // TLS 1.3 forbids PKCS#1 v1.5 for handshake signatures.
            "rsa pkcs1 sha256" | "rsa pkcs1 sha384" | "rsa pkcs1 sha512" => {
                assert_eq!(tls13, Err(VerifyError::CertificateOnlyScheme), "{}", v.name)
            }
            other => panic!("unclassified vector {other}"),
        }
    }
}

#[test]
fn a_scheme_for_the_wrong_kind_of_key_is_refused() {
    let by_name = |name: &str| VECTORS.iter().find(|v| v.name == name).expect(name);
    let rsa = by_name("rsa pkcs1 sha256");
    let ec = by_name("ecdsa P-256 key, sha256 scheme");
    let ed = by_name("ed25519");

    // An RSA scheme against an EC key, and the reverse, and Ed25519 against both.
    let mismatches: [(&SigVector, u16); 6] = [
        (ec, 0x0401),
        (ec, 0x0804),
        (rsa, 0x0403),
        (rsa, 0x0807),
        (ed, 0x0403),
        (ed, 0x0401),
    ];
    for (key_from, scheme) in mismatches {
        let der = unhex(key_from.certificate);
        let cert = Certificate::parse(&der).expect("parses");
        let result = verify_tls12_signature(
            SignatureScheme(scheme),
            &cert.subject_public_key_info(),
            &unhex(MESSAGE),
            &unhex(key_from.signature),
        );
        assert_eq!(
            result,
            Err(VerifyError::KeyAlgorithmMismatch),
            "{} under 0x{scheme:04x}",
            key_from.name
        );
    }
}

#[test]
fn weak_and_unknown_schemes_are_refused() {
    let v = VECTORS
        .iter()
        .find(|v| v.name == "rsa pkcs1 sha256")
        .expect("vector");
    let der = unhex(v.certificate);
    let cert = Certificate::parse(&der).expect("parses");
    let spki = cert.subject_public_key_info();
    let (msg, sig) = (unhex(MESSAGE), unhex(v.signature));

    // SHA-1 is refused on strength, not reported as merely unsupported.
    for sha1 in [0x0201u16, 0x0203] {
        assert!(
            matches!(
                verify_tls12_signature(SignatureScheme(sha1), &spki, &msg, &sig),
                Err(VerifyError::WeakSignatureAlgorithm(_))
            ),
            "0x{sha1:04x}"
        );
    }
    // A scheme this verifier does not implement, and a made-up one.
    for unknown in [0x0603u16, 0x0808, 0x0000, 0xffff] {
        assert_eq!(
            verify_tls12_signature(SignatureScheme(unknown), &spki, &msg, &sig),
            Err(VerifyError::UnsupportedSignatureAlgorithm),
            "0x{unknown:04x}"
        );
    }
}

#[test]
fn the_tls12_supported_list_is_exactly_what_the_verifier_accepts() {
    // The ClientHello offers this list. A scheme offered but refused invites a
    // server to choose it and fail the handshake for nothing; a scheme accepted
    // but not offered is harmless but dead. Both directions are checked.
    let accepted: Vec<u16> = VECTORS.iter().map(|v| v.scheme).collect();
    for scheme in SignatureScheme::TLS12_SUPPORTED {
        assert!(
            accepted.contains(&scheme.0),
            "0x{:04x} is offered but has no vector",
            scheme.0
        );
    }
    for scheme in accepted {
        assert!(
            SignatureScheme::TLS12_SUPPORTED.contains(&SignatureScheme(scheme)),
            "0x{scheme:04x} verifies but is not offered"
        );
    }
    // SHA-1 and the TLS 1.3-forbidden combinations are never offered.
    for forbidden in [0x0201u16, 0x0203, 0x0603] {
        assert!(!SignatureScheme::TLS12_SUPPORTED.contains(&SignatureScheme(forbidden)));
    }
}
