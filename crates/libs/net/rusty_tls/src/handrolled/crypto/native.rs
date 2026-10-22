//! The first-party backend (ADR-0005): `rusty_sha2`, `rusty_pk`, `rusty_aead`
//! and `rusty_rand` instead of `ring`.
//!
//! Selected only by `--cfg rusty_tls_native_crypto` with the `native-crypto`
//! feature. Every item mirrors [`super::ring`] and keeps its contract; where
//! the two crates' APIs differ (tags returned rather than appended, keys taken
//! as fixed-size arrays, randomness supplied by the caller), the difference is
//! absorbed here so callers cannot tell which backend they have.
//!
//! The secret-handling primitives (AEAD, X25519, ECDH) carry their crates'
//! constant-time evidence, which is "no leak found by the listed methods on
//! the listed toolchain", never proof (`CRYPTO-REPLACEMENT-PLAN.md` §4). This
//! module adds no branch or index on secret data: tags are compared by
//! [`rusty_crypto_key::constant_time_eq`], and the only branches are on
//! lengths and on a verdict that is already public.

use rusty_aead::{Aes128Gcm, Aes256Gcm, ChaCha20Poly1305};
use rusty_crypto_key::{constant_time_eq, wipe};
use rusty_pk::{ecdh, ecdsa, ed25519, rsa, x25519};
use rusty_sha2::{Hash, HmacSha256, HmacSha384, Sha256, Sha384};

use super::{
    AeadAlgorithm, CryptoError, HashAlgorithm, KxAlgorithm, Verifier, AEAD_NONCE_LEN, AEAD_TAG_LEN,
};

/// `alg(message)`.
pub(crate) fn digest(alg: HashAlgorithm, message: &[u8]) -> Vec<u8> {
    match alg {
        HashAlgorithm::Sha256 => Sha256::digest(message).as_ref().to_vec(),
        HashAlgorithm::Sha384 => Sha384::digest(message).as_ref().to_vec(),
    }
}

/// An HMAC key: the keyed state after absorbing `key ⊕ ipad`, cloned per
/// message. `rusty_sha2` wipes its key blocks on drop.
#[derive(Clone)]
pub(crate) enum HmacKey {
    Sha256(HmacSha256),
    Sha384(HmacSha384),
}

impl HmacKey {
    pub(crate) fn new(alg: HashAlgorithm, key: &[u8]) -> Self {
        match alg {
            HashAlgorithm::Sha256 => Self::Sha256(HmacSha256::new(key)),
            HashAlgorithm::Sha384 => Self::Sha384(HmacSha384::new(key)),
        }
    }

    /// `HMAC(key, message)`.
    pub(crate) fn sign(&self, message: &[u8]) -> Vec<u8> {
        let mut context = self.context();
        context.update(message);
        context.sign()
    }

    /// Whether `tag` is `HMAC(key, message)`, compared in constant time.
    ///
    /// `tag` must be the full-length MAC; a truncated tag is a mismatch.
    pub(crate) fn verify(&self, message: &[u8], tag: &[u8]) -> bool {
        let mut expected = self.sign(message);
        let ok = constant_time_eq(&expected, tag);
        wipe(&mut expected);
        ok
    }

    /// An incremental HMAC under this key, for a message built from pieces.
    pub(crate) fn context(&self) -> HmacContext {
        HmacContext(self.clone())
    }
}

/// An HMAC being computed over a message supplied in pieces.
pub(crate) struct HmacContext(HmacKey);

impl HmacContext {
    pub(crate) fn update(&mut self, piece: &[u8]) {
        match &mut self.0 {
            HmacKey::Sha256(state) => state.update(piece),
            HmacKey::Sha384(state) => state.update(piece),
        }
    }

    pub(crate) fn sign(self) -> Vec<u8> {
        match self.0 {
            HmacKey::Sha256(state) => state.finalize().as_ref().to_vec(),
            HmacKey::Sha384(state) => state.finalize().as_ref().to_vec(),
        }
    }
}

/// One direction's AEAD key. The crates wipe key material on drop.
///
/// Nonce uniqueness is the caller's contract: the record layers derive every
/// nonce from a sequence number that never repeats for a key.
pub(crate) enum AeadKey {
    Aes128Gcm(Aes128Gcm),
    Aes256Gcm(Aes256Gcm),
    ChaCha20Poly1305(ChaCha20Poly1305),
}

impl AeadKey {
    /// Fails only when `key` is the wrong length for `alg`.
    pub(crate) fn new(alg: AeadAlgorithm, key: &[u8]) -> Result<Self, CryptoError> {
        Ok(match alg {
            AeadAlgorithm::Aes128Gcm => {
                Self::Aes128Gcm(Aes128Gcm::new(key.try_into().map_err(|_| CryptoError)?))
            }
            AeadAlgorithm::Aes256Gcm => {
                Self::Aes256Gcm(Aes256Gcm::new(key.try_into().map_err(|_| CryptoError)?))
            }
            AeadAlgorithm::ChaCha20Poly1305 => Self::ChaCha20Poly1305(ChaCha20Poly1305::new(
                key.try_into().map_err(|_| CryptoError)?,
            )),
        })
    }

    /// Encrypt `in_out` in place and append the tag.
    pub(crate) fn seal_in_place_append_tag(
        &self,
        nonce: [u8; AEAD_NONCE_LEN],
        aad: &[u8],
        in_out: &mut Vec<u8>,
    ) -> Result<(), CryptoError> {
        let tag = match self {
            Self::Aes128Gcm(key) => key.seal_in_place(&nonce, aad, in_out),
            Self::Aes256Gcm(key) => key.seal_in_place(&nonce, aad, in_out),
            Self::ChaCha20Poly1305(key) => key.seal_in_place(&nonce, aad, in_out),
        }
        .map_err(|_| CryptoError)?;
        in_out.extend_from_slice(&tag);
        Ok(())
    }

    /// Check the tag at the end of `in_out` and decrypt in place, returning
    /// the plaintext prefix. On failure the contents of `in_out` must not be
    /// used (this backend leaves them unchanged; `ring` does not promise to).
    pub(crate) fn open_in_place<'a>(
        &self,
        nonce: [u8; AEAD_NONCE_LEN],
        aad: &[u8],
        in_out: &'a mut [u8],
    ) -> Result<&'a mut [u8], CryptoError> {
        let plain_len = in_out.len().checked_sub(AEAD_TAG_LEN).ok_or(CryptoError)?;
        let (ciphertext, tag) = in_out.split_at_mut(plain_len);
        match self {
            Self::Aes128Gcm(key) => key.open_in_place(&nonce, aad, ciphertext, tag),
            Self::Aes256Gcm(key) => key.open_in_place(&nonce, aad, ciphertext, tag),
            Self::ChaCha20Poly1305(key) => key.open_in_place(&nonce, aad, ciphertext, tag),
        }
        .map_err(|_| CryptoError)?;
        Ok(ciphertext)
    }
}

/// A single-use ephemeral private key.
pub(crate) enum EphemeralKey {
    /// The raw scalar; [`x25519`] clamps it on use. Wiped on drop.
    X25519([u8; x25519::KEY_LEN]),
    /// A scalar in `[1, n-1]`, wiped on drop by `rusty_pk`.
    Nist(ecdh::PrivateKey),
}

impl Drop for EphemeralKey {
    fn drop(&mut self) {
        if let Self::X25519(scalar) = self {
            wipe(scalar);
        }
    }
}

impl EphemeralKey {
    /// Generate a key from the system random source. Fails only if that
    /// source fails (or, for the NIST curves, if it keeps returning values
    /// outside `[1, n-1]`, which a working source does not).
    pub(crate) fn generate(alg: KxAlgorithm) -> Result<Self, CryptoError> {
        let curve = match alg {
            KxAlgorithm::X25519 => {
                let mut scalar = [0u8; x25519::KEY_LEN];
                if fill_random(&mut scalar).is_err() {
                    wipe(&mut scalar);
                    return Err(CryptoError);
                }
                return Ok(Self::X25519(scalar));
            }
            KxAlgorithm::P256 => ecdh::Curve::P256,
            KxAlgorithm::P384 => ecdh::Curve::P384,
        };
        ecdh::PrivateKey::generate(curve, rusty_rand::fill)
            .map(Self::Nist)
            .map_err(|_| CryptoError)
    }

    /// The public key: 32 raw octets for X25519, an uncompressed SEC1 point
    /// for the NIST curves.
    pub(crate) fn public_key(&self) -> Result<Vec<u8>, CryptoError> {
        match self {
            Self::X25519(scalar) => Ok(x25519::public_key(scalar).to_vec()),
            Self::Nist(private) => Ok(private
                .public_key()
                .map_err(|_| CryptoError)?
                .as_bytes()
                .to_vec()),
        }
    }

    /// Agree a secret with `peer` and hand it to `f`. Refuses a peer key of
    /// the wrong length, a NIST point off the curve, and an X25519 exchange
    /// whose output is all zeroes.
    pub(crate) fn agree<T>(
        self,
        peer: &[u8],
        f: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, CryptoError> {
        match &self {
            Self::X25519(scalar) => {
                let peer: &[u8; x25519::KEY_LEN] = peer.try_into().map_err(|_| CryptoError)?;
                let mut shared = x25519::agree(scalar, peer).map_err(|_| CryptoError)?;
                let result = f(&shared);
                wipe(&mut shared);
                Ok(result)
            }
            Self::Nist(private) => {
                let shared = private.agree(peer).map_err(|_| CryptoError)?;
                Ok(f(shared.as_bytes()))
            }
        }
    }
}

/// See [`Verifier::verify`].
pub(crate) fn verify(
    verifier: Verifier,
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), CryptoError> {
    use ecdsa::{CurveId, Digest};

    let rsa_scheme = match verifier {
        Verifier::RsaPkcs1Sha256 => Some(rsa::Scheme::Pkcs1Sha256),
        Verifier::RsaPkcs1Sha384 => Some(rsa::Scheme::Pkcs1Sha384),
        Verifier::RsaPkcs1Sha512 => Some(rsa::Scheme::Pkcs1Sha512),
        Verifier::RsaPssSha256 => Some(rsa::Scheme::PssSha256),
        Verifier::RsaPssSha384 => Some(rsa::Scheme::PssSha384),
        Verifier::RsaPssSha512 => Some(rsa::Scheme::PssSha512),
        _ => None,
    };
    let result = match (rsa_scheme, verifier) {
        (Some(scheme), _) => rsa::verify(scheme, public_key, message, signature),
        (None, Verifier::EcdsaP256Sha256) => ecdsa::verify(
            CurveId::P256,
            Digest::Sha256,
            public_key,
            message,
            signature,
        ),
        (None, Verifier::EcdsaP256Sha384) => ecdsa::verify(
            CurveId::P256,
            Digest::Sha384,
            public_key,
            message,
            signature,
        ),
        (None, Verifier::EcdsaP384Sha256) => ecdsa::verify(
            CurveId::P384,
            Digest::Sha256,
            public_key,
            message,
            signature,
        ),
        (None, Verifier::EcdsaP384Sha384) => ecdsa::verify(
            CurveId::P384,
            Digest::Sha384,
            public_key,
            message,
            signature,
        ),
        (None, _) => ed25519::verify(public_key, message, signature),
    };
    result.map_err(|_| CryptoError)
}

/// Fill `out` from the OS random source. A failure is reported, never papered
/// over with predictable bytes.
pub(crate) fn fill_random(out: &mut [u8]) -> Result<(), CryptoError> {
    rusty_rand::fill(out).map_err(|_| CryptoError)
}

/// The seam differential (`TLS-NATIVE-CRYPTO-SEAM.md` §4): the same inputs
/// through both backends, byte-identical outputs, and the same refusals.
///
/// RSA and the two cross-curve ECDSA verifiers (P-256 with SHA-384, P-384 with
/// SHA-256) are not repeated here: `ring` signs neither of those ECDSA pairs
/// and cannot sign RSA without a key fixture. The engine's own suites
/// (`handrolled_tls_signature`, `handrolled_tls12_signature`,
/// `handrolled_verify`, the RSA-leaf interop tests) cover them, and they run
/// through this backend whenever the native cfg is set.
#[cfg(test)]
mod tests {
    use super::super::ring;
    use super::*;

    use ::ring::rand::SystemRandom;
    use ::ring::signature::{self as ring_sig, KeyPair};

    /// Deterministic, varied bytes: inputs here are public test data.
    fn bytes(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    const HASHES: [HashAlgorithm; 2] = [HashAlgorithm::Sha256, HashAlgorithm::Sha384];
    const AEADS: [AeadAlgorithm; 3] = [
        AeadAlgorithm::Aes128Gcm,
        AeadAlgorithm::Aes256Gcm,
        AeadAlgorithm::ChaCha20Poly1305,
    ];
    const GROUPS: [KxAlgorithm; 3] = [KxAlgorithm::X25519, KxAlgorithm::P256, KxAlgorithm::P384];
    /// Around every block boundary that matters: SHA-256's 64, SHA-384's 128,
    /// the AEADs' 16 and 64.
    const LENGTHS: [usize; 12] = [0, 1, 15, 16, 17, 55, 63, 64, 65, 127, 128, 300];

    #[test]
    fn digests_match() {
        for alg in HASHES {
            for len in LENGTHS {
                let message = bytes(len, 7);
                assert_eq!(
                    digest(alg, &message),
                    ring::digest(alg, &message),
                    "{alg:?} {len}"
                );
            }
        }
    }

    #[test]
    fn hmacs_match_one_shot_and_in_pieces() {
        for alg in HASHES {
            // Keys shorter than, equal to and longer than the block are three
            // different code paths in HMAC.
            for key_len in [0, 32, 64, 128, 200] {
                let key = bytes(key_len, 1);
                let ours = HmacKey::new(alg, &key);
                let theirs = ring::HmacKey::new(alg, &key);
                for len in LENGTHS {
                    let message = bytes(len, 9);
                    let tag = ours.sign(&message);
                    assert_eq!(
                        tag,
                        theirs.sign(&message),
                        "{alg:?} key {key_len} msg {len}"
                    );

                    let (a, b) = message.split_at(len / 2);
                    let mut context = ours.context();
                    context.update(a);
                    context.update(b);
                    assert_eq!(context.sign(), tag);

                    assert!(ours.verify(&message, &tag));
                    let mut wrong = tag.clone();
                    wrong[0] ^= 1;
                    assert!(!ours.verify(&message, &wrong));
                    assert!(
                        !ours.verify(&message, &tag[..tag.len() - 1]),
                        "truncated tag"
                    );
                }
            }
        }
    }

    #[test]
    fn aeads_seal_identically_and_open_each_other() {
        for alg in AEADS {
            let key = bytes(alg.key_len(), 3);
            let ours = AeadKey::new(alg, &key).unwrap();
            let theirs = ring::AeadKey::new(alg, &key).unwrap();
            for len in LENGTHS {
                let nonce: [u8; AEAD_NONCE_LEN] =
                    bytes(AEAD_NONCE_LEN, len as u8).try_into().unwrap();
                let aad = bytes(13, 5);
                let plain = bytes(len, 11);

                let mut sealed = plain.clone();
                ours.seal_in_place_append_tag(nonce, &aad, &mut sealed)
                    .unwrap();
                let mut sealed_by_ring = plain.clone();
                theirs
                    .seal_in_place_append_tag(nonce, &aad, &mut sealed_by_ring)
                    .unwrap();
                assert_eq!(sealed, sealed_by_ring, "{alg:?} {len}");

                let mut buf = sealed_by_ring.clone();
                assert_eq!(
                    ours.open_in_place(nonce, &aad, &mut buf).unwrap(),
                    &plain[..]
                );
                let mut buf = sealed.clone();
                assert_eq!(
                    theirs.open_in_place(nonce, &aad, &mut buf).unwrap(),
                    &plain[..]
                );
            }
        }
    }

    #[test]
    fn aeads_refuse_what_ring_refuses() {
        for alg in AEADS {
            let key = bytes(alg.key_len(), 3);
            let ours = AeadKey::new(alg, &key).unwrap();
            let theirs = ring::AeadKey::new(alg, &key).unwrap();
            let nonce = [4u8; AEAD_NONCE_LEN];
            let mut sealed = bytes(40, 2);
            ours.seal_in_place_append_tag(nonce, b"aad", &mut sealed)
                .unwrap();

            // A flipped bit in the ciphertext, in the tag, a different AAD, a
            // different nonce, and a buffer shorter than a tag.
            let last = sealed.len() - 1;
            let cases: [(Vec<u8>, &[u8], [u8; AEAD_NONCE_LEN]); 5] = [
                (
                    {
                        let mut b = sealed.clone();
                        b[0] ^= 1;
                        b
                    },
                    b"aad",
                    nonce,
                ),
                (
                    {
                        let mut b = sealed.clone();
                        b[last] ^= 1;
                        b
                    },
                    b"aad",
                    nonce,
                ),
                (sealed.clone(), b"aaD", nonce),
                (sealed.clone(), b"aad", [5u8; AEAD_NONCE_LEN]),
                (sealed[..AEAD_TAG_LEN - 1].to_vec(), b"aad", nonce),
            ];
            for (i, (buf, aad, nonce)) in cases.into_iter().enumerate() {
                let mut a = buf.clone();
                let mut b = buf;
                assert!(
                    ours.open_in_place(nonce, aad, &mut a).is_err(),
                    "{alg:?} case {i}"
                );
                assert!(
                    theirs.open_in_place(nonce, aad, &mut b).is_err(),
                    "{alg:?} case {i}"
                );
            }
        }
    }

    #[test]
    fn aead_keys_of_the_wrong_length_are_refused() {
        for alg in AEADS {
            for len in [0, alg.key_len() - 1, alg.key_len() + 1] {
                assert!(AeadKey::new(alg, &bytes(len, 0)).is_err());
                assert!(ring::AeadKey::new(alg, &bytes(len, 0)).is_err());
            }
        }
    }

    #[test]
    fn key_exchange_agrees_with_ring_in_both_directions() {
        for alg in GROUPS {
            let ours = EphemeralKey::generate(alg).unwrap();
            let theirs = ring::EphemeralKey::generate(alg).unwrap();
            let our_public = ours.public_key().unwrap();
            let their_public = theirs.public_key().unwrap();
            assert_eq!(our_public.len(), their_public.len(), "{alg:?} encoding");

            let a = ours.agree(&their_public, <[u8]>::to_vec).unwrap();
            let b = theirs.agree(&our_public, <[u8]>::to_vec).unwrap();
            assert_eq!(a, b, "{alg:?}");
        }
    }

    #[test]
    fn key_exchange_refuses_what_ring_refuses() {
        for alg in GROUPS {
            let good = ring::EphemeralKey::generate(alg)
                .unwrap()
                .public_key()
                .unwrap();
            let mut off_curve = good.clone();
            let last = off_curve.len() - 1;
            off_curve[last] ^= 1;
            let mut cases = vec![
                Vec::new(),
                good[..good.len() - 1].to_vec(),
                [good.clone(), vec![0]].concat(),
            ];
            match alg {
                // All-zero output: the identity point has small order.
                KxAlgorithm::X25519 => cases.push(vec![0u8; 32]),
                // A NIST point with one coordinate changed is off the curve,
                // and a compressed or infinity encoding is not accepted.
                KxAlgorithm::P256 | KxAlgorithm::P384 => {
                    cases.push(off_curve);
                    cases.push(vec![0]);
                }
            }
            for (i, peer) in cases.iter().enumerate() {
                let ours = EphemeralKey::generate(alg).unwrap().agree(peer, |_| ());
                let theirs = ring::EphemeralKey::generate(alg)
                    .unwrap()
                    .agree(peer, |_| ());
                assert!(ours.is_err(), "{alg:?} case {i} accepted by native");
                assert!(theirs.is_err(), "{alg:?} case {i} accepted by ring");
            }
        }
    }

    #[test]
    fn ecdsa_and_ed25519_verify_like_ring() {
        let rng = SystemRandom::new();
        let message = bytes(100, 8);
        let ecdsa = [
            (
                &ring_sig::ECDSA_P256_SHA256_ASN1_SIGNING,
                Verifier::EcdsaP256Sha256,
            ),
            (
                &ring_sig::ECDSA_P384_SHA384_ASN1_SIGNING,
                Verifier::EcdsaP384Sha384,
            ),
        ];
        let mut cases = Vec::new();
        for (signing, verifier) in ecdsa {
            let pkcs8 = ring_sig::EcdsaKeyPair::generate_pkcs8(signing, &rng).unwrap();
            let pair = ring_sig::EcdsaKeyPair::from_pkcs8(signing, pkcs8.as_ref(), &rng).unwrap();
            let sig = pair.sign(&rng, &message).unwrap().as_ref().to_vec();
            cases.push((verifier, pair.public_key().as_ref().to_vec(), sig));
        }
        let pkcs8 = ring_sig::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = ring_sig::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        cases.push((
            Verifier::Ed25519,
            pair.public_key().as_ref().to_vec(),
            pair.sign(&message).as_ref().to_vec(),
        ));

        for (verifier, key, sig) in cases {
            assert!(
                verify(verifier, &key, &message, &sig).is_ok(),
                "{verifier:?}"
            );
            assert!(
                ring::verify(verifier, &key, &message, &sig).is_ok(),
                "{verifier:?}"
            );

            let mut wrong_message = message.clone();
            wrong_message[0] ^= 1;
            let mut wrong_sig = sig.clone();
            let last = wrong_sig.len() - 1;
            wrong_sig[last] ^= 1;
            for (m, s) in [(&wrong_message, &sig), (&message, &wrong_sig)] {
                assert!(verify(verifier, &key, m, s).is_err(), "{verifier:?}");
                assert!(ring::verify(verifier, &key, m, s).is_err(), "{verifier:?}");
            }
        }
    }

    #[test]
    fn random_fill_is_filled() {
        let mut out = [0u8; 64];
        fill_random(&mut out).unwrap();
        // 2^-512 chance of a false failure.
        assert_ne!(out, [0u8; 64]);
    }
}
