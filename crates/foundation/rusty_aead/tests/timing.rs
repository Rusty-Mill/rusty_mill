//! dudect-style timing tests. Statistical and machine-dependent, so ignored
//! by default; run in the scheduled job with
//! `cargo test -p rusty_aead --release -- --ignored --nocapture`.
//! A pass means "no leak detected here", nothing more.

use rusty_aead::ChaCha20Poly1305;
use rusty_ct_check::timing::{leak_statistic, THRESHOLD};
use std::hint::black_box;

fn check(name: &str, t: Option<f64>) {
    let t = t.expect("enough samples");
    eprintln!("{name}: |t| = {t:.2}");
    assert!(t < THRESHOLD, "{name}: |t| = {t}");
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn key_classes_not_distinguishable() {
    let fixed = [0u8; 32];
    let mut other = [0u8; 32];
    for (i, b) in other.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37).wrapping_add(11);
    }
    let t = leak_statistic(100_000, 1, |class| {
        let key = if class {
            black_box(&other)
        } else {
            black_box(&fixed)
        };
        let mut buf = [0x55u8; 256];
        ChaCha20Poly1305::new(key)
            .seal_in_place(&[0; 12], b"aad", &mut buf)
            .unwrap()
    });
    check("seal: fixed key vs other key", t);
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn plaintext_classes_not_distinguishable() {
    let cipher = ChaCha20Poly1305::new(&[7u8; 32]);
    let t = leak_statistic(100_000, 2, |class| {
        let mut buf = if class { [0xffu8; 256] } else { [0u8; 256] };
        cipher
            .seal_in_place(&[1; 12], b"", black_box(&mut buf))
            .unwrap()
    });
    check("seal: all-zero vs all-ones plaintext", t);
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn tag_mismatch_position_not_distinguishable() {
    let cipher = ChaCha20Poly1305::new(&[7u8; 32]);
    let mut buf = [0x11u8; 256];
    let tag = cipher.seal_in_place(&[1; 12], b"", &mut buf).unwrap();
    // Both classes are rejected (so decryption is skipped in both); they
    // differ only in where the wrong byte sits.
    let t = leak_statistic(100_000, 3, |class| {
        let mut bad = tag;
        bad[if class { 15 } else { 0 }] ^= 1;
        let mut copy = buf;
        cipher
            .open_in_place(&[1; 12], b"", black_box(&mut copy), &bad)
            .is_err()
    });
    check("open: first-byte vs last-byte tag mismatch", t);
}
