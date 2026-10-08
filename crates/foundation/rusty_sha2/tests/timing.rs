//! dudect-style timing test, key class: fixed key vs a different key, same
//! message. Statistical and machine-dependent, so ignored by default; run in
//! the scheduled job with `cargo test -p rusty_sha2 --release -- --ignored`.
//! A pass means "no leak detected here", nothing more.
//!
//! Method: class-dependent setup runs in `prepare` (untimed) into one buffer
//! both classes share, so the timed code and the address it reads are
//! identical (see `rusty_aead/tests/timing.rs`).

use rusty_ct_check::timing::{leak_statistic_split, THRESHOLD};
use rusty_sha2::{Hmac, Sha256, Sha512};
use std::cell::RefCell;
use std::hint::black_box;

fn run<H: rusty_sha2::Hash>(name: &str) {
    let fixed = black_box([0u8; 32]);
    let msg = [0x11u8; 256];
    let mut other = [0u8; 32];
    for (i, b) in other.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37).wrapping_add(11);
    }
    let other = black_box(other);
    let key = RefCell::new([0u8; 32]);
    let t = leak_statistic_split(
        200_000,
        99,
        |class| {
            key.borrow_mut()
                .copy_from_slice(if class { &other } else { &fixed })
        },
        || Hmac::<H>::mac(black_box(&key.borrow()[..]), &msg),
    )
    .expect("enough samples");
    eprintln!("{name}: |t| = {t:.2}");
    assert!(t < THRESHOLD, "{name}: |t| = {t}");
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn hmac_sha256_key_classes_not_distinguishable() {
    run::<Sha256>("HMAC-SHA256");
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn hmac_sha512_key_classes_not_distinguishable() {
    run::<Sha512>("HMAC-SHA512");
}

/// A/A calibration: both classes do identical work, so any `|t|` is noise.
/// Measures how often this machine "detects" a leak that cannot exist, which
/// is the false-positive rate every other result must be read against.
#[test]
#[ignore = "calibration; scheduled job only"]
fn null_calibration_identical_classes() {
    let key = black_box([0x33u8; 32]);
    let msg = [0x11u8; 256];
    let k = RefCell::new([0u8; 32]);
    let t = leak_statistic_split(
        200_000,
        99,
        |_| k.borrow_mut().copy_from_slice(&key),
        || Hmac::<Sha512>::mac(black_box(&k.borrow()[..]), &msg),
    )
    .expect("enough samples");
    eprintln!("A/A HMAC-SHA512: |t| = {t:.2}");
}
