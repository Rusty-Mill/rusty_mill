//! ECDSA verification against Wycheproof and `ring`.

use ring::signature::{self, UnparsedPublicKey, VerificationAlgorithm};
use rusty_ct_check::wycheproof::{cases, Verdict};
use rusty_pk::ecdsa::{verify, CurveId, Digest};

macro_rules! vectors {
    ($name:literal) => {
        include_str!(concat!("vectors/", $name, ".json"))
    };
}

fn run(
    json: &str,
    curve: CurveId,
    digest: Digest,
    oracle: &'static dyn VerificationAlgorithm,
    point_len: usize,
) {
    let all = cases(json).unwrap();
    assert!(all.len() > 100, "file looks truncated");
    for case in all {
        // The SPKI ends with the BIT STRING holding the uncompressed point.
        let spki = case.hex("publicKeyDer").unwrap();
        let key = &spki[spki.len() - point_len..];
        let msg = case.hex("msg").unwrap();
        let sig = case.hex("sig").unwrap();
        let ours = verify(curve, digest, key, &msg, &sig).is_ok();
        let ring = UnparsedPublicKey::new(oracle, key)
            .verify(&msg, &sig)
            .is_ok();
        assert_eq!(
            ours, ring,
            "differs from ring: tcId {} ({})",
            case.id, case.comment
        );
        match case.verdict {
            Verdict::Valid => assert!(ours, "tcId {} should verify ({})", case.id, case.comment),
            Verdict::Invalid => assert!(!ours, "tcId {} must reject ({})", case.id, case.comment),
            Verdict::Acceptable => {}
        }
    }
}

#[test]
fn p256_sha256() {
    run(
        vectors!("ecdsa_secp256r1_sha256_test"),
        CurveId::P256,
        Digest::Sha256,
        &signature::ECDSA_P256_SHA256_ASN1,
        65,
    );
}

#[test]
fn p384_sha384() {
    run(
        vectors!("ecdsa_secp384r1_sha384_test"),
        CurveId::P384,
        Digest::Sha384,
        &signature::ECDSA_P384_SHA384_ASN1,
        97,
    );
}

#[test]
fn p384_sha256() {
    run(
        vectors!("ecdsa_secp384r1_sha256_test"),
        CurveId::P384,
        Digest::Sha256,
        &signature::ECDSA_P384_SHA256_ASN1,
        97,
    );
}

/// P-256 with SHA-384: Wycheproof has no file for this pairing and ring cannot
/// sign with it, so the vectors come from an independent Python signer
/// (`scripts/gen_ecdsa_p256_sha384.py`). The hash is truncated to 256 bits.
#[test]
fn p256_sha384_from_independent_signer() {
    let mut valid = 0;
    for line in include_str!("vectors/ecdsa_p256_sha384.txt").lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let key = rusty_hex::decode(f[1]).unwrap();
        let msg = rusty_hex::decode(f[2]).unwrap();
        let sig = rusty_hex::decode(f[3]).unwrap();
        let ok = verify(CurveId::P256, Digest::Sha384, &key, &msg, &sig).is_ok();
        assert_eq!(ok, f[0] == "valid", "{line}");
        valid += (f[0] == "valid") as usize;
        // The same signature must not verify under SHA-256: wrong digest.
        if f[0] == "valid" && !msg.is_empty() {
            assert!(verify(CurveId::P256, Digest::Sha256, &key, &msg, &sig).is_err());
        }
    }
    assert!(valid >= 20);
}
