//! dudect-style timing tests for X25519. Statistical and machine-dependent,
//! so ignored by default; run in the scheduled job with
//! `cargo test -p rusty_pk --release --test timing -- --ignored --nocapture`.
//! A pass means "no leak detected here", nothing more.
//!
//! Method: class-dependent setup runs in `prepare` (untimed) into a buffer both
//! classes share (see `rusty_aead/tests/timing.rs`).

use rusty_ct_check::timing::{leak_statistic_split, THRESHOLD};
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
