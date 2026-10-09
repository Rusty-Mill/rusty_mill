//! Valgrind taint run for the secret-scalar code. The scalar is secret, the
//! peer's u-coordinate is public. Modes:
//!
//! - `x25519`: `x25519` and `public_key` must report 0 errors.
//! - `agree`: reports exactly 1, in `agree`: the all-zero check branches on
//!   the (tainted) result, which reveals only whether the *peer* key had small
//!   order. It cannot be declassified from outside the library.
//! - `ecdh256`, `ecdh384`: a secret ECDH scalar; `public_key` and `agree` (own
//!   public key, peer point public) must report 0 errors. The peer's validation
//!   and the infinity check branch on public data only.
//! - `control`: a deliberate branch on the secret; must be caught.
//! - `ecdh-control`: a deliberate branch on the ECDH shared secret; must be caught.
//!
//! See `scripts/ct_check.sh`.

use rusty_ct_check::taint::mark_secret;
use rusty_pk::ecdh::{Curve, PrivateKey};
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
        "ecdh256" => ecdh(Curve::P256),
        "ecdh384" => ecdh(Curve::P384),
        "ecdh-control" => {
            let (key, peer) = ecdh_keys(Curve::P256);
            if key.agree(&peer).unwrap().as_bytes()[0] == black_box(0) {
                black_box(1);
            }
        }
        _ => {
            eprintln!("modes: x25519 | agree | control | ecdh256 | ecdh384 | ecdh-control");
            std::process::exit(2);
        }
    }
}

/// A key whose scalar bytes are tainted (a tainted scalar is the secret), plus a
/// valid public peer point.
fn ecdh_keys(curve: Curve) -> (PrivateKey, Vec<u8>) {
    let mut bytes = vec![0x77u8; curve.len()];
    bytes[0] = 0x01; // below both group orders
    let peer = PrivateKey::from_bytes(curve, &[0x05u8; 48][..curve.len()])
        .unwrap()
        .public_key()
        .unwrap();
    // Validation reads the scalar (range check), which is public in this model:
    // taint after the key exists, on its own copy of the bytes.
    let key = PrivateKey::from_bytes(curve, &bytes).unwrap();
    mark_secret(key.scalar_bytes_for_taint_run());
    (key, peer.as_bytes().to_vec())
}

fn ecdh(curve: Curve) {
    let (key, peer) = ecdh_keys(curve);
    black_box(key.public_key().unwrap().as_bytes()[0]);
    black_box(key.agree(&peer).is_ok());
}
