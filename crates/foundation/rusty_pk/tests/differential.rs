//! Differential tests: signatures made by `ring` must verify with us, and any
//! mutation of key, message or signature must get the same verdict from both.
//! Divergence is a finding until proven otherwise.

use ring::rand::SystemRandom;
use ring::signature::{
    self, EcdsaKeyPair, Ed25519KeyPair, KeyPair, UnparsedPublicKey, VerificationAlgorithm,
};
use rusty_pk::ecdsa::{self, CurveId, Digest};
use rusty_pk::ed25519;

/// xorshift64*: deterministic test input.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn message(&mut self) -> Vec<u8> {
        let len = self.below(300);
        (0..len).map(|_| self.next() as u8).collect()
    }
}

/// Flips one random bit of one of the three inputs.
fn mutate(rng: &mut Rng, key: &[u8], msg: &[u8], sig: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (mut k, mut m, mut s) = (key.to_vec(), msg.to_vec(), sig.to_vec());
    let target = match rng.below(3) {
        0 => &mut k,
        1 if !m.is_empty() => &mut m,
        _ => &mut s,
    };
    let (i, bit) = (rng.below(target.len()), rng.below(8));
    target[i] ^= 1 << bit;
    (k, m, s)
}

fn ecdsa_round(
    curve: CurveId,
    digest: Digest,
    signing: &'static signature::EcdsaSigningAlgorithm,
    oracle: &'static dyn VerificationAlgorithm,
    seed: u64,
) {
    let (rng, mut r) = (SystemRandom::new(), Rng(seed));
    let mut mutated_agree = 0;
    for _ in 0..40 {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(signing, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(signing, pkcs8.as_ref(), &rng).unwrap();
        let key = pair.public_key().as_ref().to_vec();
        let msg = r.message();
        let sig = pair.sign(&rng, &msg).unwrap().as_ref().to_vec();
        assert!(
            ecdsa::verify(curve, digest, &key, &msg, &sig).is_ok(),
            "ring signature rejected"
        );
        for _ in 0..25 {
            let (k, m, s) = mutate(&mut r, &key, &msg, &sig);
            let ours = ecdsa::verify(curve, digest, &k, &m, &s).is_ok();
            let theirs = UnparsedPublicKey::new(oracle, &k).verify(&m, &s).is_ok();
            assert_eq!(ours, theirs, "mutated input: verdicts differ");
            mutated_agree += 1;
        }
    }
    assert_eq!(mutated_agree, 1000);
}

#[test]
fn ecdsa_p256_sha256() {
    ecdsa_round(
        CurveId::P256,
        Digest::Sha256,
        &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &signature::ECDSA_P256_SHA256_ASN1,
        1,
    );
}

#[test]
fn ecdsa_p384_sha384() {
    ecdsa_round(
        CurveId::P384,
        Digest::Sha384,
        &signature::ECDSA_P384_SHA384_ASN1_SIGNING,
        &signature::ECDSA_P384_SHA384_ASN1,
        2,
    );
}

#[test]
fn ed25519() {
    let (rng, mut r) = (SystemRandom::new(), Rng(3));
    for _ in 0..40 {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let key = pair.public_key().as_ref().to_vec();
        let msg = r.message();
        let sig = pair.sign(&msg).as_ref().to_vec();
        assert!(
            ed25519::verify(&key, &msg, &sig).is_ok(),
            "ring signature rejected"
        );
        for _ in 0..25 {
            let (k, m, s) = mutate(&mut r, &key, &msg, &sig);
            let ours = ed25519::verify(&k, &m, &s).is_ok();
            let theirs = UnparsedPublicKey::new(&signature::ED25519, &k)
                .verify(&m, &s)
                .is_ok();
            assert_eq!(ours, theirs, "mutated input: verdicts differ");
        }
    }
}
