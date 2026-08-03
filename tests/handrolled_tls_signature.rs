//! TLS 1.3 handshake signatures — stage 3c-i.
//!
//! The companion to `handrolled_verify.rs`, which covers the same operation in
//! the X.509 namespace. Two files because they are two rule sets, and the
//! whole risk of this stage is applying one where the other belongs.
//!
//! # The three tests that carry this file
//!
//! - [`the_curve_rule_is_the_opposite_of_the_x509_one`] — a P-384 key under
//!   `ecdsa_secp256r1_sha256` is refused, while the *same key* under X.509's
//!   `ecdsa-with-SHA256` is fine. Stage 2b-i was corrected by real
//!   certificates into reading the curve off the key; TLS 1.3 puts it back in
//!   the scheme. Both are right, in their own namespace.
//! - [`a_pkcs1_scheme_is_refused_in_a_handshake_but_not_in_a_certificate`] —
//!   RFC 8446 §4.4.3 forbids PKCS#1 v1.5 for CertificateVerify no matter what
//!   the client offered. The same algorithm is ordinary in a certificate.
//! - [`the_rfc8448_certificate_verify_cannot_be_checked_and_that_is_correct`]
//!   — the RFC's own example handshake is signed with a 1024-bit RSA key, and
//!   this stack refuses it. Stated as a test rather than left as a surprise.
//!
//! # Test material
//!
//! ECDSA and Ed25519 keys are generated per-test. RSA is not: `ring` cannot
//! generate RSA keys, so one 2048-bit key is embedded below. It exists solely
//! to sign test messages in this file and has never protected anything.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use ring::rand::SystemRandom;
use ring::signature;
use rusty_tls::handrolled::der::{ObjectIdentifier, Reader};
use rusty_tls::handrolled::handshake::{
    certificate_verify_content, messages, CertificateMessage, CertificateVerify, HandshakeType,
    Transcript, SERVER_CERTIFICATE_VERIFY_CONTEXT,
};
use rusty_tls::handrolled::schedule::Hash;
use rusty_tls::handrolled::verify::{
    verify_signed_data, verify_tls13_signature, SignatureScheme, VerifyError,
};
use rusty_tls::handrolled::x509::{AlgorithmIdentifier, Certificate, SubjectPublicKeyInfo};

mod rfc8448;

/// A throwaway 2048-bit RSA key in PKCS#8, for the PSS tests. Not a secret,
/// not used anywhere else, generated for this file.
const RSA_TEST_KEY_PKCS8: &str = "\
    308204bc020100300d06092a864886f70d0101010500048204a6308204a20201\
    000282010100999ed75cb9afd5fd48214ec6c973e2c322c859608e1ac4e31778\
    911cbf66fb745968623b0e977ff4ed71a383a5164be63adbca03921dd68b52ab\
    ee5ead8c843c27987a9ad1049c3d87d2d6a6bd7de5381fc71cc5bd0139ecb2e4\
    135a0ae4ccd5a28336cd09fc5299183ba950fd3447670f26bb77db9b1cd26145\
    1054c46bf38373f9ccd0eb6e1180783f96b860c480d17d9cb1ae27de953b7906\
    7e9041afa0f90b34e19e7703c41b7d74be8dee533585c86dbaacbd11bcbfb41e\
    71c7406c0407146d3957dfb6f10d37c64312cad31faa1e8fb4d7f157dc84cfe0\
    6af29c5549bc8819a9bf6a2a639d9133ed96459202c0121be6b013e3585c69c7\
    8123a4f868bd0203010001028201003437c712f0d77150d024ea472e1123429b\
    5f28ea6643792b8c3de26db82e04496f5ec90d340f91622b1816b1d7faf53fc4\
    4013b21507e976a05a6b0369d0bade4bc34be1b62cf003065947b793efe86ba4\
    79a5311de6fdce949c6c6b8e0a6a0a305c93d32c92c56cdecce0e7f3b9c28fa8\
    99ccdd981b68b7a672b87367f51de7c9014ea900039f5875dd0cb0b485e877a8\
    6f63872e19fc0aaf8b77da43eb30889f386b0886a959b3c61831f570a095f48c\
    f6790e8be14bf1e941addda5afec63fc58441c682c43b1ed70af92536410ec36\
    44e6cf3a4780cdc31297307766acfac062ab1e2d40674c01bf22b19b64248996\
    0014baf859dd4506a624c7e5cb0dcd02818100d2cc7b3955841b9dd766570241\
    63189c829778fe4dbe6396bb4a7455f21922bf5ae4fc59e93667d3c6d929380b\
    954fedac1567559e4b0d18202cb63aa6b64550791a63376c2a8aee2abdc9ba7e\
    eec3a38e143591ecc1b2e2349d9bc52a3386e658775001ae4f8a8b87eb0c59c0\
    e12637c95875d5ba2b4fed162dc11988fc7a9702818100ba8fa24298d06bfee8\
    2becd325e986aa354e3d6d2b8dc9d1ee60840503e02062f257c0c2671982f89e\
    5381c7e63c262ff37dea22425e1d2af8ac69d6f4e9acc98f3e3130c71bb0aa62\
    c9d52fac7f7e53391f148990a08fc817ffc09afd39b783306347498c7ac27eff\
    f3a9fcc5e5c2297ea6d755c816391a762481d5a79bc5cb0281805edaa8702a2c\
    e2086a8ea084614be81d351e57d186c62f25fa6d0c60482a4b5a73da2a3b8317\
    7a2cef83746ac5bb9055d06369363b9e65ebff3e5f5990fedfbdd060b1589511\
    a7a67811229f0ad301b1ad1326efdceb6445298bea3614414f6883525cec04b5\
    c20ffb3f273593f73c2d4a2ac60b405491088c3c5671d914534d0281804e01be\
    fc0fbf9e5890a5c539a8b193a059f7a411a0d3819ee908ac4b188abf9fffeb17\
    6b7206a5cfe50bcfc95108b782f7521ff34142eef947cf77d5ecc4447e6709bf\
    31c11f5fe86eb42c12ea12c9346d3f04ac5caff64ed952142bfa5406dab101e2\
    0debd26cbf03b0d3d420bf68474770e5bb3595ad3cd6477f8e61adedaf028180\
    65d25767ed298f7998bfde44a051aaa9dd4d6fdec4f8648ed3d8a8eb742e627c\
    2d021425be8dd73526a8c088daa5791e669496e42ed9aeeaa87c6d8e197a643f\
    c5d7edc594907010a45f7ad5eaaa8ddc92338e5ff0ab8a9490e14138ed25b662\
    f9d0fad2f821fbb35e8ca260175fd8a2463fb5b6f612c4fa87a6b4621fb5c496
    ";

// ---------------------------------------------------------------------------
// Key material
// ---------------------------------------------------------------------------

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<char> = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(&pair.iter().collect::<String>(), 16).expect("hex"))
        .collect()
}

/// A self-signed certificate and the PKCS#8 of the key inside it.
///
/// The certificate is there because a `SubjectPublicKeyInfo` comes from
/// parsing one — which is also how a real client obtains the key it checks a
/// CertificateVerify against, so the test path and the real path agree.
struct TestKey {
    certificate: Vec<u8>,
    pkcs8: Vec<u8>,
}

impl TestKey {
    fn generate(algorithm: &'static rcgen::SignatureAlgorithm) -> Self {
        let key = rcgen::KeyPair::generate_for(algorithm).expect("keypair");
        Self::from_key(key)
    }

    fn rsa() -> Self {
        let pkcs8 = hex(RSA_TEST_KEY_PKCS8);
        let der = rustls::pki_types::PrivatePkcs8KeyDer::from(pkcs8);
        let key = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&der, &rcgen::PKCS_RSA_SHA256)
            .expect("the embedded RSA key loads");
        Self::from_key(key)
    }

    fn from_key(key: rcgen::KeyPair) -> Self {
        let pkcs8 = key.serialize_der();
        let params =
            rcgen::CertificateParams::new(vec!["scheme.example".to_string()]).expect("params");
        let certificate = params.self_signed(&key).expect("self-sign").der().to_vec();
        Self { certificate, pkcs8 }
    }
}

fn sign_ecdsa(
    pkcs8: &[u8],
    algorithm: &'static signature::EcdsaSigningAlgorithm,
    message: &[u8],
) -> Vec<u8> {
    let rng = SystemRandom::new();
    let pair = signature::EcdsaKeyPair::from_pkcs8(algorithm, pkcs8, &rng).expect("ecdsa key");
    pair.sign(&rng, message).expect("sign").as_ref().to_vec()
}

fn sign_ed25519(pkcs8: &[u8], message: &[u8]) -> Vec<u8> {
    let pair = signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(pkcs8).expect("ed25519 key");
    pair.sign(message).as_ref().to_vec()
}

fn sign_rsa_pss(
    pkcs8: &[u8],
    padding: &'static dyn signature::RsaEncoding,
    message: &[u8],
) -> Vec<u8> {
    let rng = SystemRandom::new();
    let pair = signature::RsaKeyPair::from_pkcs8(pkcs8).expect("rsa key");
    let mut out = vec![0u8; pair.public().modulus_len()];
    pair.sign(padding, &rng, message, &mut out).expect("sign");
    out
}

const MESSAGE: &[u8] = b"the bytes a CertificateVerify would cover";

// ---------------------------------------------------------------------------
// Every supported scheme verifies what it signed
// ---------------------------------------------------------------------------

#[test]
fn every_supported_scheme_verifies_a_real_signature() {
    // ECDSA, both curves.
    for (rcgen_alg, ring_alg, scheme) in [
        (
            &rcgen::PKCS_ECDSA_P256_SHA256,
            &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            SignatureScheme::ECDSA_SECP256R1_SHA256,
        ),
        (
            &rcgen::PKCS_ECDSA_P384_SHA384,
            &signature::ECDSA_P384_SHA384_ASN1_SIGNING,
            SignatureScheme::ECDSA_SECP384R1_SHA384,
        ),
    ] {
        let key = TestKey::generate(rcgen_alg);
        let certificate = Certificate::parse(&key.certificate).expect("parses");
        let signature = sign_ecdsa(&key.pkcs8, ring_alg, MESSAGE);

        assert_eq!(
            verify_tls13_signature(
                scheme,
                &certificate.subject_public_key_info(),
                MESSAGE,
                &signature
            ),
            Ok(()),
            "{scheme:?} did not verify its own signature"
        );
    }

    // Ed25519.
    let key = TestKey::generate(&rcgen::PKCS_ED25519);
    let certificate = Certificate::parse(&key.certificate).expect("parses");
    let signature = sign_ed25519(&key.pkcs8, MESSAGE);
    assert_eq!(
        verify_tls13_signature(
            SignatureScheme::ED25519,
            &certificate.subject_public_key_info(),
            MESSAGE,
            &signature
        ),
        Ok(())
    );

    // RSASSA-PSS, all three hashes. This is what stage 2b-i refused and what
    // every RSA server on the internet signs a TLS 1.3 handshake with.
    let key = TestKey::rsa();
    let certificate = Certificate::parse(&key.certificate).expect("parses");
    for (padding, scheme) in [
        (
            &signature::RSA_PSS_SHA256 as &dyn signature::RsaEncoding,
            SignatureScheme::RSA_PSS_RSAE_SHA256,
        ),
        (
            &signature::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_RSAE_SHA384,
        ),
        (
            &signature::RSA_PSS_SHA512,
            SignatureScheme::RSA_PSS_RSAE_SHA512,
        ),
    ] {
        let signature = sign_rsa_pss(&key.pkcs8, padding, MESSAGE);
        assert_eq!(
            verify_tls13_signature(
                scheme,
                &certificate.subject_public_key_info(),
                MESSAGE,
                &signature
            ),
            Ok(()),
            "{scheme:?} did not verify its own signature"
        );
    }
}

/// The set a client advertises must be the set it will accept. Offering a
/// scheme that would then be refused invites a server to pick it and fail the
/// handshake for no reason at all.
#[test]
fn every_advertised_scheme_is_one_this_module_can_actually_use() {
    let ecdsa256 = TestKey::generate(&rcgen::PKCS_ECDSA_P256_SHA256);
    let ecdsa384 = TestKey::generate(&rcgen::PKCS_ECDSA_P384_SHA384);
    let ed25519 = TestKey::generate(&rcgen::PKCS_ED25519);
    let rsa = TestKey::rsa();

    for scheme in SignatureScheme::TLS13_SUPPORTED {
        let key = match *scheme {
            SignatureScheme::ECDSA_SECP256R1_SHA256 => &ecdsa256,
            SignatureScheme::ECDSA_SECP384R1_SHA384 => &ecdsa384,
            SignatureScheme::ED25519 => &ed25519,
            _ => &rsa,
        };
        let certificate = Certificate::parse(&key.certificate).expect("parses");

        // A wrong signature is fine here — what must not happen is a refusal
        // *before* the signature is looked at, which is what an unsupported or
        // mismatched scheme would produce.
        assert_eq!(
            verify_tls13_signature(
                *scheme,
                &certificate.subject_public_key_info(),
                MESSAGE,
                &[0u8; 64]
            ),
            Err(VerifyError::BadSignature),
            "{scheme:?} is advertised but was refused before the signature was checked"
        );
    }
}

// ---------------------------------------------------------------------------
// The two namespaces disagree — the point of the split
// ---------------------------------------------------------------------------

/// 1.2.840.10045.4.3.2 — `ecdsa-with-SHA256`, the X.509 identifier.
const ECDSA_WITH_SHA256: [u8; 8] = [0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
/// 1.2.840.113549.1.1.11 — `sha256WithRSAEncryption`.
const SHA256_WITH_RSA: [u8; 9] = [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];

fn identifier(oid: &[u8]) -> AlgorithmIdentifier<'_> {
    AlgorithmIdentifier {
        oid: ObjectIdentifier(oid),
        parameters: None,
        encoded: &[],
    }
}

/// The headline. Stage 2b-i was corrected by real certificates into reading an
/// ECDSA key's curve off the *key*; TLS 1.3 puts the curve back in the
/// *scheme*. So one P-384 key gets two different answers to what looks like
/// the same question, and both answers are right.
#[test]
fn the_curve_rule_is_the_opposite_of_the_x509_one() {
    let key = TestKey::generate(&rcgen::PKCS_ECDSA_P384_SHA384);
    let certificate = Certificate::parse(&key.certificate).expect("parses");
    let spki = certificate.subject_public_key_info();

    // TLS: the scheme names P-256, the key is P-384, so it is refused before
    // any signature is examined.
    assert_eq!(
        verify_tls13_signature(
            SignatureScheme::ECDSA_SECP256R1_SHA256,
            &spki,
            MESSAGE,
            &[0u8; 64]
        ),
        Err(VerifyError::CurveMismatch),
        "TLS accepted a P-384 key under a scheme that names P-256"
    );

    // X.509: `ecdsa-with-SHA256` names only a hash. The identifier resolves,
    // the key's own curve is used, and the call gets as far as checking the
    // signature — which is the distinction being asserted. (That this
    // combination verifies for real is covered by the trust-store corpus in
    // `handrolled_verify.rs`: three roots on this machine are P-384 keys
    // signed with SHA-256.)
    assert_eq!(
        verify_signed_data(&identifier(&ECDSA_WITH_SHA256), &spki, MESSAGE, &[0u8; 64]),
        Err(VerifyError::BadSignature),
        "X.509 refused a P-384 key on curve grounds, which is the TLS rule"
    );
}

/// RFC 8446 §4.4.3: "RSA signatures MUST use an RSASSA-PSS algorithm,
/// regardless of whether RSASSA-PKCS1-v1_5 algorithms appear in
/// 'signature_algorithms'." The same algorithm is entirely ordinary in a
/// certificate, which is why this needs its own error rather than being folded
/// into "unsupported".
#[test]
fn a_pkcs1_scheme_is_refused_in_a_handshake_but_not_in_a_certificate() {
    let key = TestKey::rsa();
    let certificate = Certificate::parse(&key.certificate).expect("parses");
    let spki = certificate.subject_public_key_info();

    for scheme in [
        SignatureScheme::RSA_PKCS1_SHA256,
        SignatureScheme::RSA_PKCS1_SHA384,
        SignatureScheme::RSA_PKCS1_SHA512,
    ] {
        assert_eq!(
            verify_tls13_signature(scheme, &spki, MESSAGE, &[0u8; 256]),
            Err(VerifyError::CertificateOnlyScheme),
            "{scheme:?} was accepted for a handshake signature"
        );
    }

    // The certificate namespace resolves the same algorithm and proceeds to
    // the signature check.
    assert_eq!(
        verify_signed_data(&identifier(&SHA256_WITH_RSA), &spki, MESSAGE, &[0u8; 256]),
        Err(VerifyError::BadSignature),
        "PKCS#1 v1.5 should be ordinary in the X.509 namespace"
    );
}

/// A refusal that is really a downgrade attempt must not read as a gap. If
/// `rsa_pkcs1_*` reported "unsupported", a reader would reasonably add support
/// for it — which is precisely the change RFC 8446 forbids.
#[test]
fn the_certificate_only_refusal_is_not_the_unsupported_refusal() {
    assert_ne!(
        VerifyError::CertificateOnlyScheme,
        VerifyError::UnsupportedSignatureAlgorithm
    );
    assert_ne!(
        VerifyError::CertificateOnlyScheme.to_string(),
        VerifyError::UnsupportedSignatureAlgorithm.to_string()
    );
    assert_ne!(
        VerifyError::CurveMismatch.to_string(),
        VerifyError::UnsupportedCurve.to_string()
    );
}

// ---------------------------------------------------------------------------
// Refusal
// ---------------------------------------------------------------------------

#[test]
fn sha1_schemes_are_refused_on_strength_not_availability() {
    let key = TestKey::generate(&rcgen::PKCS_ECDSA_P256_SHA256);
    let certificate = Certificate::parse(&key.certificate).expect("parses");
    let spki = certificate.subject_public_key_info();

    assert_eq!(
        verify_tls13_signature(SignatureScheme::ECDSA_SHA1, &spki, MESSAGE, &[0u8; 64]),
        Err(VerifyError::WeakSignatureAlgorithm("ecdsa_sha1"))
    );
    assert_eq!(
        verify_tls13_signature(SignatureScheme::RSA_PKCS1_SHA1, &spki, MESSAGE, &[0u8; 64]),
        Err(VerifyError::WeakSignatureAlgorithm("rsa_pkcs1_sha1"))
    );
}

#[test]
fn unimplemented_schemes_are_refused_rather_than_approximated() {
    let key = TestKey::generate(&rcgen::PKCS_ECDSA_P256_SHA256);
    let certificate = Certificate::parse(&key.certificate).expect("parses");
    let spki = certificate.subject_public_key_info();

    for scheme in [
        SignatureScheme::ECDSA_SECP521R1_SHA512,
        SignatureScheme::ED448,
        SignatureScheme::RSA_PSS_PSS_SHA256,
        SignatureScheme::RSA_PSS_PSS_SHA384,
        SignatureScheme::RSA_PSS_PSS_SHA512,
        // Unallocated, and whatever a hostile peer invents.
        SignatureScheme(0x0000),
        SignatureScheme(0x0402),
        SignatureScheme(0xfe00),
        SignatureScheme(0xffff),
    ] {
        assert_eq!(
            verify_tls13_signature(scheme, &spki, MESSAGE, &[0u8; 64]),
            Err(VerifyError::UnsupportedSignatureAlgorithm),
            "{scheme:?} was not refused"
        );
    }
}

/// A scheme naming one key type cannot be used with another, however
/// well-formed both are.
#[test]
fn a_scheme_and_a_key_of_different_kinds_are_refused() {
    let ecdsa = TestKey::generate(&rcgen::PKCS_ECDSA_P256_SHA256);
    let ed25519 = TestKey::generate(&rcgen::PKCS_ED25519);
    let rsa = TestKey::rsa();

    let ecdsa_cert = Certificate::parse(&ecdsa.certificate).expect("parses");
    let ed25519_cert = Certificate::parse(&ed25519.certificate).expect("parses");
    let rsa_cert = Certificate::parse(&rsa.certificate).expect("parses");

    for (scheme, spki, what) in [
        (
            SignatureScheme::RSA_PSS_RSAE_SHA256,
            ecdsa_cert.subject_public_key_info(),
            "PSS with an EC key",
        ),
        (
            SignatureScheme::ECDSA_SECP256R1_SHA256,
            rsa_cert.subject_public_key_info(),
            "ECDSA with an RSA key",
        ),
        (
            SignatureScheme::ED25519,
            rsa_cert.subject_public_key_info(),
            "Ed25519 with an RSA key",
        ),
        (
            SignatureScheme::RSA_PSS_RSAE_SHA256,
            ed25519_cert.subject_public_key_info(),
            "PSS with an Ed25519 key",
        ),
    ] {
        assert_eq!(
            verify_tls13_signature(scheme, &spki, MESSAGE, &[0u8; 64]),
            Err(VerifyError::KeyAlgorithmMismatch),
            "{what} was accepted"
        );
    }
}

/// A signature over different bytes does not verify — the property that makes
/// the whole thing worth doing.
#[test]
fn a_signature_over_other_bytes_does_not_verify() {
    let key = TestKey::generate(&rcgen::PKCS_ECDSA_P256_SHA256);
    let certificate = Certificate::parse(&key.certificate).expect("parses");
    let spki = certificate.subject_public_key_info();
    let signature = sign_ecdsa(
        &key.pkcs8,
        &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        MESSAGE,
    );

    assert_eq!(
        verify_tls13_signature(
            SignatureScheme::ECDSA_SECP256R1_SHA256,
            &spki,
            MESSAGE,
            &signature
        ),
        Ok(())
    );

    let mut other = MESSAGE.to_vec();
    other[0] ^= 1;
    assert_eq!(
        verify_tls13_signature(
            SignatureScheme::ECDSA_SECP256R1_SHA256,
            &spki,
            &other,
            &signature
        ),
        Err(VerifyError::BadSignature)
    );

    for index in 0..signature.len() {
        let mut tampered = signature.clone();
        tampered[index] ^= 0x80;
        assert_eq!(
            verify_tls13_signature(
                SignatureScheme::ECDSA_SECP256R1_SHA256,
                &spki,
                MESSAGE,
                &tampered
            ),
            Err(VerifyError::BadSignature),
            "a signature with byte {index} flipped verified"
        );
    }
}

// ---------------------------------------------------------------------------
// RFC 8448, and a limitation stated rather than discovered
// ---------------------------------------------------------------------------

/// Read an RSA key's modulus size out of its `SubjectPublicKeyInfo`, so this
/// file's claim about RFC 8448 is measured rather than asserted.
fn rsa_modulus_bits(spki: &SubjectPublicKeyInfo<'_>) -> usize {
    let mut reader = Reader::new(spki.key);
    let mut sequence = reader.read_sequence().expect("RSAPublicKey is a SEQUENCE");
    let modulus = sequence
        .read_unsigned_integer()
        .expect("the modulus is an INTEGER");
    modulus.len() * 8
}

/// RFC 8448's own CertificateVerify cannot be checked here, and should not be.
///
/// Everything lines up: the scheme is `rsa_pss_rsae_sha256`, which this module
/// supports; the transcript is computable from stage 3b; the certificate
/// parses. What stops it is the key — RFC 8448 uses a 1024-bit RSA key, and
/// `ring`'s PSS verifiers enforce a 2048–8192 bit modulus.
///
/// That refusal is the right answer in 2026, so this asserts it rather than
/// quietly leaving the RFC's handshake unverified and unmentioned. The cost is
/// real and worth naming: the one published TLS 1.3 trace with a signature in
/// it cannot serve as a known-answer test for this code, which is why the
/// positive coverage above is built on generated keys instead.
#[test]
fn the_rfc8448_certificate_verify_cannot_be_checked_and_that_is_correct() {
    let flight = rfc8448::hex(rfc8448::SERVER_FLIGHT);
    let parsed = messages(&flight).expect("the flight parses");

    // The transcript a CertificateVerify covers: everything up to it.
    let mut transcript = Transcript::new(Hash::Sha256);
    transcript.add(&rfc8448::hex(rfc8448::CLIENT_HELLO));
    transcript.add(&rfc8448::hex(rfc8448::SERVER_HELLO));
    for message in &parsed {
        if message.typ == HandshakeType::CertificateVerify {
            break;
        }
        transcript.add_message(message);
    }

    let certificates = parsed
        .iter()
        .find(|m| m.typ == HandshakeType::Certificate)
        .map(|m| CertificateMessage::parse(m.body).expect("parses"))
        .expect("the flight carries a Certificate");
    let leaf = Certificate::parse(certificates.entries[0].certificate).expect("parses");
    let spki = leaf.subject_public_key_info();

    let verify = parsed
        .iter()
        .find(|m| m.typ == HandshakeType::CertificateVerify)
        .map(|m| CertificateVerify::parse(m.body).expect("parses"))
        .expect("the flight carries a CertificateVerify");

    // The scheme is one this module implements...
    assert_eq!(
        SignatureScheme(verify.scheme),
        SignatureScheme::RSA_PSS_RSAE_SHA256
    );
    assert!(SignatureScheme::TLS13_SUPPORTED.contains(&SignatureScheme(verify.scheme)));

    // ...and the key is half the size `ring` will accept.
    assert_eq!(
        rsa_modulus_bits(&spki),
        1024,
        "RFC 8448's example key is not 1024-bit after all — this test's premise has moved"
    );

    let content = certificate_verify_content(SERVER_CERTIFICATE_VERIFY_CONTEXT, &transcript.hash());
    assert_eq!(
        verify_tls13_signature(
            SignatureScheme(verify.scheme),
            &spki,
            &content,
            verify.signature
        ),
        Err(VerifyError::BadSignature),
        "a 1024-bit RSA signature was accepted"
    );
}

/// The same key with different `AlgorithmIdentifier` parameters.
///
/// Only the parameters are replaced — the key bytes and the algorithm OID are
/// the real ones off a real certificate, so nothing built with this is a
/// fabricated key.
fn with_parameters<'a>(
    spki: SubjectPublicKeyInfo<'a>,
    parameters: Option<&'a [u8]>,
) -> SubjectPublicKeyInfo<'a> {
    SubjectPublicKeyInfo {
        algorithm: AlgorithmIdentifier {
            oid: spki.algorithm.oid,
            parameters,
            encoded: spki.algorithm.encoded,
        },
        key: spki.key,
        encoded: spki.encoded,
    }
}

/// A key whose `AlgorithmIdentifier` parameters are wrong is refused, by the
/// same rules the certificate namespace applies.
///
/// This exists because a mutation run found it missing. Deleting the Ed25519
/// parameters check left every test in this file passing — `rcgen` produces
/// conforming keys, so nothing here had ever presented a malformed one. The
/// certificate namespace *did* cover it, which made the gap invisible: the two
/// paths looked equally tested and were not.
///
/// The RSA half is a real asymmetry the same investigation turned up. A leaf's
/// own key is what a CertificateVerify is checked against, and path validation
/// never inspects it — only the *issuer's* key — so without this check a leaf
/// would be held to a lower standard than the CA above it.
#[test]
fn a_key_with_malformed_parameters_is_refused() {
    /// `NULL`, DER-encoded: the classic wrong answer for "no parameters".
    const NULL: &[u8] = &[0x05, 0x00];
    /// A `SEQUENCE {}`, standing in for anything else entirely.
    const NONSENSE: &[u8] = &[0x30, 0x00];

    let ed25519 = TestKey::generate(&rcgen::PKCS_ED25519);
    let ed25519_cert = Certificate::parse(&ed25519.certificate).expect("parses");
    let rsa = TestKey::rsa();
    let rsa_cert = Certificate::parse(&rsa.certificate).expect("parses");

    // Ed25519: RFC 8410 §3 says absent. A NULL is not absent.
    for parameters in [NULL, NONSENSE] {
        assert_eq!(
            verify_tls13_signature(
                SignatureScheme::ED25519,
                &with_parameters(ed25519_cert.subject_public_key_info(), Some(parameters)),
                MESSAGE,
                &[0u8; 64]
            ),
            Err(VerifyError::MalformedParameters),
            "an Ed25519 key with parameters {parameters:02x?} was accepted"
        );
    }

    // RSA: NULL or absent, and nothing else.
    for parameters in [None, Some(NULL)] {
        assert_eq!(
            verify_tls13_signature(
                SignatureScheme::RSA_PSS_RSAE_SHA256,
                &with_parameters(rsa_cert.subject_public_key_info(), parameters),
                MESSAGE,
                &[0u8; 256]
            ),
            Err(VerifyError::BadSignature),
            "a conforming RSA key was refused before its signature was checked"
        );
    }
    assert_eq!(
        verify_tls13_signature(
            SignatureScheme::RSA_PSS_RSAE_SHA256,
            &with_parameters(rsa_cert.subject_public_key_info(), Some(NONSENSE)),
            MESSAGE,
            &[0u8; 256]
        ),
        Err(VerifyError::MalformedParameters),
        "an RSA key with nonsense parameters was accepted"
    );

    // An EC key's parameters are required — they are where the curve lives —
    // so removing them is malformed rather than tidy.
    let ecdsa = TestKey::generate(&rcgen::PKCS_ECDSA_P256_SHA256);
    let ecdsa_cert = Certificate::parse(&ecdsa.certificate).expect("parses");
    assert_eq!(
        verify_tls13_signature(
            SignatureScheme::ECDSA_SECP256R1_SHA256,
            &with_parameters(ecdsa_cert.subject_public_key_info(), None),
            MESSAGE,
            &[0u8; 64]
        ),
        Err(VerifyError::MalformedParameters),
        "an EC key with no curve named was accepted"
    );
}
