//! The `ring` backend (ADR-0002 §6), the engine's default.
//!
//! Also compiled into native-backend test builds, so the seam differential in
//! [`super::native`]'s tests can hold the two side by side.

use ring::rand::SecureRandom;
use ring::{aead, agreement, digest as ring_digest, hmac, rand, signature};

use super::{AeadAlgorithm, CryptoError, HashAlgorithm, KxAlgorithm, Verifier, AEAD_NONCE_LEN};

fn digest_algorithm(alg: HashAlgorithm) -> &'static ring_digest::Algorithm {
    match alg {
        HashAlgorithm::Sha256 => &ring_digest::SHA256,
        HashAlgorithm::Sha384 => &ring_digest::SHA384,
    }
}

fn hmac_algorithm(alg: HashAlgorithm) -> hmac::Algorithm {
    match alg {
        HashAlgorithm::Sha256 => hmac::HMAC_SHA256,
        HashAlgorithm::Sha384 => hmac::HMAC_SHA384,
    }
}

/// `alg(message)`.
pub(crate) fn digest(alg: HashAlgorithm, message: &[u8]) -> Vec<u8> {
    ring_digest::digest(digest_algorithm(alg), message)
        .as_ref()
        .to_vec()
}

/// An HMAC key.
pub(crate) struct HmacKey(hmac::Key);

impl HmacKey {
    pub(crate) fn new(alg: HashAlgorithm, key: &[u8]) -> Self {
        Self(hmac::Key::new(hmac_algorithm(alg), key))
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

fn aead_algorithm(alg: AeadAlgorithm) -> &'static aead::Algorithm {
    match alg {
        AeadAlgorithm::Aes128Gcm => &aead::AES_128_GCM,
        AeadAlgorithm::Aes256Gcm => &aead::AES_256_GCM,
        AeadAlgorithm::ChaCha20Poly1305 => &aead::CHACHA20_POLY1305,
    }
}

/// One direction's AEAD key.
///
/// Nonce uniqueness is the caller's contract: the record layers derive every
/// nonce from a sequence number that never repeats for a key.
pub(crate) struct AeadKey(aead::LessSafeKey);

impl AeadKey {
    /// Fails only when `key` is the wrong length for `alg`.
    pub(crate) fn new(alg: AeadAlgorithm, key: &[u8]) -> Result<Self, CryptoError> {
        let unbound = aead::UnboundKey::new(aead_algorithm(alg), key).map_err(|_| CryptoError)?;
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

fn kx_algorithm(alg: KxAlgorithm) -> &'static agreement::Algorithm {
    match alg {
        KxAlgorithm::P256 => &agreement::ECDH_P256,
        KxAlgorithm::P384 => &agreement::ECDH_P384,
        KxAlgorithm::X25519 => &agreement::X25519,
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
            agreement::EphemeralPrivateKey::generate(kx_algorithm(alg), &rand::SystemRandom::new())
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

    /// Agree a secret with `peer` and hand it to `f`. Refuses a peer key of
    /// the wrong length, a NIST point off the curve, and an X25519 exchange
    /// whose output is all zeroes.
    pub(crate) fn agree<T>(
        self,
        peer: &[u8],
        f: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, CryptoError> {
        let peer = agreement::UnparsedPublicKey::new(kx_algorithm(self.alg), peer);
        agreement::agree_ephemeral(self.private, &peer, f).map_err(|_| CryptoError)
    }
}

fn verification_algorithm(verifier: Verifier) -> &'static dyn signature::VerificationAlgorithm {
    match verifier {
        Verifier::RsaPkcs1Sha256 => &signature::RSA_PKCS1_2048_8192_SHA256,
        Verifier::RsaPkcs1Sha384 => &signature::RSA_PKCS1_2048_8192_SHA384,
        Verifier::RsaPkcs1Sha512 => &signature::RSA_PKCS1_2048_8192_SHA512,
        Verifier::RsaPssSha256 => &signature::RSA_PSS_2048_8192_SHA256,
        Verifier::RsaPssSha384 => &signature::RSA_PSS_2048_8192_SHA384,
        Verifier::RsaPssSha512 => &signature::RSA_PSS_2048_8192_SHA512,
        Verifier::EcdsaP256Sha256 => &signature::ECDSA_P256_SHA256_ASN1,
        Verifier::EcdsaP256Sha384 => &signature::ECDSA_P256_SHA384_ASN1,
        Verifier::EcdsaP384Sha256 => &signature::ECDSA_P384_SHA256_ASN1,
        Verifier::EcdsaP384Sha384 => &signature::ECDSA_P384_SHA384_ASN1,
        Verifier::Ed25519 => &signature::ED25519,
    }
}

/// See [`Verifier::verify`].
pub(crate) fn verify(
    verifier: Verifier,
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), CryptoError> {
    signature::UnparsedPublicKey::new(verification_algorithm(verifier), public_key)
        .verify(message, signature)
        .map_err(|_| CryptoError)
}

/// Fill `out` from the system random source. A failure is reported, never
/// papered over with predictable bytes.
pub(crate) fn fill_random(out: &mut [u8]) -> Result<(), CryptoError> {
    rand::SystemRandom::new().fill(out).map_err(|_| CryptoError)
}
