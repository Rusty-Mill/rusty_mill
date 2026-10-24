//! ECDSA signing against `ring`: each side verifies the other's signatures and agrees on
//! public keys. ECDSA nonces are random in `ring` and hedged here, so signatures cannot be
//! compared byte for byte; the RFC 6979 known answers are unit tests in `src/ecdsa.rs`.

use std::collections::HashSet;

use ring::rand::SystemRandom;
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1,
    ECDSA_P256_SHA256_ASN1_SIGNING, ECDSA_P384_SHA384_ASN1, ECDSA_P384_SHA384_ASN1_SIGNING,
};
use rusty_pk::ecdsa::{hedge_len, verify, CurveId, Digest, SignError, SigningKey};

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

/// Signs one message `count` times under one key per curve with noise from `fill`, and
/// asserts that no `r` repeats.
fn assert_r_never_repeats(count: usize, mut fill: impl FnMut(&mut [u8])) {
    for curve in [CurveId::P256, CurveId::P384] {
        let (len, _) = params(curve);
        let key = SigningKey::from_bytes(curve, &vec![0x42; len]).unwrap();
        let mut seen = HashSet::with_capacity(count);
        for _ in 0..count {
            let sig = key
                .sign(b"same message", |h: &mut [u8]| {
                    fill(h);
                    Ok::<(), ()>(())
                })
                .unwrap();
            // Both curves' sequences fit a short-form length, so `r`'s header is at 2.
            let r_len = usize::from(sig[3]);
            assert!(
                seen.insert(sig[4..4 + r_len].to_vec()),
                "{curve:?}: r repeated"
            );
        }
    }
}

/// This bounded smoke test observes no repeated `r` for one key and message (2,000
/// signatures per curve), and a source that returns all zeros still yields a valid
/// (deterministic) signature.
#[test]
fn bounded_nonce_smoke_test_and_fixed_zero_noise() {
    let mut lcg = Lcg(7);
    assert_r_never_repeats(2_000, |h| lcg.fill(h));
    for curve in [CurveId::P256, CurveId::P384] {
        let (len, digest) = params(curve);
        let key = SigningKey::from_bytes(curve, &vec![0x42; len]).unwrap();
        let public = key.public_key().unwrap();
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

/// The plan's bar: `r` never repeats over 100,000 signatures of one message per curve, with
/// noise from the operating system. Slow; run with
/// `cargo test --release -p rusty_pk --test ecdsa_sign -- --ignored`.
#[test]
#[ignore = "slow: 100,000 signatures per curve"]
fn hedged_nonces_do_not_repeat_over_100k_signatures() {
    use ring::rand::SecureRandom;
    let rng = SystemRandom::new();
    assert_r_never_repeats(100_000, |h| rng.fill(h).unwrap());
}

#[test]
fn a_failed_random_source_signs_nothing() {
    for curve in [CurveId::P256, CurveId::P384] {
        let key = SigningKey::from_bytes(curve, &vec![1; hedge_len(curve)]).unwrap();
        let mut asked = 0;
        let result = key.sign(b"m", |h: &mut [u8]| {
            asked = h.len();
            h.fill(0x42); // Even a source that partially writes then fails signs nothing.
            Err("no entropy")
        });
        assert_eq!(result, Err(SignError::Random));
        assert_eq!(asked, hedge_len(curve));
    }
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

/// Exercise the production build (which has no plain RFC 6979 path) against the
/// independent fixed-noise answers, and have ring verify each resulting signature.
#[test]
fn draft_05_fixed_noise_public_signing_path() {
    let decode = |s: &str| -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    };
    for line in include_str!("vectors/hedged_ecdsa.txt").lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let curve = if fields[0] == "256" {
            CurveId::P256
        } else {
            CurveId::P384
        };
        let len = hedge_len(curve);
        let key = SigningKey::from_bytes(curve, &vec![0x11; len]).unwrap();
        let message = vec![b'a'; fields[1].parse().unwrap()];
        let noise: u8 = fields[2].parse().unwrap();
        let signature = key
            .sign(&message, |out: &mut [u8]| {
                assert_eq!(out.len(), len);
                out.fill(noise);
                Ok::<(), ()>(())
            })
            .unwrap();
        let mut body = Vec::new();
        for component in &fields[6..8] {
            let bytes = decode(component);
            let first = bytes.iter().position(|&b| b != 0).unwrap();
            let bytes = &bytes[first..];
            let pad = usize::from(bytes[0] & 0x80 != 0);
            body.extend_from_slice(&[2, (bytes.len() + pad) as u8]);
            body.extend(std::iter::repeat_n(0, pad));
            body.extend_from_slice(bytes);
        }
        let mut expected = vec![0x30, body.len() as u8];
        expected.extend_from_slice(&body);
        assert_eq!(signature, expected, "{line}");
        let verifying = if curve == CurveId::P256 {
            &ECDSA_P256_SHA256_ASN1
        } else {
            &ECDSA_P384_SHA384_ASN1
        };
        let public = key.public_key().unwrap();
        UnparsedPublicKey::new(verifying, public.as_bytes())
            .verify(&message, &signature)
            .unwrap();
    }
}
