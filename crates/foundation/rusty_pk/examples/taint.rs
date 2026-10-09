//! Valgrind taint run for the secret-scalar code. The scalar is secret, the
//! peer's u-coordinate is public. Modes:
//!
//! - `x25519`: `x25519` and `public_key` must report 0 errors.
//! - `agree`: reports exactly 1, in `agree`: the all-zero check branches on
//!   the (tainted) result, which reveals only whether the *peer* key had small
//!   order. It cannot be declassified from outside the library.
//! - `ecdh256`, `ecdh384`: a secret ECDH scalar marked secret *before* import, so
//!   `from_bytes` validation, `public_key` and `agree` are all covered. Every report must
//!   be in `ecdh::ensure`, the single branch on a secret-derived verdict (scalar valid;
//!   result not the point at infinity); none anywhere else.
//! - `ecdh-import256`, `ecdh-import384`: `from_bytes` on tainted accepted scalars (1, 2^64,
//!   2^128, a high-bit scalar, n-1) and rejected ones (0, n, all ones); same expectation.
//! - `ecdh-generate256`, `ecdh-generate384`: `generate` with a source that marks its output secret.
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
        "ecdh-import256" => import(Curve::P256, &N256),
        "ecdh-import384" => import(Curve::P384, &N384),
        "ecdh-generate256" => generate(Curve::P256),
        "ecdh-generate384" => generate(Curve::P384),
        "ecdh-control" => {
            let (key, peer) = ecdh_key(Curve::P256);
            if key.agree(&peer).unwrap().as_bytes()[0] == black_box(0) {
                black_box(1);
            }
        }
        _ => {
            eprintln!("modes: x25519 | agree | control | ecdh256 | ecdh384 | ecdh-import256 | ecdh-import384 | ecdh-generate256 | ecdh-generate384 | ecdh-control");
            std::process::exit(2);
        }
    }
}

const N256: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84, 0xf3, 0xb9, 0xca, 0xc2, 0xfc, 0x63, 0x25, 0x51,
];
const N384: [u8; 48] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xc7, 0x63, 0x4d, 0x81, 0xf4, 0x37, 0x2d, 0xdf,
    0x58, 0x1a, 0x0d, 0xb2, 0x48, 0xb0, 0xa7, 0x7a, 0xec, 0xec, 0x19, 0x6a, 0xcc, 0xc5, 0x29, 0x73,
];

/// A valid key imported from tainted bytes (the taint precedes validation), plus a public peer point.
fn ecdh_key(curve: Curve) -> (PrivateKey, Vec<u8>) {
    let mut bytes = vec![0x77u8; curve.len()];
    bytes[0] = 0x01; // below both group orders
    let bytes = black_box(bytes);
    mark_secret(&bytes);
    let key = PrivateKey::from_bytes(curve, &bytes).unwrap();
    let peer = PrivateKey::from_bytes(curve, &vec![0x05u8; curve.len()])
        .unwrap()
        .public_key()
        .unwrap();
    (key, peer.as_bytes().to_vec())
}

fn ecdh(curve: Curve) {
    let (key, peer) = ecdh_key(curve);
    black_box(key.public_key().unwrap().as_bytes()[0]);
    black_box(key.agree(&peer).is_ok());
}

/// Import accepted and rejected scalars, each marked secret before validation.
fn import(curve: Curve, n: &[u8]) {
    let len = curve.len();
    let mut cases: Vec<Vec<u8>> = Vec::new();
    for bit in [0usize, 64, 128, len * 8 - 1] {
        let mut k = vec![0u8; len];
        k[len - 1 - bit / 8] = 1 << (bit % 8); // 1, 2^64, 2^128, 2^(8*len-1)
        cases.push(k);
    }
    let mut n_minus_1 = n.to_vec();
    n_minus_1[len - 1] -= 1;
    cases.extend([n_minus_1, vec![0u8; len], n.to_vec(), vec![0xffu8; len]]);
    for case in cases {
        let case = black_box(case);
        mark_secret(&case);
        black_box(PrivateKey::from_bytes(curve, &case).is_ok());
    }
}

fn generate(curve: Curve) {
    let key = PrivateKey::generate(curve, |buf: &mut [u8]| -> Result<(), ()> {
        buf.fill(0x01);
        buf[0] = 0x00;
        let buf = black_box(buf);
        mark_secret(buf);
        Ok(())
    });
    black_box(key.is_ok());
}
