//! The cryptographic primitives the client path uses, behind one seam.
//!
//! Every hash, HMAC, AEAD, key exchange, signature check and random fill that
//! a client handshake or its records perform goes through this module and
//! nowhere else. That is the whole point of it: the backend that provides the
//! primitives can then change in one file instead of ten, and a reader can see
//! in one place exactly what the engine asks of its cryptography.
//!
//! Today there is one backend, `ring`, per ADR-0002 §6, and nothing about the
//! engine's behaviour depends on this module existing — it is the same calls,
//! moved. `docs/research/TLS-NATIVE-CRYPTO-SEAM.md` is the plan for a second,
//! first-party backend behind its own cfg; until that lands, `ring` is the
//! only implementation and is not selectable away.
//!
//! # What is not here
//!
//! Signing and the server's session-ticket cipher. Both are server-role (or
//! client-certificate) operations, which the native backend will not cover
//! (gap 5 of the plan is closed), so [`super::sign`] and [`super::ticket`]
//! keep calling `ring` directly. Routing them through here would suggest a
//! second backend could serve them, which it cannot.
//!
//! # Errors
//!
//! [`CryptoError`] carries nothing, on purpose: every primitive's failure is
//! opaque at this level (as `ring`'s are), and each caller already maps it to
//! the error that means something in its own context.

use core::fmt;

use ring::{aead, agreement, digest, hmac, rand, signature};

use ring::rand::SecureRandom;

/// A primitive failed. Deliberately says nothing more — see the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CryptoError;

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cryptographic operation failed")
    }
}

impl std::error::Error for CryptoError {}

// ---------------------------------------------------------------------------
// Hashes and HMAC
// ---------------------------------------------------------------------------

/// The two hashes TLS cipher suites in this engine use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HashAlgorithm {
    Sha256,
    Sha384,
}

impl HashAlgorithm {
    fn digest(self) -> &'static digest::Algorithm {
        match self {
            Self::Sha256 => &digest::SHA256,
            Self::Sha384 => &digest::SHA384,
        }
    }

    fn hmac(self) -> hmac::Algorithm {
        match self {
            Self::Sha256 => hmac::HMAC_SHA256,
            Self::Sha384 => hmac::HMAC_SHA384,
        }
    }
}

/// `alg(message)`.
pub(crate) fn digest(alg: HashAlgorithm, message: &[u8]) -> Vec<u8> {
    digest::digest(alg.digest(), message).as_ref().to_vec()
}

/// An HMAC key.
pub(crate) struct HmacKey(hmac::Key);

impl HmacKey {
    pub(crate) fn new(alg: HashAlgorithm, key: &[u8]) -> Self {
        Self(hmac::Key::new(alg.hmac(), key))
    }

    /// `HMAC(key, message)`.
    pub(crate) fn sign(&self, message: &[u8]) -> Vec<u8> {
        hmac::sign(&self.0, message).as_ref().to_vec()
    }

    /// Whether `tag` is `HMAC(key, message)`, compared in constant time.
    ///
    /// `tag` must be the full-length MAC; a truncated tag is a mismatch.
    pub(crate) fn verify(&self, message: &[u8], tag: &[u8]) -> bool {
        hmac::verify(&self.0, message, tag).is_ok()
    }

    /// An incremental HMAC under this key, for a message built from pieces.
    pub(crate) fn context(&self) -> HmacContext {
        HmacContext(hmac::Context::with_key(&self.0))
    }
}

/// An HMAC being computed over a message supplied in pieces.
pub(crate) struct HmacContext(hmac::Context);

impl HmacContext {
    pub(crate) fn update(&mut self, piece: &[u8]) {
        self.0.update(piece);
    }

    pub(crate) fn sign(self) -> Vec<u8> {
        self.0.sign().as_ref().to_vec()
    }
}

// ---------------------------------------------------------------------------
// AEAD
// ---------------------------------------------------------------------------

/// The record-protection AEADs, all with a 12-octet nonce and 16-octet tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AeadAlgorithm {
    Aes128Gcm,
    Aes256Gcm,
    ChaCha20Poly1305,
}

impl AeadAlgorithm {
    fn ring(self) -> &'static aead::Algorithm {
        match self {
            Self::Aes128Gcm => &aead::AES_128_GCM,
            Self::Aes256Gcm => &aead::AES_256_GCM,
            Self::ChaCha20Poly1305 => &aead::CHACHA20_POLY1305,
        }
    }
}

/// The AEAD nonce length every algorithm here uses.
pub(crate) const AEAD_NONCE_LEN: usize = aead::NONCE_LEN;

/// One direction's AEAD key.
///
/// Nonce uniqueness is the caller's contract: the record layers derive every
/// nonce from a sequence number that never repeats for a key.
pub(crate) struct AeadKey(aead::LessSafeKey);

impl AeadKey {
    /// Fails only when `key` is the wrong length for `alg`.
    pub(crate) fn new(alg: AeadAlgorithm, key: &[u8]) -> Result<Self, CryptoError> {
        let unbound = aead::UnboundKey::new(alg.ring(), key).map_err(|_| CryptoError)?;
        Ok(Self(aead::LessSafeKey::new(unbound)))
    }

    /// Encrypt `in_out` in place and append the tag.
    pub(crate) fn seal_in_place_append_tag(
        &self,
        nonce: [u8; AEAD_NONCE_LEN],
        aad: &[u8],
        in_out: &mut Vec<u8>,
    ) -> Result<(), CryptoError> {
        self.0
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(aad),
                in_out,
            )
            .map_err(|_| CryptoError)
    }

    /// Check the tag at the end of `in_out` and decrypt in place, returning
    /// the plaintext prefix. On failure the contents of `in_out` are
    /// unspecified and must not be used.
    pub(crate) fn open_in_place<'a>(
        &self,
        nonce: [u8; AEAD_NONCE_LEN],
        aad: &[u8],
        in_out: &'a mut [u8],
    ) -> Result<&'a mut [u8], CryptoError> {
        self.0
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(aad),
                in_out,
            )
            .map_err(|_| CryptoError)
    }
}

// ---------------------------------------------------------------------------
// Key exchange
// ---------------------------------------------------------------------------

/// The ephemeral key-exchange groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KxAlgorithm {
    P256,
    P384,
    X25519,
}

impl KxAlgorithm {
    fn ring(self) -> &'static agreement::Algorithm {
        match self {
            Self::P256 => &agreement::ECDH_P256,
            Self::P384 => &agreement::ECDH_P384,
            Self::X25519 => &agreement::X25519,
        }
    }
}

/// A single-use ephemeral private key.
pub(crate) struct EphemeralKey {
    alg: KxAlgorithm,
    private: agreement::EphemeralPrivateKey,
}

impl EphemeralKey {
    /// Generate a key from the system random source. Fails only if that
    /// source fails.
    pub(crate) fn generate(alg: KxAlgorithm) -> Result<Self, CryptoError> {
        let private =
            agreement::EphemeralPrivateKey::generate(alg.ring(), &rand::SystemRandom::new())
                .map_err(|_| CryptoError)?;
        Ok(Self { alg, private })
    }

    /// The public key: 32 raw octets for X25519, an uncompressed SEC1 point
    /// for the NIST curves.
    pub(crate) fn public_key(&self) -> Result<Vec<u8>, CryptoError> {
        Ok(self
            .private
            .compute_public_key()
            .map_err(|_| CryptoError)?
            .as_ref()
            .to_vec())
    }

    /// Agree a secret with `peer` and hand it to `f`. The backend refuses a
    /// peer key of the wrong length, a NIST point off the curve, and an X25519
    /// exchange whose output is all zeroes.
    pub(crate) fn agree<T>(
        self,
        peer: &[u8],
        f: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, CryptoError> {
        let peer = agreement::UnparsedPublicKey::new(self.alg.ring(), peer);
        agreement::agree_ephemeral(self.private, &peer, f).map_err(|_| CryptoError)
    }
}

// ---------------------------------------------------------------------------
// Signature verification
// ---------------------------------------------------------------------------

/// The signature checks the engine performs: exactly these, and no others.
///
/// RSA moduli are bounded to 2048–8192 bits by every RSA variant, and the PSS
/// variants fix MGF1 with the same hash and a salt as long as the digest.
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
    fn ring(self) -> &'static dyn signature::VerificationAlgorithm {
        match self {
            Self::RsaPkcs1Sha256 => &signature::RSA_PKCS1_2048_8192_SHA256,
            Self::RsaPkcs1Sha384 => &signature::RSA_PKCS1_2048_8192_SHA384,
            Self::RsaPkcs1Sha512 => &signature::RSA_PKCS1_2048_8192_SHA512,
            Self::RsaPssSha256 => &signature::RSA_PSS_2048_8192_SHA256,
            Self::RsaPssSha384 => &signature::RSA_PSS_2048_8192_SHA384,
            Self::RsaPssSha512 => &signature::RSA_PSS_2048_8192_SHA512,
            Self::EcdsaP256Sha256 => &signature::ECDSA_P256_SHA256_ASN1,
            Self::EcdsaP256Sha384 => &signature::ECDSA_P256_SHA384_ASN1,
            Self::EcdsaP384Sha256 => &signature::ECDSA_P384_SHA256_ASN1,
            Self::EcdsaP384Sha384 => &signature::ECDSA_P384_SHA384_ASN1,
            Self::Ed25519 => &signature::ED25519,
        }
    }

    /// Check `signature` over `message` under `public_key`, which is the
    /// `subjectPublicKey` bit string's contents (a DER `RSAPublicKey`, an
    /// uncompressed SEC1 point, or 32 raw Ed25519 octets).
    pub(crate) fn verify(
        self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), CryptoError> {
        signature::UnparsedPublicKey::new(self.ring(), public_key)
            .verify(message, signature)
            .map_err(|_| CryptoError)
    }
}

// ---------------------------------------------------------------------------
// Randomness
// ---------------------------------------------------------------------------

/// Fill `out` from the system random source. A failure is reported, never
/// papered over with predictable bytes.
pub(crate) fn fill_random(out: &mut [u8]) -> Result<(), CryptoError> {
    rand::SystemRandom::new().fill(out).map_err(|_| CryptoError)
}
