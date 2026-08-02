//! Certificate signature verification — stage 2b-i.
//!
//! This is the first part of the hand-rolled engine that *decides* something.
//! Everything before it reported: the record layer framed bytes,
//! [`super::x509`] said what a certificate claims. This module answers a
//! question with a security consequence — "was this certificate signed by the
//! private key belonging to that public key?" — and a wrong answer here is
//! the wrong kind of wrong.
//!
//! # What it still does not do
//!
//! One signature, against one key you supply. It does not build a chain, does
//! not look at a clock, does not check `basicConstraints` or `keyUsage`, and
//! does not decide whether the key you passed in should be trusted. Verifying
//! a signature proves who signed something, never that they were allowed to.
//! Full path validation is stage 2b-ii.
//!
//! In particular: a self-signed certificate verifying against its own key
//! means nothing at all about trust. Anyone can generate one. The corpus
//! tests do exactly that, on real roots, as a *correctness* check on this
//! code — not as a statement about those roots.
//!
//! # Algorithms, and the ones deliberately refused
//!
//! | Algorithm | Status |
//! | --- | --- |
//! | RSA PKCS#1 v1.5 + SHA-256/384/512 | supported |
//! | ECDSA + SHA-256/384, over P-256 or P-384 | supported |
//! | Ed25519 | supported |
//! | RSA PKCS#1 v1.5 + SHA-1, + MD5 | **refused** as weak |
//! | ECDSA + SHA-1 | **refused** as weak |
//! | RSASSA-PSS | **refused** as unsupported |
//!
//! SHA-1 is refused rather than merely discouraged, and that is a real
//! decision rather than a default: 28 of the 152 roots in this machine's own
//! trust store carry SHA-1 self-signatures. They are not broken by this — a
//! trust anchor's self-signature is never checked during path validation (RFC
//! 5280 §6.1 starts from the anchor's *key*, not its certificate), so
//! refusing to verify it costs nothing. What refusing does buy is that a
//! SHA-1 signature can never authenticate a link *within* a chain, which is
//! where chosen-prefix collisions actually bite.
//!
//! RSASSA-PSS is refused for a duller reason: its parameters carry the hash,
//! the mask generation function, the salt length, and a trailer field, and
//! getting any of them wrong means verifying something other than what was
//! signed. It is rare in the certificates that exist, so the honest move is
//! to fail closed and implement it when something needs it, rather than
//! implement it speculatively and be subtly wrong in a way no test here would
//! catch.
//!
//! Refusal is always [`VerifyError`], never `Ok`. There is no configuration
//! that relaxes any of this.
//!
//! # Primitives
//!
//! `ring`, per ADR-0002 §6 — this module hand-rolls the *decision* (which
//! algorithm, which key, which bytes), not the arithmetic. `ring`'s RSA
//! verifiers also enforce a 2048–8192 bit modulus, so undersized RSA keys are
//! refused as a side effect rather than needing a check here.
//!
//! # The curve is not implied by the hash
//!
//! Worth stating because getting it wrong is easy and this module did, until
//! real certificates said otherwise. `ecdsa-with-SHA256` names a *hash*. It
//! says nothing about which curve the key uses, and RFC 5758 does not pair
//! them: a P-384 key signed with SHA-256 is entirely conforming, and three of
//! the roots in this machine's trust store are exactly that.
//!
//! So the hash comes from the signature algorithm, the curve comes from the
//! key's own parameters, and the verifier is chosen from the combination —
//! all four of which `ring` provides. An implementation that reads the curve
//! off the signature algorithm rejects real certificates, which is the safe
//! direction to be wrong in but wrong nonetheless.

use ring::signature;

use super::der::{ObjectIdentifier, Reader};
use super::x509::{AlgorithmIdentifier, Certificate, SubjectPublicKeyInfo};

/// Why a signature was not accepted.
///
/// Every variant means "not verified". None of them means "verified with a
/// caveat".
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VerifyError {
    /// The signature algorithm is one this module refuses on strength
    /// grounds — SHA-1 or MD5. See the module docs on why refusing costs
    /// nothing for trust anchors.
    WeakSignatureAlgorithm(&'static str),
    /// The signature algorithm is not implemented. Distinct from
    /// [`VerifyError::WeakSignatureAlgorithm`]: this is a gap, not a refusal.
    UnsupportedSignatureAlgorithm,
    /// The public key's algorithm is not one this module can use.
    UnsupportedKeyAlgorithm,
    /// The signature algorithm and the key algorithm disagree — an RSA
    /// signature cannot be verified with an elliptic-curve key, however
    /// well-formed both are.
    KeyAlgorithmMismatch,
    /// The key is on an elliptic curve this module does not support. Only
    /// P-256 and P-384 are implemented; P-521 and everything else is refused
    /// rather than approximated.
    UnsupportedCurve,
    /// An `AlgorithmIdentifier`'s parameters were absent where they are
    /// required, present where they are forbidden, or malformed.
    MalformedParameters,
    /// The signature did not verify.
    ///
    /// Deliberately carries nothing. Every reason a signature can fail looks
    /// identical from outside, because the alternative is an oracle.
    BadSignature,
}

impl core::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WeakSignatureAlgorithm(name) => {
                write!(f, "{name} is too weak to authenticate a certificate")
            }
            Self::UnsupportedSignatureAlgorithm => f.write_str("unsupported signature algorithm"),
            Self::UnsupportedKeyAlgorithm => f.write_str("unsupported public key algorithm"),
            Self::KeyAlgorithmMismatch => {
                f.write_str("the signature algorithm does not match the key's algorithm")
            }
            Self::UnsupportedCurve => f.write_str("unsupported elliptic curve"),
            Self::MalformedParameters => f.write_str("malformed algorithm parameters"),
            Self::BadSignature => f.write_str("signature verification failed"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Signature algorithm OIDs.
mod oid {
    use super::ObjectIdentifier;

    const fn oid(bytes: &'static [u8]) -> ObjectIdentifier<'static> {
        ObjectIdentifier(bytes)
    }

    /// 1.2.840.113549.1.1.4 — md5WithRSAEncryption.
    pub const MD5_RSA: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x04]);
    /// 1.2.840.113549.1.1.5 — sha1WithRSAEncryption.
    pub const SHA1_RSA: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x05]);
    /// 1.2.840.113549.1.1.10 — RSASSA-PSS.
    pub const RSA_PSS: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0a]);
    /// 1.2.840.113549.1.1.11 — sha256WithRSAEncryption.
    pub const SHA256_RSA: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b]);
    /// 1.2.840.113549.1.1.12 — sha384WithRSAEncryption.
    pub const SHA384_RSA: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0c]);
    /// 1.2.840.113549.1.1.13 — sha512WithRSAEncryption.
    pub const SHA512_RSA: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0d]);

    /// 1.2.840.10045.4.1 — ecdsa-with-SHA1.
    pub const SHA1_ECDSA: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x01]);
    /// 1.2.840.10045.4.3.2 — ecdsa-with-SHA256.
    pub const SHA256_ECDSA: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]);
    /// 1.2.840.10045.4.3.3 — ecdsa-with-SHA384.
    pub const SHA384_ECDSA: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x03]);

    /// 1.2.840.113549.1.1.1 — rsaEncryption.
    pub const RSA_ENCRYPTION: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01]);
    /// 1.2.840.10045.2.1 — id-ecPublicKey.
    pub const EC_PUBLIC_KEY: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]);
    /// 1.3.101.112 — id-Ed25519.
    pub const ED25519: ObjectIdentifier<'static> = oid(&[0x2b, 0x65, 0x70]);

    /// 1.2.840.10045.3.1.7 — prime256v1 / secp256r1 / P-256.
    pub const P256: ObjectIdentifier<'static> =
        oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]);
    /// 1.3.132.0.34 — secp384r1 / P-384.
    pub const P384: ObjectIdentifier<'static> = oid(&[0x2b, 0x81, 0x04, 0x00, 0x22]);
}

/// A signature algorithm this module will verify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignatureAlgorithm {
    /// RSA PKCS#1 v1.5 with SHA-256.
    RsaPkcs1Sha256,
    /// RSA PKCS#1 v1.5 with SHA-384.
    RsaPkcs1Sha384,
    /// RSA PKCS#1 v1.5 with SHA-512.
    RsaPkcs1Sha512,
    /// ECDSA with SHA-256, over whichever curve the key uses.
    EcdsaSha256,
    /// ECDSA with SHA-384, over whichever curve the key uses.
    EcdsaSha384,
    /// Ed25519 (which fixes its own hash).
    Ed25519,
}

impl SignatureAlgorithm {
    /// Identify a signature algorithm, refusing the weak and the unsupported.
    ///
    /// Also checks the `AlgorithmIdentifier`'s parameters, which are not
    /// decoration: RFC 4055 §5 requires `NULL` for RSA PKCS#1 v1.5 and RFC
    /// 5758 §3.2 requires *absent* for ECDSA. A certificate that gets this
    /// wrong is not one to guess about.
    pub fn from_identifier(identifier: &AlgorithmIdentifier<'_>) -> Result<Self, VerifyError> {
        let algorithm = match identifier.oid {
            oid::SHA256_RSA => Self::RsaPkcs1Sha256,
            oid::SHA384_RSA => Self::RsaPkcs1Sha384,
            oid::SHA512_RSA => Self::RsaPkcs1Sha512,
            oid::SHA256_ECDSA => Self::EcdsaSha256,
            oid::SHA384_ECDSA => Self::EcdsaSha384,
            oid::ED25519 => Self::Ed25519,

            oid::SHA1_RSA => {
                return Err(VerifyError::WeakSignatureAlgorithm("sha1WithRSAEncryption"))
            }
            oid::MD5_RSA => {
                return Err(VerifyError::WeakSignatureAlgorithm("md5WithRSAEncryption"))
            }
            oid::SHA1_ECDSA => return Err(VerifyError::WeakSignatureAlgorithm("ecdsa-with-SHA1")),
            // Named rather than left to the catch-all, so that "PSS was
            // considered and deliberately not implemented" is visible here
            // and not only in the module docs.
            oid::RSA_PSS => return Err(VerifyError::UnsupportedSignatureAlgorithm),
            _ => return Err(VerifyError::UnsupportedSignatureAlgorithm),
        };

        match algorithm {
            // RFC 4055 §5: parameters MUST be present and NULL. Absent is
            // accepted too — some issuers omit it, the meaning is not in
            // dispute, and rejecting would refuse certificates that verify
            // correctly everywhere else. Anything *else* is refused, because
            // then the meaning genuinely is in dispute.
            Self::RsaPkcs1Sha256 | Self::RsaPkcs1Sha384 | Self::RsaPkcs1Sha512 => {
                match identifier.parameters {
                    None => {}
                    Some(bytes) => {
                        let mut reader = Reader::new(bytes);
                        reader
                            .read_null()
                            .map_err(|_| VerifyError::MalformedParameters)?;
                        reader
                            .finish()
                            .map_err(|_| VerifyError::MalformedParameters)?;
                    }
                }
            }
            // RFC 5758 §3.2 and RFC 8410 §3: absent, full stop.
            Self::EcdsaSha256 | Self::EcdsaSha384 | Self::Ed25519 => {
                if identifier.parameters.is_some() {
                    return Err(VerifyError::MalformedParameters);
                }
            }
        }

        Ok(algorithm)
    }

    /// The `ring` verifier for this algorithm, given the key's curve.
    ///
    /// `curve` is `Some` exactly for ECDSA, because that is the only case
    /// where the verifier depends on something the signature algorithm does
    /// not state.
    fn ring_algorithm(self, curve: Option<Curve>) -> &'static dyn signature::VerificationAlgorithm {
        match (self, curve) {
            // The 2048–8192 bound is `ring`'s, and is why undersized RSA
            // moduli need no check of their own here.
            (Self::RsaPkcs1Sha256, _) => &signature::RSA_PKCS1_2048_8192_SHA256,
            (Self::RsaPkcs1Sha384, _) => &signature::RSA_PKCS1_2048_8192_SHA384,
            (Self::RsaPkcs1Sha512, _) => &signature::RSA_PKCS1_2048_8192_SHA512,
            (Self::EcdsaSha256, Some(Curve::P256)) => &signature::ECDSA_P256_SHA256_ASN1,
            (Self::EcdsaSha256, Some(Curve::P384)) => &signature::ECDSA_P384_SHA256_ASN1,
            (Self::EcdsaSha384, Some(Curve::P256)) => &signature::ECDSA_P256_SHA384_ASN1,
            (Self::EcdsaSha384, Some(Curve::P384)) => &signature::ECDSA_P384_SHA384_ASN1,
            (Self::Ed25519, _) => &signature::ED25519,
            // Unreachable: `check_key_compatibility` returns `Some` for every
            // ECDSA algorithm before this is called, and errors otherwise.
            (Self::EcdsaSha256 | Self::EcdsaSha384, None) => &signature::ED25519,
        }
    }

    /// The key algorithm OID this signature algorithm requires.
    fn required_key_algorithm(self) -> ObjectIdentifier<'static> {
        match self {
            Self::RsaPkcs1Sha256 | Self::RsaPkcs1Sha384 | Self::RsaPkcs1Sha512 => {
                oid::RSA_ENCRYPTION
            }
            Self::EcdsaSha256 | Self::EcdsaSha384 => oid::EC_PUBLIC_KEY,
            Self::Ed25519 => oid::ED25519,
        }
    }

    const fn is_ecdsa(self) -> bool {
        matches!(self, Self::EcdsaSha256 | Self::EcdsaSha384)
    }
}

/// The elliptic curves this module verifies over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Curve {
    P256,
    P384,
}

/// Check that a public key can be used with this signature algorithm, and
/// return the key's curve if it has one.
///
/// The algorithm check stops an RSA signature from being checked against an
/// EC key. The curve is *read from the key*, not inferred from the signature
/// algorithm — see the module docs on why that distinction matters — and an
/// unrecognised curve is refused rather than guessed at.
fn check_key_compatibility(
    algorithm: SignatureAlgorithm,
    key: &SubjectPublicKeyInfo<'_>,
) -> Result<Option<Curve>, VerifyError> {
    if key.algorithm.oid != algorithm.required_key_algorithm() {
        // Distinguish "this key is of a kind we do not handle" from "this key
        // is fine but not for this signature", because they are different
        // bugs for whoever is reading the error.
        return Err(match key.algorithm.oid {
            oid::RSA_ENCRYPTION | oid::EC_PUBLIC_KEY | oid::ED25519 => {
                VerifyError::KeyAlgorithmMismatch
            }
            _ => VerifyError::UnsupportedKeyAlgorithm,
        });
    }

    if algorithm.is_ecdsa() {
        // RFC 5480 §2.1.1: the key's parameters name the curve.
        let bytes = key
            .algorithm
            .parameters
            .ok_or(VerifyError::MalformedParameters)?;
        let mut reader = Reader::new(bytes);
        let named = reader
            .read_oid()
            .map_err(|_| VerifyError::MalformedParameters)?;
        reader
            .finish()
            .map_err(|_| VerifyError::MalformedParameters)?;

        return match named {
            oid::P256 => Ok(Some(Curve::P256)),
            oid::P384 => Ok(Some(Curve::P384)),
            // P-521, secp256k1, an explicit curve specification — all
            // refused. Verifying over a curve this module does not know is
            // not something to attempt.
            _ => Err(VerifyError::UnsupportedCurve),
        };
    }

    // RSA: parameters MUST be NULL (RFC 4055 §1.2), with absent accepted for
    // the same reason as on the signature side. Ed25519: absent (RFC 8410
    // §3), with nothing accepted in its place.
    match (algorithm, key.algorithm.parameters) {
        (SignatureAlgorithm::Ed25519, Some(_)) => Err(VerifyError::MalformedParameters),
        (_, None) => Ok(None),
        (_, Some(bytes)) => {
            let mut reader = Reader::new(bytes);
            reader
                .read_null()
                .map_err(|_| VerifyError::MalformedParameters)?;
            reader
                .finish()
                .map_err(|_| VerifyError::MalformedParameters)?;
            Ok(None)
        }
    }
}

/// Verify `signature` over `message`, under `key`, using `algorithm`.
///
/// The general form, separate from certificates so that anything else signed
/// by a certificate's key — a CRL, an OCSP response — can reuse it without
/// this module growing a second copy of the algorithm table.
pub fn verify_signed_data(
    algorithm: &AlgorithmIdentifier<'_>,
    key: &SubjectPublicKeyInfo<'_>,
    message: &[u8],
    signature: &[u8],
) -> Result<(), VerifyError> {
    let algorithm = SignatureAlgorithm::from_identifier(algorithm)?;
    let curve = check_key_compatibility(algorithm, key)?;

    signature::UnparsedPublicKey::new(algorithm.ring_algorithm(curve), key.key)
        .verify(message, signature)
        .map_err(|_| VerifyError::BadSignature)
}

/// Verify that `certificate` was signed by the key matching `issuer_key`.
///
/// The message is [`Certificate::tbs_der`] — the exact bytes parsed off the
/// wire, never a re-encoding. That is why [`super::x509`] keeps them as a
/// borrow: a signature verified over re-encoded bytes proves something about
/// the re-encoding, not about the certificate that arrived.
///
/// # This proves authorship, not authority
///
/// A successful return means the holder of `issuer_key`'s private key signed
/// this certificate. It does **not** mean:
///
/// - that `issuer_key` belongs to anyone trustworthy,
/// - that its owner was permitted to issue certificates (`basicConstraints`,
///   `keyUsage`, path length — none of it is looked at here),
/// - that the certificate is currently valid (no clock is read),
/// - that it is valid for any particular name.
///
/// Those are stage 2b-ii. Calling this alone and treating the result as a
/// trust decision would accept any certificate an attacker generated, because
/// an attacker can sign their own.
pub fn verify_signature(
    certificate: &Certificate<'_>,
    issuer_key: &SubjectPublicKeyInfo<'_>,
) -> Result<(), VerifyError> {
    verify_signed_data(
        &certificate.signature_algorithm(),
        issuer_key,
        certificate.tbs_der(),
        certificate.signature(),
    )
}
