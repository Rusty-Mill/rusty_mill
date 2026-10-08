//! Deterministic mutation fuzzing of every verifier: take each Wycheproof
//! vector, damage key, message or signature (bit flips, truncation, extension,
//! zeroing, byte repetition), and require (a) no panic and (b) the same verdict
//! as `ring`. Seeded, so a failure reproduces. Not coverage-guided.

use ring::signature::{self, UnparsedPublicKey, VerificationAlgorithm};
use rusty_ct_check::wycheproof::{cases, Case};
use rusty_pk::{ecdsa, ed25519, rsa};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn damage(rng: &mut Rng, v: &[u8]) -> Vec<u8> {
    let mut out = v.to_vec();
    match rng.below(6) {
        0 if !out.is_empty() => {
            let i = rng.below(out.len());
            out[i] ^= 1 << rng.below(8);
        }
        1 => out.truncate(rng.below(out.len() + 1)),
        2 => out.extend((0..1 + rng.below(4)).map(|_| rng.next() as u8)),
        3 if !out.is_empty() => {
            let i = rng.below(out.len());
            out[i] = 0;
        }
        4 if !out.is_empty() => {
            let (i, j) = (rng.below(out.len()), rng.below(out.len()));
            out[i] = out[j];
        }
        _ if !out.is_empty() => {
            let i = rng.below(out.len());
            out[i] = 0xff;
        }
        _ => {}
    }
    out
}

type Extract = fn(&Case) -> (Vec<u8>, Vec<u8>, Vec<u8>);

fn fuzz(
    json: &str,
    extract: Extract,
    ours: impl Fn(&[u8], &[u8], &[u8]) -> bool,
    oracle: &'static dyn VerificationAlgorithm,
    seed: u64,
    rounds: usize,
) {
    let mut rng = Rng(seed);
    let mut checked = 0;
    for case in cases(json).unwrap() {
        let (key, msg, sig) = extract(&case);
        for _ in 0..rounds {
            let (k, m, s) = match rng.below(3) {
                0 => (damage(&mut rng, &key), msg.clone(), sig.clone()),
                1 => (key.clone(), damage(&mut rng, &msg), sig.clone()),
                _ => (key.clone(), msg.clone(), damage(&mut rng, &sig)),
            };
            let theirs = UnparsedPublicKey::new(oracle, &k).verify(&m, &s).is_ok();
            assert_eq!(ours(&k, &m, &s), theirs, "tcId {} seed {seed}", case.id);
            checked += 1;
        }
    }
    assert!(checked > 100);
}

fn rsa_parts(c: &Case) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (
        c.hex("publicKeyAsn").unwrap(),
        c.hex("msg").unwrap(),
        c.hex("sig").unwrap(),
    )
}

fn spki_tail(n: usize) -> impl Fn(&Case) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    move |c| {
        let spki = c.hex("publicKeyDer").unwrap();
        (
            spki[spki.len() - n..].to_vec(),
            c.hex("msg").unwrap(),
            c.hex("sig").unwrap(),
        )
    }
}

#[test]
fn rsa_pkcs1_and_pss() {
    fuzz(
        include_str!("vectors/rsa_signature_2048_sha256_test.json"),
        rsa_parts,
        |k, m, s| rsa::verify(rsa::Scheme::Pkcs1Sha256, k, m, s).is_ok(),
        &signature::RSA_PKCS1_2048_8192_SHA256,
        11,
        8,
    );
    fuzz(
        include_str!("vectors/rsa_pss_2048_sha256_mgf1_32_test.json"),
        rsa_parts,
        |k, m, s| rsa::verify(rsa::Scheme::PssSha256, k, m, s).is_ok(),
        &signature::RSA_PSS_2048_8192_SHA256,
        12,
        8,
    );
}

#[test]
fn ecdsa_p256_and_p384() {
    fuzz(
        include_str!("vectors/ecdsa_secp256r1_sha256_test.json"),
        |c| spki_tail(65)(c),
        |k, m, s| ecdsa::verify(ecdsa::CurveId::P256, ecdsa::Digest::Sha256, k, m, s).is_ok(),
        &signature::ECDSA_P256_SHA256_ASN1,
        13,
        6,
    );
    fuzz(
        include_str!("vectors/ecdsa_secp384r1_sha384_test.json"),
        |c| spki_tail(97)(c),
        |k, m, s| ecdsa::verify(ecdsa::CurveId::P384, ecdsa::Digest::Sha384, k, m, s).is_ok(),
        &signature::ECDSA_P384_SHA384_ASN1,
        14,
        4,
    );
}

#[test]
fn ed25519_keys_and_signatures() {
    fuzz(
        include_str!("vectors/ed25519_test.json"),
        |c| spki_tail(32)(c),
        |k, m, s| ed25519::verify(k, m, s).is_ok(),
        &signature::ED25519,
        15,
        40,
    );
}

#[test]
fn pure_garbage_never_panics() {
    let mut rng = Rng(99);
    for _ in 0..2000 {
        let len = rng.below(600);
        let junk: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let other: Vec<u8> = (0..rng.below(300)).map(|_| rng.next() as u8).collect();
        let _ = rsa::verify(rsa::Scheme::PssSha384, &junk, &other, &other);
        let _ = ecdsa::verify(
            ecdsa::CurveId::P384,
            ecdsa::Digest::Sha256,
            &junk,
            &other,
            &junk,
        );
        let _ = ed25519::verify(&junk, &other, &junk);
    }
}
