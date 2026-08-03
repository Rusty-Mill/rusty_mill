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
//! # Two namespaces, and why they are not one type
//!
//! This module verifies signatures in two different worlds, and stage 3c-i
//! added the second:
//!
//! - [`SignatureAlgorithm`] — an X.509 `AlgorithmIdentifier`, an OID plus
//!   parameters, naming how a *certificate* was signed.
//! - [`SignatureScheme`] — a TLS `SignatureScheme`, a `uint16`, naming how a
//!   *handshake* was signed (RFC 8446 §4.2.3).
//!
//! They overlap in what they can express and disagree on almost every rule,
//! so they are separate types with separate tables. Three disagreements are
//! worth stating outright, because each is a way to be wrong:
//!
//! | Question | X.509 | TLS 1.3 |
//! | --- | --- | --- |
//! | Where does an ECDSA key's curve come from? | the **key** | the **scheme** |
//! | Is RSA PKCS#1 v1.5 acceptable? | yes | **no**, PSS only |
//! | Is RSASSA-PSS acceptable? | not implemented | **required** for RSA |
//!
//! The first row is the one that bites. Stage 2b-i learned the hard way that
//! `ecdsa-with-SHA256` names a hash and says nothing about the curve — a
//! P-384 key signed with SHA-256 is conforming, and three roots in this
//! machine's trust store are exactly that. TLS 1.3 then inverts it:
//! `ecdsa_secp256r1_sha256` names *both*, so a P-384 key under that scheme is
//! invalid and must be refused. Same-looking question, opposite answers —
//! which is precisely why a shared "signature algorithm" type would be a trap.
//!
//! # Algorithms, and the ones deliberately refused
//!
//! For certificates ([`SignatureAlgorithm`]):
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
//! For handshakes ([`SignatureScheme`]):
//!
//! | Scheme | Status |
//! | --- | --- |
//! | `rsa_pss_rsae_sha256/384/512` | supported |
//! | `ecdsa_secp256r1_sha256`, `ecdsa_secp384r1_sha384` | supported |
//! | `ed25519` | supported |
//! | `rsa_pkcs1_*` | **refused** — certificates only, RFC 8446 §4.4.3 |
//! | `rsa_pkcs1_sha1`, `ecdsa_sha1` | **refused** as weak |
//! | `ecdsa_secp521r1_sha512`, `ed448`, `rsa_pss_pss_*` | **refused** as unsupported |
//!
//! PSS being refused in one column and required in the other is not an
//! inconsistency. In X.509 the PSS *parameters* are a DER structure carrying
//! the hash, the mask generation function, the salt length, and a trailer
//! field, any of which can be got wrong in a way that verifies something other
//! than what was signed — so 2b-i failed closed. A TLS `SignatureScheme` is a
//! single number that fixes all four. There is nothing left to misparse, which
//! is why the safe answer differs.
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
    /// A TLS [`SignatureScheme`] named a curve, and the key is on a different
    /// one.
    ///
    /// Distinct from [`VerifyError::UnsupportedCurve`]: both curves may be
    /// perfectly supported, and the scheme still does not describe this key.
    /// Only reachable through the TLS namespace — X.509 reads the curve off
    /// the key, so there is nothing there for it to disagree with. See the
    /// module docs.
    CurveMismatch,
    /// A TLS [`SignatureScheme`] that RFC 8446 §4.4.3 permits in certificates
    /// but forbids in a handshake signature — the `rsa_pkcs1_*` family.
    ///
    /// Its own variant rather than
    /// [`VerifyError::UnsupportedSignatureAlgorithm`], because this is not a
    /// gap: the algorithm is implemented, and is being refused *in this
    /// position* on the RFC's instruction.
    CertificateOnlyScheme,
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
            Self::CurveMismatch => {
                f.write_str("the signature scheme names a curve the key is not on")
            }
            Self::CertificateOnlyScheme => {
                f.write_str("this signature scheme is permitted in certificates but not in a TLS 1.3 handshake signature")
            }
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
            Self::RsaPkcs1Sha256 | Self::RsaPkcs1Sha384 | Self::RsaPkcs1Sha512 => {
                require_null_or_absent_parameters(identifier)?;
            }
            Self::EcdsaSha256 | Self::EcdsaSha384 | Self::Ed25519 => {
                require_absent_parameters(identifier)?;
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
        return named_curve(key).map(Some);
    }

    // RSA: NULL or absent. Ed25519: absent, with nothing accepted in its
    // place.
    match algorithm {
        SignatureAlgorithm::Ed25519 => require_absent_parameters(&key.algorithm)?,
        _ => require_null_or_absent_parameters(&key.algorithm)?,
    }
    Ok(None)
}

/// Require an `AlgorithmIdentifier`'s parameters to be `NULL` or absent.
///
/// RFC 4055 §1.2 says `NULL` for RSA. Absent is accepted too: some issuers
/// omit it, the meaning is not in dispute, and refusing would turn away
/// certificates that verify correctly everywhere else. Anything else is
/// refused, because then the meaning genuinely is in dispute.
fn require_null_or_absent_parameters(
    identifier: &AlgorithmIdentifier<'_>,
) -> Result<(), VerifyError> {
    let Some(bytes) = identifier.parameters else {
        return Ok(());
    };
    let mut reader = Reader::new(bytes);
    reader
        .read_null()
        .map_err(|_| VerifyError::MalformedParameters)?;
    reader
        .finish()
        .map_err(|_| VerifyError::MalformedParameters)
}

/// Require an `AlgorithmIdentifier` to carry no parameters at all.
///
/// RFC 8410 §3 for Ed25519 and RFC 5758 §3.2 for ECDSA signature algorithms:
/// absent, full stop. A `NULL` here is the classic mistake — it looks like
/// "no parameters" and is a different encoding, so tolerating it would mean
/// accepting a key nobody conforming produces.
fn require_absent_parameters(identifier: &AlgorithmIdentifier<'_>) -> Result<(), VerifyError> {
    if identifier.parameters.is_some() {
        return Err(VerifyError::MalformedParameters);
    }
    Ok(())
}

/// Read the curve an EC key is on, from the key's own parameters.
///
/// RFC 5480 §2.1.1: an `id-ecPublicKey` key's `AlgorithmIdentifier`
/// parameters name the curve. Both namespaces need this and want opposite
/// things from it — X.509 *takes* the curve from here, TLS *checks* it
/// against the scheme — so it is one function with one reading of the DER.
fn named_curve(key: &SubjectPublicKeyInfo<'_>) -> Result<Curve, VerifyError> {
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

    match named {
        oid::P256 => Ok(Curve::P256),
        oid::P384 => Ok(Curve::P384),
        // P-521, secp256k1, an explicit curve specification — all refused.
        // Verifying over a curve this module does not know is not something
        // to attempt.
        _ => Err(VerifyError::UnsupportedCurve),
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

// ---------------------------------------------------------------------------
// The TLS namespace — stage 3c-i
// ---------------------------------------------------------------------------

/// A TLS `SignatureScheme` (RFC 8446 §4.2.3).
///
/// A newtype over the wire value rather than an enum, because the registry is
/// open: a peer may offer anything, and a client has to be able to hold a
/// number it does not recognise long enough to refuse it. What it *means* is
/// decided by [`SignatureScheme::tls13_algorithm`], which recognises a fixed
/// set and refuses the rest.
///
/// See the module docs for how this differs from [`SignatureAlgorithm`], which
/// is the same idea for certificates and follows different rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignatureScheme(pub u16);

impl SignatureScheme {
    /// `rsa_pkcs1_sha256(0x0401)` — certificates only.
    pub const RSA_PKCS1_SHA256: Self = Self(0x0401);
    /// `rsa_pkcs1_sha384(0x0501)` — certificates only.
    pub const RSA_PKCS1_SHA384: Self = Self(0x0501);
    /// `rsa_pkcs1_sha512(0x0601)` — certificates only.
    pub const RSA_PKCS1_SHA512: Self = Self(0x0601);

    /// `ecdsa_secp256r1_sha256(0x0403)`.
    pub const ECDSA_SECP256R1_SHA256: Self = Self(0x0403);
    /// `ecdsa_secp384r1_sha384(0x0503)`.
    pub const ECDSA_SECP384R1_SHA384: Self = Self(0x0503);
    /// `ecdsa_secp521r1_sha512(0x0603)` — P-521 is not implemented here.
    pub const ECDSA_SECP521R1_SHA512: Self = Self(0x0603);

    /// `rsa_pss_rsae_sha256(0x0804)` — what an RSA server actually signs a
    /// TLS 1.3 handshake with.
    pub const RSA_PSS_RSAE_SHA256: Self = Self(0x0804);
    /// `rsa_pss_rsae_sha384(0x0805)`.
    pub const RSA_PSS_RSAE_SHA384: Self = Self(0x0805);
    /// `rsa_pss_rsae_sha512(0x0806)`.
    pub const RSA_PSS_RSAE_SHA512: Self = Self(0x0806);

    /// `ed25519(0x0807)`.
    pub const ED25519: Self = Self(0x0807);
    /// `ed448(0x0808)` — not implemented.
    pub const ED448: Self = Self(0x0808);

    /// `rsa_pss_pss_sha256(0x0809)` — needs an `id-RSASSA-PSS` key, which the
    /// X.509 parser does not produce.
    pub const RSA_PSS_PSS_SHA256: Self = Self(0x0809);
    /// `rsa_pss_pss_sha384(0x080a)`.
    pub const RSA_PSS_PSS_SHA384: Self = Self(0x080a);
    /// `rsa_pss_pss_sha512(0x080b)`.
    pub const RSA_PSS_PSS_SHA512: Self = Self(0x080b);

    /// `rsa_pkcs1_sha1(0x0201)` — legacy, refused.
    pub const RSA_PKCS1_SHA1: Self = Self(0x0201);
    /// `ecdsa_sha1(0x0203)` — legacy, refused.
    pub const ECDSA_SHA1: Self = Self(0x0203);

    /// The schemes this module will verify a TLS 1.3 handshake signature
    /// with, in the order a client should offer them.
    ///
    /// Exposed so that the `signature_algorithms` extension a client sends and
    /// the set it will actually accept cannot drift apart — offering a scheme
    /// that would then be refused invites a server to pick it and fail the
    /// handshake for no reason.
    pub const TLS13_SUPPORTED: &'static [Self] = &[
        Self::ECDSA_SECP256R1_SHA256,
        Self::ECDSA_SECP384R1_SHA384,
        Self::ED25519,
        Self::RSA_PSS_RSAE_SHA256,
        Self::RSA_PSS_RSAE_SHA384,
        Self::RSA_PSS_RSAE_SHA512,
    ];

    /// Resolve this scheme for use in a TLS 1.3 handshake signature, given
    /// the key it will be checked against.
    ///
    /// `key` is needed because two of the rules involve it: an ECDSA scheme
    /// names a curve the key must actually be on, and every scheme names a key
    /// type. Refusals are as specific as the reason — a `rsa_pkcs1_*` scheme
    /// is [`VerifyError::CertificateOnlyScheme`] rather than "unsupported",
    /// because it is implemented and being turned away on the RFC's
    /// instruction.
    fn tls13_algorithm(
        self,
        key: &SubjectPublicKeyInfo<'_>,
    ) -> Result<&'static dyn signature::VerificationAlgorithm, VerifyError> {
        // Weak first, so a SHA-1 scheme is never reported as merely
        // unsupported — it is refused on strength, and that distinction is
        // the same one certificates make.
        match self {
            Self::RSA_PKCS1_SHA1 => {
                return Err(VerifyError::WeakSignatureAlgorithm("rsa_pkcs1_sha1"))
            }
            Self::ECDSA_SHA1 => return Err(VerifyError::WeakSignatureAlgorithm("ecdsa_sha1")),

            // RFC 8446 §4.4.3: "RSA signatures MUST use an RSASSA-PSS
            // algorithm, regardless of whether RSASSA-PKCS1-v1_5 algorithms
            // appear in 'signature_algorithms'." §4.2.3 lists these as
            // defined for use in certificates only. Accepting one here would
            // accept a signature the RFC says a conforming peer never sends,
            // which is a downgrade in everything but name.
            Self::RSA_PKCS1_SHA256 | Self::RSA_PKCS1_SHA384 | Self::RSA_PKCS1_SHA512 => {
                return Err(VerifyError::CertificateOnlyScheme)
            }

            _ => {}
        }

        let required_key = match self {
            Self::RSA_PSS_RSAE_SHA256 | Self::RSA_PSS_RSAE_SHA384 | Self::RSA_PSS_RSAE_SHA512 => {
                oid::RSA_ENCRYPTION
            }
            Self::ECDSA_SECP256R1_SHA256 | Self::ECDSA_SECP384R1_SHA384 => oid::EC_PUBLIC_KEY,
            Self::ED25519 => oid::ED25519,
            _ => return Err(VerifyError::UnsupportedSignatureAlgorithm),
        };

        if key.algorithm.oid != required_key {
            return Err(match key.algorithm.oid {
                oid::RSA_ENCRYPTION | oid::EC_PUBLIC_KEY | oid::ED25519 => {
                    VerifyError::KeyAlgorithmMismatch
                }
                _ => VerifyError::UnsupportedKeyAlgorithm,
            });
        }

        // The key's own parameters, by the same rules the X.509 side applies —
        // the namespaces disagree about signature algorithms, not about how a
        // `SubjectPublicKeyInfo` is encoded. Checking here and not there would
        // leave a leaf's key held to a lower standard than its issuer's, for
        // no reason anyone could state.
        match self {
            Self::RSA_PSS_RSAE_SHA256 | Self::RSA_PSS_RSAE_SHA384 | Self::RSA_PSS_RSAE_SHA512 => {
                require_null_or_absent_parameters(&key.algorithm)?;
            }
            Self::ED25519 => require_absent_parameters(&key.algorithm)?,
            // EC keys are the exception: their parameters are *required*,
            // because that is where the curve is. `named_curve` enforces it.
            _ => {}
        }

        match self {
            // `ring`'s PSS verifiers fix MGF1 with the same hash and a salt
            // length equal to the digest length, which is what RFC 8446
            // §4.2.3 requires of these schemes. They also enforce a 2048–8192
            // bit modulus, so an undersized RSA key is refused without a
            // check here — including RFC 8448's own 1024-bit example key.
            Self::RSA_PSS_RSAE_SHA256 => Ok(&signature::RSA_PSS_2048_8192_SHA256),
            Self::RSA_PSS_RSAE_SHA384 => Ok(&signature::RSA_PSS_2048_8192_SHA384),
            Self::RSA_PSS_RSAE_SHA512 => Ok(&signature::RSA_PSS_2048_8192_SHA512),

            // The scheme names the curve, so the key has to be on it. This is
            // the inverse of the X.509 rule and the reason these are two
            // types — see the module docs.
            Self::ECDSA_SECP256R1_SHA256 => match named_curve(key)? {
                Curve::P256 => Ok(&signature::ECDSA_P256_SHA256_ASN1),
                Curve::P384 => Err(VerifyError::CurveMismatch),
            },
            Self::ECDSA_SECP384R1_SHA384 => match named_curve(key)? {
                Curve::P384 => Ok(&signature::ECDSA_P384_SHA384_ASN1),
                Curve::P256 => Err(VerifyError::CurveMismatch),
            },

            Self::ED25519 => Ok(&signature::ED25519),

            // Unreachable: `required_key` above returns for everything else.
            _ => Err(VerifyError::UnsupportedSignatureAlgorithm),
        }
    }
}

/// Verify a TLS 1.3 handshake signature — a CertificateVerify.
///
/// `message` is the blob RFC 8446 §4.4.3 defines, which
/// [`super::handshake::certificate_verify_content`] builds: 64 octets of
/// `0x20`, a context string, `0x00`, then the transcript hash. Composing the
/// two is the caller's job, deliberately — this module knows how to check a
/// signature and nothing about what a handshake looks like, and
/// [`super::handshake`] parses messages and verifies nothing. Stage 3c-ii is
/// where they meet.
///
/// # This proves authorship, not authority — again
///
/// The same caveat as [`verify_signature`], and it is easier to lose here. A
/// CertificateVerify that checks out proves the peer holds the private key for
/// the certificate it sent. It says nothing about whether that certificate
/// chains to a trust anchor ([`super::path`]) or is valid for the name that
/// was asked for ([`super::name`]). All three are required, and any one of
/// them alone accepts an attacker who generated their own certificate.
pub fn verify_tls13_signature(
    scheme: SignatureScheme,
    key: &SubjectPublicKeyInfo<'_>,
    message: &[u8],
    signature: &[u8],
) -> Result<(), VerifyError> {
    let algorithm = scheme.tls13_algorithm(key)?;
    signature::UnparsedPublicKey::new(algorithm, key.key)
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
