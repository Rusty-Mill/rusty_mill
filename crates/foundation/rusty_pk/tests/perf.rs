//! Verification speed against `ring`, on the first valid Wycheproof vector of
//! each kind. Ignored (machine-dependent); run with
//! `cargo test -p rusty_pk --release --test perf -- --ignored --nocapture`.

use ring::signature::{self, UnparsedPublicKey, VerificationAlgorithm};
use rusty_ct_check::wycheproof::{cases, Verdict};
use rusty_pk::{ecdsa, ed25519, rsa};
use std::time::Instant;

fn time<F: FnMut() -> bool>(iters: u32, mut f: F) -> f64 {
    assert!(f(), "vector must verify");
    let start = Instant::now();
    for _ in 0..iters {
        std::hint::black_box(f());
    }
    start.elapsed().as_secs_f64() * 1e6 / iters as f64
}

fn first_valid(json: &str) -> rusty_ct_check::wycheproof::Case {
    cases(json)
        .unwrap()
        .into_iter()
        .find(|c| c.verdict == Verdict::Valid)
        .unwrap()
}

fn report(name: &str, ours: f64, ring: f64) {
    eprintln!(
        "{name:<22} ours {ours:>8.1} us   ring {ring:>7.1} us   {:>5.1}x",
        ours / ring
    );
}

#[test]
#[ignore = "timing; run manually"]
fn verification_speed() {
    for (bits, json, alg) in [
        (
            2048,
            include_str!("vectors/rsa_signature_2048_sha256_test.json"),
            &signature::RSA_PKCS1_2048_8192_SHA256,
        ),
        (
            3072,
            include_str!("vectors/rsa_signature_3072_sha256_test.json"),
            &signature::RSA_PKCS1_2048_8192_SHA256,
        ),
        (
            4096,
            include_str!("vectors/rsa_signature_4096_sha256_test.json"),
            &signature::RSA_PKCS1_2048_8192_SHA256,
        ),
    ] {
        let c = first_valid(json);
        let (k, m, s) = (
            c.hex("publicKeyAsn").unwrap(),
            c.hex("msg").unwrap(),
            c.hex("sig").unwrap(),
        );
        let ours = time(200, || {
            rsa::verify(rsa::Scheme::Pkcs1Sha256, &k, &m, &s).is_ok()
        });
        let ring = time(200, || {
            UnparsedPublicKey::new(alg, &k).verify(&m, &s).is_ok()
        });
        report(&format!("RSA-{bits} PKCS1 SHA256"), ours, ring);
    }
    type Row = (
        &'static str,
        &'static str,
        ecdsa::CurveId,
        ecdsa::Digest,
        &'static dyn VerificationAlgorithm,
        usize,
    );
    let rows: [Row; 2] = [
        (
            "ECDSA P-256 SHA256",
            include_str!("vectors/ecdsa_secp256r1_sha256_test.json"),
            ecdsa::CurveId::P256,
            ecdsa::Digest::Sha256,
            &signature::ECDSA_P256_SHA256_ASN1,
            65,
        ),
        (
            "ECDSA P-384 SHA384",
            include_str!("vectors/ecdsa_secp384r1_sha384_test.json"),
            ecdsa::CurveId::P384,
            ecdsa::Digest::Sha384,
            &signature::ECDSA_P384_SHA384_ASN1,
            97,
        ),
    ];
    for (name, json, curve, digest, oracle, point) in rows {
        let c = first_valid(json);
        let spki = c.hex("publicKeyDer").unwrap();
        let (k, m, s) = (
            &spki[spki.len() - point..],
            c.hex("msg").unwrap(),
            c.hex("sig").unwrap(),
        );
        let ours = time(100, || ecdsa::verify(curve, digest, k, &m, &s).is_ok());
        let ring = time(100, || {
            UnparsedPublicKey::new(oracle, k).verify(&m, &s).is_ok()
        });
        report(name, ours, ring);
    }
    let c = first_valid(include_str!("vectors/ed25519_test.json"));
    let spki = c.hex("publicKeyDer").unwrap();
    let (k, m, s) = (
        &spki[spki.len() - 32..],
        c.hex("msg").unwrap(),
        c.hex("sig").unwrap(),
    );
    let ours = time(200, || ed25519::verify(k, &m, &s).is_ok());
    let ring = time(200, || {
        UnparsedPublicKey::new(&signature::ED25519, k)
            .verify(&m, &s)
            .is_ok()
    });
    report("Ed25519", ours, ring);
}
