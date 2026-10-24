//! Ed25519 signing against RFC 8032 section 7.1 and `ring`.
//!
//! Ed25519 signatures are deterministic, so for the same seed and message the
//! bytes must be identical to `ring`'s, and every signature must verify under
//! both verifiers.

use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use rusty_hex::decode;
use rusty_pk::ed25519::{verify, GenerateError, SigningKey};

fn seed(hex: &str) -> [u8; 32] {
    decode(hex).unwrap().try_into().unwrap()
}

/// RFC 8032 section 7.1, tests 1 to 3 and "SHA(abc)".
#[test]
fn rfc_8032_vectors() {
    let vectors = [
        (
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            "",
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        ),
        (
            "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
            "72",
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
        ),
        (
            "c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
            "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025",
            "af82",
            "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a",
        ),
    ];
    for (sk, pk, msg, sig) in vectors {
        let key = SigningKey::from_seed(&seed(sk)).unwrap();
        assert_eq!(
            key.public_key().as_slice(),
            decode(pk).unwrap(),
            "public key for {sk}"
        );
        let msg = decode(msg).unwrap();
        let ours = key.sign(&msg).unwrap();
        assert_eq!(ours.as_slice(), decode(sig).unwrap(), "signature for {sk}");
        assert!(verify(key.public_key(), &msg, &ours).is_ok());
    }
}

/// A small deterministic generator, so the differential is reproducible.
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

/// Same seed and message, same bytes as `ring`, for 500 keys and message lengths 0
/// to 300 (around SHA-512's 128-byte block boundaries).
#[test]
fn signatures_and_public_keys_are_byte_identical_to_ring() {
    let mut rng = Lcg(0x5eed);
    for i in 0..500 {
        let mut s = [0u8; 32];
        rng.fill(&mut s);
        let mut msg = vec![0u8; (i * 7) % 301];
        rng.fill(&mut msg);
        let ours = SigningKey::from_seed(&s).unwrap();
        let theirs = Ed25519KeyPair::from_seed_unchecked(&s).unwrap();
        assert_eq!(
            ours.public_key().as_slice(),
            theirs.public_key().as_ref(),
            "key {i}"
        );
        let sig = ours.sign(&msg).unwrap();
        assert_eq!(sig.as_slice(), theirs.sign(&msg).as_ref(), "signature {i}");
        assert!(verify(ours.public_key(), &msg, &sig).is_ok());
        assert!(UnparsedPublicKey::new(&ED25519, ours.public_key())
            .verify(&msg, &sig)
            .is_ok());
    }
}

/// Seeds at the edges of the clamping and reduction logic.
#[test]
fn edge_seeds_match_ring() {
    for fill in [0x00u8, 0x01, 0x7f, 0x80, 0xfe, 0xff] {
        let s = [fill; 32];
        let ours = SigningKey::from_seed(&s).unwrap();
        let theirs = Ed25519KeyPair::from_seed_unchecked(&s).unwrap();
        let sig = ours.sign(b"edge").unwrap();
        assert_eq!(
            sig.as_slice(),
            theirs.sign(b"edge").as_ref(),
            "seed {fill:#04x}"
        );
    }
}

/// A changed message or key gives a signature that no longer verifies.
#[test]
fn a_signature_is_bound_to_its_message_and_key() {
    let a = SigningKey::from_seed(&[1; 32]).unwrap();
    let b = SigningKey::from_seed(&[2; 32]).unwrap();
    let sig = a.sign(b"message").unwrap();
    assert!(verify(a.public_key(), b"message", &sig).is_ok());
    assert!(verify(a.public_key(), b"messagf", &sig).is_err());
    assert!(verify(b.public_key(), b"message", &sig).is_err());
}

#[test]
fn generate_uses_the_source_and_reports_its_failure() {
    let key = SigningKey::generate(|out: &mut [u8]| {
        out.fill(7);
        Ok::<(), ()>(())
    })
    .unwrap();
    assert_eq!(
        key.public_key(),
        SigningKey::from_seed(&[7; 32]).unwrap().public_key()
    );
    let failed = SigningKey::generate(|_: &mut [u8]| Err::<(), _>("no entropy"));
    assert_eq!(failed.unwrap_err(), GenerateError::Source("no entropy"));
}

#[test]
fn debug_does_not_print_the_key() {
    let key = SigningKey::from_seed(&[9; 32]).unwrap();
    assert_eq!(format!("{key:?}"), "SigningKey(..)");
}
