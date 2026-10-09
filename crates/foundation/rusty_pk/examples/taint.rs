//! Valgrind taint run for the secret-scalar code. The scalar is secret, the
//! peer's u-coordinate is public. Modes:
//!
//! - `x25519`: `x25519` and `public_key` must report 0 errors.
//! - `agree`: reports exactly 1, in `agree`: the all-zero check branches on
//!   the (tainted) result, which reveals only whether the *peer* key had small
//!   order. It cannot be declassified from outside the library.
//! - `control`: a deliberate branch on the secret; must be caught.
//!
//! See `scripts/ct_check.sh`.

use rusty_ct_check::taint::mark_secret;
use rusty_pk::x25519::{agree, public_key, x25519};
use std::hint::black_box;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let scalar = black_box([0x77u8; 32]);
    mark_secret(&scalar);
    let mut peer = black_box([0x11u8; 32]);
    peer[0] = 9;
    match mode.as_str() {
        "x25519" => {
            black_box(x25519(&scalar, &peer));
            black_box(public_key(&scalar));
        }
        "agree" => {
            black_box(agree(&scalar, &peer).is_ok());
        }
        "control" => {
            if x25519(&scalar, &peer)[0] == black_box(0) {
                black_box(1);
            }
        }
        _ => {
            eprintln!("modes: x25519 | agree | control");
            std::process::exit(2);
        }
    }
}
