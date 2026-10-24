//! ECDSA signing against `ring`: each side verifies the other's signatures and agrees on
//! public keys. ECDSA nonces are random in `ring` and hedged here, so signatures cannot be
//! compared byte for byte; the RFC 6979 known answers are unit tests in `src/ecdsa.rs`.

use std::collections::HashSet;

use ring::rand::SystemRandom;
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1,
    ECDSA_P256_SHA256_ASN1_SIGNING, ECDSA_P384_SHA384_ASN1, ECDSA_P384_SHA384_ASN1_SIGNING,
};
use rusty_pk::ecdsa::{verify, CurveId, Digest, SignError, SigningKey, HEDGE_LEN};

struct Lcg(u64);

impl Lcg {
    fn fill(&mut self, out: &mut [u8]) {
        for b in out {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *b = (self.0 >> 33) as u8;
        }
    }
}

fn params(curve: CurveId) -> (usize, Digest) {
    match curve {
        CurveId::P256 => (32, Digest::Sha256),
        CurveId::P384 => (48, Digest::Sha384),
    }
}

/// 200 keys per curve, message lengths 0 to 398: our signatures verify under `ring` and
/// ours; `ring`'s verify under ours; `ring` accepts our public key for the private key.
#[test]
fn cross_verifies_with_ring_on_both_curves() {
    let rng = SystemRandom::new();
    let mut lcg = Lcg(0xec);
    for curve in [CurveId::P256, CurveId::P384] {
        let (len, digest) = params(curve);
        let (signing, verifying) = match curve {
            CurveId::P256 => (&ECDSA_P256_SHA256_ASN1_SIGNING, &ECDSA_P256_SHA256_ASN1),
            CurveId::P384 => (&ECDSA_P384_SHA384_ASN1_SIGNING, &ECDSA_P384_SHA384_ASN1),
        };
        for i in 0..200 {
            let mut d = vec![0u8; len];
            lcg.fill(&mut d);
            d[0] &= 0x7f; // below n
            let ours = SigningKey::from_bytes(curve, &d).unwrap();
            let public = ours.public_key().unwrap();
            let theirs =
                EcdsaKeyPair::from_private_key_and_public_key(signing, &d, public.as_bytes(), &rng)
                    .expect("ring accepts our public key for this private key");
            assert_eq!(theirs.public_key().as_ref(), public.as_bytes());
            let mut msg = vec![0u8; i * 2];
            lcg.fill(&mut msg);

            let sig = ours
                .sign(&msg, |h: &mut [u8]| {
                    lcg.fill(h);
                    Ok::<(), ()>(())
                })
                .unwrap();
            assert!(
                verify(curve, digest, public.as_bytes(), &msg, &sig).is_ok(),
                "{curve:?} {i}"
            );
            assert!(UnparsedPublicKey::new(verifying, public.as_bytes())
                .verify(&msg, &sig)
                .is_ok());

            let ring_sig = theirs.sign(&rng, &msg).unwrap();
            assert!(verify(curve, digest, public.as_bytes(), &msg, ring_sig.as_ref()).is_ok());
        }
    }
}

/// The hedge never repeats `r` for one key and message (2,000 signatures per curve), and a
/// source that returns all zeros still yields a valid (deterministic) signature.
#[test]
fn hedged_nonces_do_not_repeat_and_a_dead_source_still_signs_safely() {
    let mut lcg = Lcg(7);
    for curve in [CurveId::P256, CurveId::P384] {
        let (len, digest) = params(curve);
        let key = SigningKey::from_bytes(curve, &vec![0x42; len]).unwrap();
        let public = key.public_key().unwrap();
        let mut seen = HashSet::new();
        for _ in 0..2_000 {
            let sig = key
                .sign(b"same message", |h: &mut [u8]| {
                    lcg.fill(h);
                    Ok::<(), ()>(())
                })
                .unwrap();
            let r_len = usize::from(sig[3]);
            assert!(
                seen.insert(sig[4..4 + r_len].to_vec()),
                "{curve:?}: r repeated"
            );
        }
        let zero = |h: &mut [u8]| {
            h.fill(0);
            Ok::<(), ()>(())
        };
        let a = key.sign(b"m", zero).unwrap();
        assert_eq!(
            a,
            key.sign(b"m", zero).unwrap(),
            "a zero hedge is deterministic"
        );
        assert!(verify(curve, digest, public.as_bytes(), b"m", &a).is_ok());
    }
}

#[test]
fn a_failed_random_source_signs_nothing() {
    let key = SigningKey::from_bytes(CurveId::P256, &[1; 32]).unwrap();
    let mut asked = 0;
    let result = key.sign(b"m", |h: &mut [u8]| {
        asked = h.len();
        Err("no entropy")
    });
    assert_eq!(result, Err(SignError::Random));
    assert_eq!(asked, HEDGE_LEN);
}

#[test]
fn invalid_private_keys_are_refused() {
    const N256: &str = "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551";
    let n: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&N256[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    for bad in [vec![0u8; 32], n, vec![0xff; 32], vec![1; 31], vec![1; 33]] {
        assert_eq!(
            SigningKey::from_bytes(CurveId::P256, &bad).unwrap_err(),
            SignError::InvalidKey
        );
    }
    assert!(SigningKey::from_bytes(CurveId::P384, &[1; 32]).is_err());
}

#[test]
fn debug_does_not_print_the_key() {
    let key = SigningKey::from_bytes(CurveId::P384, &[3; 48]).unwrap();
    assert_eq!(format!("{key:?}"), "ecdsa::SigningKey(..)");
}
