//! dudect-style timing tests. Statistical and machine-dependent, so ignored
//! by default; run in the scheduled job with
//! `cargo test -p rusty_aead --release -- --ignored --nocapture`.
//! A pass means "no leak detected here", nothing more.
//!
//! Method: class-dependent setup runs in `prepare` (untimed) into a buffer both
//! classes share, so the timed code and the addresses it touches are identical.
//! `docs/research/crypto-evidence/` explains why: a version that set up inside
//! the timed region "detected" leaks that were not there.

use rusty_aead::ChaCha20Poly1305;
use rusty_ct_check::timing::{leak_statistic_split, THRESHOLD};
use std::cell::RefCell;
use std::hint::black_box;

fn check(name: &str, t: Option<f64>) {
    let t = t.expect("enough samples");
    eprintln!("{name}: |t| = {t:.2}");
    assert!(t < THRESHOLD, "{name}: |t| = {t}");
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn key_classes_not_distinguishable() {
    let fixed = black_box([0u8; 32]);
    let mut other = [0u8; 32];
    for (i, b) in other.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37).wrapping_add(11);
    }
    let other = black_box(other);
    let key = RefCell::new([0u8; 32]);
    let t = leak_statistic_split(
        100_000,
        1,
        |class| {
            key.borrow_mut()
                .copy_from_slice(if class { &other } else { &fixed })
        },
        || {
            let mut buf = [0x55u8; 256];
            ChaCha20Poly1305::new(black_box(&key.borrow()))
                .seal_in_place(&[0; 12], b"aad", &mut buf)
                .unwrap()
        },
    );
    check("seal: fixed key vs other key", t);
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn plaintext_classes_not_distinguishable() {
    let cipher = ChaCha20Poly1305::new(&[7u8; 32]);
    let buf = RefCell::new([0u8; 256]);
    let t = leak_statistic_split(
        100_000,
        2,
        |class| buf.borrow_mut().fill(if class { 0xff } else { 0 }),
        || {
            cipher
                .seal_in_place(&[1; 12], b"", black_box(&mut *buf.borrow_mut()))
                .unwrap()
        },
    );
    check("seal: all-zero vs all-ones plaintext", t);
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn tag_mismatch_position_not_distinguishable() {
    let cipher = ChaCha20Poly1305::new(&[7u8; 32]);
    let mut sealed = [0x11u8; 256];
    let tag = cipher.seal_in_place(&[1; 12], b"", &mut sealed).unwrap();
    // Both classes are rejected (so decryption is skipped in both); they
    // differ only in where the wrong byte sits.
    let (bad, copy) = (RefCell::new(tag), RefCell::new(sealed));
    let t = leak_statistic_split(
        100_000,
        3,
        |class| {
            let mut b = bad.borrow_mut();
            *b = tag;
            b[if class { 15 } else { 0 }] ^= 1;
            *copy.borrow_mut() = sealed;
        },
        || {
            let tag = *bad.borrow();
            cipher
                .open_in_place(
                    &[1; 12],
                    b"",
                    black_box(&mut *copy.borrow_mut()),
                    black_box(&tag),
                )
                .is_err()
        },
    );
    check("open: first-byte vs last-byte tag mismatch", t);
}

/// A/A calibration: identical work in both classes, so any `|t|` is noise.
/// Read every other result against this machine's false-positive rate.
#[test]
#[ignore = "calibration; scheduled job only"]
fn null_calibration_identical_classes() {
    let cipher = ChaCha20Poly1305::new(&[7u8; 32]);
    let buf = RefCell::new([0u8; 256]);
    let t = leak_statistic_split(
        100_000,
        4,
        |_| buf.borrow_mut().fill(0x5a),
        || {
            cipher
                .seal_in_place(&[1; 12], b"", black_box(&mut *buf.borrow_mut()))
                .unwrap()
        },
    )
    .expect("enough samples");
    eprintln!("A/A seal: |t| = {t:.2}");
}
