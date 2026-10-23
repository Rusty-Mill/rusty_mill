//! The cryptographic primitives the client path uses, behind one seam.
//!
//! Every hash, HMAC, AEAD, key exchange, signature check and random fill that
//! a client handshake or its records perform goes through this module and
//! nowhere else. The backend that provides the primitives can then change in
//! one place, and a reader can see in one place exactly what the engine asks
//! of its cryptography.
//!
//! # Backends
//!
//! - [`ring`] (the default): `ring`, per ADR-0002 §6.
//! - `native`, with the `native-crypto` feature **and**
//!   `--cfg rusty_tls_native_crypto` (ADR-0005): the first-party crates
//!   `rusty_sha2`, `rusty_pk`, `rusty_aead` and `rusty_rand`. The cfg is what
//!   selects it; the feature only makes it available, so a dependency cannot
//!   switch it on for anyone else (the ADR-0002 §1 construction).
//!
//! Both implement the same items — [`digest`], [`HmacKey`] (and its context),
//! [`AeadKey`], [`EphemeralKey`], [`Verifier::verify`] and [`fill_random`] —
//! and the selected one is re-exported here. Callers never name a backend.
//!
//! # What is not here
//!
//! Signing and the server's session-ticket cipher. Both are server-role (or
//! client-certificate) operations, which the native backend does not cover
//! (gap 5 of the plan is closed). [`super::sign`] and [`super::ticket`] keep
//! calling `ring` directly, and under the native cfg they are compiled out.
//!
//! # Errors
//!
//! [`CryptoError`] carries nothing, on purpose: every primitive's failure is
//! opaque at this level, and each caller already maps it to the error that
//! means something in its own context.

use core::fmt;

#[cfg(any(not(rusty_tls_native_crypto), test))]
#[cfg_attr(rusty_tls_native_crypto, allow(dead_code))]
mod ring;

#[cfg(rusty_tls_native_crypto)]
mod native;

#[cfg(not(rusty_tls_native_crypto))]
use self::ring as backend;

#[cfg(rusty_tls_native_crypto)]
use self::native as backend;

pub(crate) use backend::{digest, fill_random, AeadKey, EphemeralKey, HmacKey};

/// A primitive failed. Deliberately says nothing more — see the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CryptoError;

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cryptographic operation failed")
    }
}

impl std::error::Error for CryptoError {}

/// The two hashes TLS cipher suites in this engine use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HashAlgorithm {
    Sha256,
    Sha384,
}

/// The record-protection AEADs, all with a 12-octet nonce and 16-octet tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AeadAlgorithm {
    Aes128Gcm,
    Aes256Gcm,
    ChaCha20Poly1305,
}

impl AeadAlgorithm {
    /// The key length, which [`AeadKey::new`] checks. Used by the seam
    /// differential, which exists only on the native backend.
    #[cfg(all(test, rusty_tls_native_crypto))]
    pub(crate) const fn key_len(self) -> usize {
        match self {
            Self::Aes128Gcm => 16,
            Self::Aes256Gcm | Self::ChaCha20Poly1305 => 32,
        }
    }
}

/// The AEAD nonce length every algorithm here uses.
pub(crate) const AEAD_NONCE_LEN: usize = 12;

/// The AEAD tag length every algorithm here uses.
#[cfg(rusty_tls_native_crypto)]
pub(crate) const AEAD_TAG_LEN: usize = 16;

/// The ephemeral key-exchange groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KxAlgorithm {
    P256,
    P384,
    X25519,
}

/// The signature checks the engine performs: exactly these, and no others.
///
/// RSA moduli are bounded to 2048–8192 bits by every RSA variant, and the PSS
/// variants fix MGF1 with the same hash and a salt as long as the digest. Both
/// backends implement exactly these rules (`CRYPTO-REPLACEMENT-PLAN.md` §0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verifier {
    RsaPkcs1Sha256,
    RsaPkcs1Sha384,
    RsaPkcs1Sha512,
    RsaPssSha256,
    RsaPssSha384,
    RsaPssSha512,
    EcdsaP256Sha256,
    EcdsaP256Sha384,
    EcdsaP384Sha256,
    EcdsaP384Sha384,
    Ed25519,
}

impl Verifier {
    /// Check `signature` over `message` under `public_key`, which is the
    /// `subjectPublicKey` bit string's contents (a DER `RSAPublicKey`, an
    /// uncompressed SEC1 point, or 32 raw Ed25519 octets).
    pub(crate) fn verify(
        self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), CryptoError> {
        backend::verify(self, public_key, message, signature)
    }
}
