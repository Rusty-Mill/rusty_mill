//! dudect-style timing tests for X25519, ECDH P-256 / P-384, Ed25519 signing and ECDSA P-256 signing. Statistical and machine-dependent,
//! so ignored by default; run in the scheduled job with
//! `cargo test -p rusty_pk --release --test timing -- --ignored --nocapture`.
//! A pass means "no leak detected here", nothing more.
//!
//! Method: class-dependent setup runs in `prepare` (untimed) into a buffer both
//! classes share (see `rusty_aead/tests/timing.rs`).

use rusty_ct_check::timing::{leak_statistic_split, THRESHOLD};
use rusty_pk::ecdh::{Curve, PrivateKey};
use rusty_pk::ecdsa::{self, CurveId};
use rusty_pk::ed25519::SigningKey;
use rusty_pk::x25519::x25519;
use std::cell::RefCell;
use std::hint::black_box;

fn check(name: &str, t: Option<f64>) {
    let t = t.expect("enough samples");
    eprintln!("{name}: |t| = {t:.2}");
    assert!(t < THRESHOLD, "{name}: |t| = {t}");
}

fn two_scalar_classes(a: [u8; 32], b: [u8; 32], seed: u64) -> Option<f64> {
    let (a, b) = (black_box(a), black_box(b));
    let mut u = [0x42u8; 32];
    u[31] &= 0x7f;
    let k = RefCell::new([0u8; 32]);
    leak_statistic_split(
        20_000,
        seed,
        |class| k.borrow_mut().copy_from_slice(if class { &b } else { &a }),
        || x25519(black_box(&k.borrow()), &u),
    )
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn scalar_classes_not_distinguishable() {
    // Few set bits vs many set bits: the classic square-and-add leak shape.
    check(
        "x25519: sparse vs dense scalar",
        two_scalar_classes([0u8; 32], [0xffu8; 32], 1),
    );
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn fixed_vs_random_scalar() {
    let mut other = [0u8; 32];
    for (i, b) in other.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(97).wrapping_add(13);
    }
    check(
        "x25519: fixed vs other scalar",
        two_scalar_classes([0x55u8; 32], other, 2),
    );
}

/// A/A calibration: identical scalars in both classes, so any `|t|` is noise.
#[test]
#[ignore = "calibration; scheduled job only"]
fn null_calibration_identical_classes() {
    let t = two_scalar_classes([0x55u8; 32], [0x55u8; 32], 3).expect("enough samples");
    eprintln!("A/A x25519: |t| = {t:.2}");
}

/// ECDH `agree` with two scalar classes. Scalars are built below the group order
/// (top byte 0x01 or 0x7f, rest as given) so both classes are valid keys. As the header
/// says, the class is chosen in `prepare` (untimed), which loads that class's key into the
/// one slot that the timed closure always reads, so both classes run the identical code on
/// the same storage.
fn ecdh_classes(curve: Curve, fill_a: u8, fill_b: u8, seed: u64, samples: usize) -> Option<f64> {
    let make = |fill: u8| {
        let mut bytes = vec![fill; curve.len()];
        bytes[0] = if fill == 0xff { 0x7f } else { fill & 0x7f };
        bytes
    };
    let (a, b) = (make(fill_a), make(fill_b));
    let slot = RefCell::new(PrivateKey::from_bytes(curve, &a).expect("valid scalar"));
    let peer = PrivateKey::from_bytes(curve, &vec![0x05; curve.len()])
        .unwrap()
        .public_key()
        .unwrap();
    leak_statistic_split(
        samples,
        seed,
        |class| {
            let bytes = if class { &b } else { &a };
            *slot.borrow_mut() = PrivateKey::from_bytes(curve, bytes).expect("valid scalar");
        },
        || {
            slot.borrow()
                .agree(black_box(peer.as_bytes()))
                .map(|s| s.as_bytes()[0])
        },
    )
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn ecdh_p256_sparse_vs_dense_scalar() {
    check(
        "ecdh p256: sparse vs dense scalar",
        ecdh_classes(Curve::P256, 0x01, 0xff, 11, 4_000),
    );
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn ecdh_p384_sparse_vs_dense_scalar() {
    check(
        "ecdh p384: sparse vs dense scalar",
        ecdh_classes(Curve::P384, 0x01, 0xff, 12, 2_000),
    );
}

/// A/A calibrations for ECDH, one per curve with that curve's own sample count (the same
/// counts as the leak tests), so each leak test is read against a baseline of its own size.
#[test]
#[ignore = "calibration; scheduled job only"]
fn ecdh_null_calibration_identical_classes() {
    let t = ecdh_classes(Curve::P256, 0x55, 0x55, 13, 4_000).expect("enough samples");
    eprintln!("A/A ecdh p256: |t| = {t:.2}");
}

#[test]
#[ignore = "calibration; scheduled job only"]
fn ecdh_p384_null_calibration_identical_classes() {
    let t = ecdh_classes(Curve::P384, 0x55, 0x55, 14, 2_000).expect("enough samples");
    eprintln!("A/A ecdh p384: |t| = {t:.2}");
}

/// Ed25519 `sign` under two keys, same public message. The class's key is built in `prepare`
/// (untimed) into the one slot the timed closure reads. Seeds of all zero and all ones bytes
/// expand (through SHA-512) to unrelated scalars and nonce prefixes, so this compares two
/// arbitrary secrets: a key-dependent branch or table index in the fixed-base
/// multiplications or the reductions would separate them.
fn ed25519_classes(seed_a: u8, seed_b: u8, seed: u64, samples: usize) -> Option<f64> {
    let slot = RefCell::new(SigningKey::from_seed(&[seed_a; 32]).expect("key"));
    leak_statistic_split(
        samples,
        seed,
        |class| {
            let fill = if class { seed_b } else { seed_a };
            *slot.borrow_mut() = SigningKey::from_seed(&[fill; 32]).expect("key");
        },
        || {
            slot.borrow()
                .sign(black_box(b"a public message"))
                .map(|s| s[63])
        },
    )
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn ed25519_sign_key_classes_not_distinguishable() {
    check(
        "ed25519 sign: zero-seed vs ones-seed key",
        ed25519_classes(0x00, 0xff, 21, 2_000),
    );
}

/// A/A calibration for Ed25519 signing, with the leak test's own sample count.
#[test]
#[ignore = "calibration; scheduled job only"]
fn ed25519_sign_null_calibration_identical_classes() {
    let t = ed25519_classes(0x55, 0x55, 22, 2_000).expect("enough samples");
    eprintln!("A/A ed25519 sign: |t| = {t:.2}");
}

/// ECDSA `sign` under two keys, same message and a fixed hedge (so both classes run the
/// identical deterministic path apart from the key). Keys are built in `prepare` (untimed)
/// into the one slot the timed closure reads: a sparse scalar (0x01 bytes) and a dense one
/// (0x7f, then 0xff bytes, last byte 0x04).
///
/// The timed region includes DER encoding, whose length depends on the published `r` and
/// `s`. With fixed noise, different DER lengths can cause a public-output timing
/// gap. These fixtures are required to give equal lengths; changing nonce derivation
/// requires rechecking this assertion and rerunning the timing evidence.
fn ecdsa_classes(curve: CurveId, a: &[u8], b: &[u8], seed: u64, samples: usize) -> Option<f64> {
    let slot = RefCell::new(ecdsa::SigningKey::from_bytes(curve, a).expect("valid scalar"));
    let sign = |key: &ecdsa::SigningKey| {
        key.sign(black_box(b"a public message"), |h: &mut [u8]| {
            h.fill(0x5a);
            Ok::<(), ()>(())
        })
    };
    let (sa, sb) = (sign(&slot.borrow()).ok()?, {
        let k = ecdsa::SigningKey::from_bytes(curve, b).expect("valid scalar");
        sign(&k).ok()?
    });
    assert_eq!(
        sa.len(),
        sb.len(),
        "classes must give equal-length signatures"
    );
    leak_statistic_split(
        samples,
        seed,
        |class| {
            let bytes = if class { b } else { a };
            *slot.borrow_mut() = ecdsa::SigningKey::from_bytes(curve, bytes).expect("valid scalar");
        },
        || sign(&slot.borrow()).map(|s| s.len()),
    )
}

fn sparse_and_dense_p256() -> (Vec<u8>, Vec<u8>) {
    let sparse = vec![0x01u8; 32];
    let mut dense = vec![0xffu8; 32];
    dense[0] = 0x7f;
    dense[31] = 0x04;
    (sparse, dense)
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn ecdsa_p256_sign_sparse_vs_dense_key() {
    let (sparse, dense) = sparse_and_dense_p256();
    check(
        "ecdsa p256 sign: sparse vs dense key",
        ecdsa_classes(CurveId::P256, &sparse, &dense, 31, 2_000),
    );
}

#[test]
#[ignore = "calibration; scheduled job only"]
fn ecdsa_p256_sign_null_calibration_identical_classes() {
    let (sparse, _) = sparse_and_dense_p256();
    let t = ecdsa_classes(CurveId::P256, &sparse, &sparse, 32, 2_000).expect("enough samples");
    eprintln!("A/A ecdsa p256 sign: |t| = {t:.2}");
}
