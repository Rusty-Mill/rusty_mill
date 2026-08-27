//! Producing handshake signatures — stage 5.
//!
//! The inverse of [`super::verify`]'s TLS half, and the first thing in this
//! module that holds a private key.
//!
//! # Why this is a separate module
//!
//! Everything before stage 5 only ever *checked* a signature. Checking is done
//! with public material and a wrong answer is a failure to authenticate;
//! signing is done with secret material and a wrong answer can be a signature
//! over something the holder never meant to endorse. They are different risks,
//! so they live in different files rather than in one module called
//! "signatures".
//!
//! # What a signing key will not do
//!
//! - **It will not hand back the key.** There is no accessor, no `Deref`, and
//!   [`SigningKey`]'s `Debug` says only what algorithm it is. `ring`'s key
//!   types do not expose their private scalars either, so this is not the only
//!   thing standing between a key and a log — but it is the part this crate
//!   owns, and inheriting the absence of a leak is not the same as promising
//!   there isn't one.
//! - **It will not choose a scheme for you.** [`SigningKey::sign`] takes the
//!   scheme and refuses anything the key cannot do, rather than silently
//!   picking. A server that signed with a scheme the client did not offer
//!   would produce a signature the client is obliged to reject, and the
//!   failure would surface three messages later as a handshake error.
//! - **It will not sign a bare hash.** The caller passes the full message —
//!   for a CertificateVerify that is
//!   [`super::handshake::certificate_verify_content`], padding and context
//!   string included. A function here that took a transcript hash and added
//!   the framing itself would be one refactor away from signing a bare hash,
//!   which is exactly the cross-protocol reuse the framing exists to prevent.

use ring::rand::SystemRandom;
use ring::signature::{self, KeyPair};

use super::verify::SignatureScheme;

/// Why a key could not be loaded, or a signature could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignError {
    /// The PKCS#8 document was rejected — malformed, the wrong algorithm, or
    /// a key size `ring` refuses (RSA below 2048 bits, for instance).
    BadKey,
    /// The scheme is not one this key can sign with.
    ///
    /// A P-256 key cannot produce an `ecdsa_secp384r1_sha384` signature, and
    /// an RSA key cannot produce an Ed25519 one. Refused rather than
    /// substituted.
    UnsupportedScheme(SignatureScheme),
    /// Signing failed inside `ring`, which in practice means the random source
    /// did.
    Failed,
}

impl core::fmt::Display for SignError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadKey => f.write_str("the private key was rejected"),
            Self::UnsupportedScheme(scheme) => {
                write!(f, "this key cannot sign with scheme 0x{:04x}", scheme.0)
            }
            Self::Failed => f.write_str("signing failed"),
        }
    }
}

impl std::error::Error for SignError {}

enum Inner {
    Ecdsa {
        pair: signature::EcdsaKeyPair,
        scheme: SignatureScheme,
    },
    Ed25519(signature::Ed25519KeyPair),
    Rsa(signature::RsaKeyPair),
}

/// A private key that can sign a TLS 1.3 handshake.
///
/// Constructed from a PKCS#8 document with the algorithm named explicitly —
/// there is no constructor that sniffs the document and decides for you. A
/// server that guessed wrong about its own key would find out at handshake
/// time, on a connection it could not complete.
pub struct SigningKey {
    inner: Inner,
}

impl SigningKey {
    /// An ECDSA P-256 key, which signs `ecdsa_secp256r1_sha256`.
    pub fn ecdsa_p256(pkcs8: &[u8]) -> Result<Self, SignError> {
        Self::ecdsa(
            &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            pkcs8,
            SignatureScheme::ECDSA_SECP256R1_SHA256,
        )
    }

    /// An ECDSA P-384 key, which signs `ecdsa_secp384r1_sha384`.
    pub fn ecdsa_p384(pkcs8: &[u8]) -> Result<Self, SignError> {
        Self::ecdsa(
            &signature::ECDSA_P384_SHA384_ASN1_SIGNING,
            pkcs8,
            SignatureScheme::ECDSA_SECP384R1_SHA384,
        )
    }

    fn ecdsa(
        algorithm: &'static signature::EcdsaSigningAlgorithm,
        pkcs8: &[u8],
        scheme: SignatureScheme,
    ) -> Result<Self, SignError> {
        let rng = SystemRandom::new();
        let pair = signature::EcdsaKeyPair::from_pkcs8(algorithm, pkcs8, &rng)
            .map_err(|_| SignError::BadKey)?;
        Ok(Self {
            inner: Inner::Ecdsa { pair, scheme },
        })
    }

    /// An Ed25519 key.
    pub fn ed25519(pkcs8: &[u8]) -> Result<Self, SignError> {
        // `maybe_unchecked` because PKCS#8 v1 Ed25519 documents omit the
        // public key, and plenty of tooling emits them. The alternative is
        // refusing keys that are perfectly valid.
        let pair = signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(pkcs8)
            .map_err(|_| SignError::BadKey)?;
        Ok(Self {
            inner: Inner::Ed25519(pair),
        })
    }

    /// An RSA key, which signs any of the three `rsa_pss_rsae_*` schemes.
    ///
    /// `ring` enforces a 2048–8192 bit modulus, so a key below that is
    /// [`SignError::BadKey`] here rather than a signature nothing will accept.
    pub fn rsa(pkcs8: &[u8]) -> Result<Self, SignError> {
        let pair = signature::RsaKeyPair::from_pkcs8(pkcs8).map_err(|_| SignError::BadKey)?;
        Ok(Self {
            inner: Inner::Rsa(pair),
        })
    }

    /// The schemes this key can sign with, most preferred first.
    ///
    /// A server intersects this with the client's `signature_algorithms` and
    /// takes the first survivor. Offering the intersection rather than a fixed
    /// choice is what stops a server from signing with something the client
    /// told it not to.
    pub fn schemes(&self) -> &'static [SignatureScheme] {
        match &self.inner {
            Inner::Ecdsa { scheme, .. } => match *scheme {
                SignatureScheme::ECDSA_SECP384R1_SHA384 => {
                    &[SignatureScheme::ECDSA_SECP384R1_SHA384]
                }
                _ => &[SignatureScheme::ECDSA_SECP256R1_SHA256],
            },
            Inner::Ed25519(_) => &[SignatureScheme::ED25519],
            Inner::Rsa(_) => &[
                SignatureScheme::RSA_PSS_RSAE_SHA256,
                SignatureScheme::RSA_PSS_RSAE_SHA384,
                SignatureScheme::RSA_PSS_RSAE_SHA512,
            ],
        }
    }

    /// The `SubjectPublicKeyInfo`-independent public key bytes, for checking
    /// that a key and a certificate belong together.
    ///
    /// Returned so a caller can compare against a parsed certificate's key
    /// rather than discovering the mismatch when a peer refuses the signature.
    pub fn public_key(&self) -> Vec<u8> {
        match &self.inner {
            Inner::Ecdsa { pair, .. } => pair.public_key().as_ref().to_vec(),
            Inner::Ed25519(pair) => pair.public_key().as_ref().to_vec(),
            Inner::Rsa(pair) => pair.public_key().as_ref().to_vec(),
        }
    }

    /// Sign `message` with `scheme`.
    ///
    /// `message` is the whole thing to be signed, not a hash of it — see the
    /// module docs on why there is no convenience form that takes a transcript
    /// hash.
    pub fn sign(&self, scheme: SignatureScheme, message: &[u8]) -> Result<Vec<u8>, SignError> {
        if !self.schemes().contains(&scheme) {
            return Err(SignError::UnsupportedScheme(scheme));
        }
        let rng = SystemRandom::new();

        match &self.inner {
            Inner::Ecdsa { pair, .. } => Ok(pair
                .sign(&rng, message)
                .map_err(|_| SignError::Failed)?
                .as_ref()
                .to_vec()),
            Inner::Ed25519(pair) => Ok(pair.sign(message).as_ref().to_vec()),
            Inner::Rsa(pair) => {
                let padding: &dyn signature::RsaEncoding = match scheme {
                    SignatureScheme::RSA_PSS_RSAE_SHA384 => &signature::RSA_PSS_SHA384,
                    SignatureScheme::RSA_PSS_RSAE_SHA512 => &signature::RSA_PSS_SHA512,
                    _ => &signature::RSA_PSS_SHA256,
                };
                let mut out = vec![0u8; pair.public().modulus_len()];
                pair.sign(padding, &rng, message, &mut out)
                    .map_err(|_| SignError::Failed)?;
                Ok(out)
            }
        }
    }
}

/// Says what kind of key it is and nothing else.
impl core::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let kind = match &self.inner {
            Inner::Ecdsa { scheme, .. } => {
                if *scheme == SignatureScheme::ECDSA_SECP384R1_SHA384 {
                    "ECDSA P-384"
                } else {
                    "ECDSA P-256"
                }
            }
            Inner::Ed25519(_) => "Ed25519",
            Inner::Rsa(_) => "RSA",
        };
        f.debug_struct("SigningKey")
            .field("algorithm", &kind)
            .field("private", &"<redacted>")
            .finish()
    }
}
