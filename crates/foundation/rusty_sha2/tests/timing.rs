//! dudect-style timing test, key class: fixed key vs a different key, same
//! message. Statistical and machine-dependent, so ignored by default; run in
//! the scheduled job with `cargo test -p rusty_sha2 --release -- --ignored`.
//! A pass means "no leak detected here", nothing more.

use rusty_ct_check::timing::{leak_statistic, THRESHOLD};
use rusty_sha2::{Hmac, Sha256, Sha512};
use std::hint::black_box;

fn run<H: rusty_sha2::Hash>(name: &str) {
    let fixed = [0u8; 32];
    let msg = [0x11u8; 256];
    let mut other = [0u8; 32];
    for (i, b) in other.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37).wrapping_add(11);
    }
    let t = leak_statistic(200_000, 99, |class| {
        let key = if class {
            black_box(&other)
        } else {
            black_box(&fixed)
        };
        Hmac::<H>::mac(key, &msg)
    })
    .expect("enough samples");
    eprintln!("{name}: |t| = {t:.2}");
    assert!(t < THRESHOLD, "{name}: |t| = {t}");
}

#[test]
#[ignore = "timing-sensitive; scheduled job only"]
fn hmac_key_classes_not_distinguishable() {
    run::<Sha256>("HMAC-SHA256");
    run::<Sha512>("HMAC-SHA512");
}
