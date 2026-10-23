//! Valgrind taint run for the secret-consuming entry points (HMAC keys, HKDF
//! input keying material). Modes: `all` (must report 0 errors) and `control`
//! (compares the tag with `==` and branches on it; must report at least 1, or
//! the run proves nothing). See `scripts/ct_check.sh`.

use rusty_ct_check::taint::{declassify, mark_secret};
use rusty_sha2::{extract, Hmac, Sha256, Sha384, Sha512};
use std::hint::black_box;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let key = black_box([0x42u8; 48]);
    let msg = black_box([0x17u8; 200]);
    mark_secret(&key);
    match mode.as_str() {
        "all" => {
            black_box(Hmac::<Sha256>::mac(&key, &msg));
            black_box(Hmac::<Sha384>::mac(&key, &msg));
            black_box(Hmac::<Sha512>::mac(&key, &msg));
            let prk = extract::<Sha256>(Some(&key), &key);
            let mut okm = [0u8; 100];
            let _ = prk.expand(&msg, &mut okm);
            // The derived output is secret too; never branch on it.
            black_box(okm);
            // Verification: the tag is public, the key is not. `verify` must
            // not branch on key-dependent data except through its final,
            // declassified boolean.
            let tag = Hmac::<Sha256>::mac(&key, &msg);
            declassify(&tag);
            let ok = Hmac::<Sha256>::verify(&key, &msg, &tag);
            declassify(&[ok as u8]);
            black_box(ok);
        }
        "control" => {
            let tag = Hmac::<Sha256>::mac(&key, &msg);
            if tag[0] == black_box(0) {
                black_box(1);
            }
        }
        _ => {
            eprintln!("modes: all | control");
            std::process::exit(2);
        }
    }
}
