//! dudect-style timing tests for X25519. Statistical and machine-dependent,
//! so ignored by default; run in the scheduled job with
//! `cargo test -p rusty_pk --release --test timing -- --ignored --nocapture`.
//! A pass means "no leak detected here", nothing more.

use rusty_ct_check::timing::{leak_statistic, THRESHOLD};
use rusty_pk::x25519::x25519;
use std::hint::black_box;

fn check(name: &str, t: Option<f64>) {
    let t = t.expect("enough samples");
    eprintln!("{name}: |t| = {t:.2}");
    assert!(t < THRESHOLD, "{name}: |t| = {t}");
}

fn point() -> [u8; 32] {
    let mut u = [0x42u8; 32];
    u[31] &= 0x7f;
    u
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn scalar_classes_not_distinguishable() {
    // Few set bits vs many set bits: the classic square-and-add leak shape.
    let sparse = [0u8; 32];
    let dense = [0xffu8; 32];
    let u = point();
    let t = leak_statistic(20_000, 1, |class| {
        let k = if class {
            black_box(&dense)
        } else {
            black_box(&sparse)
        };
        x25519(k, &u)
    });
    check("x25519: sparse vs dense scalar", t);
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn fixed_vs_random_scalar() {
    let fixed = [0x55u8; 32];
    let mut other = [0u8; 32];
    for (i, b) in other.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(97).wrapping_add(13);
    }
    let u = point();
    let t = leak_statistic(20_000, 2, |class| {
        let k = if class {
            black_box(&other)
        } else {
            black_box(&fixed)
        };
        x25519(k, &u)
    });
    check("x25519: fixed vs other scalar", t);
}
