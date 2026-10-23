//! Valgrind taint run: key, nonce-independent plaintext and the one-time
//! Poly1305 key are secret; no branch or address may depend on them.
//! Modes: `seal`, `open`, `control` (branches on a secret; must be caught).
//! See `scripts/ct_check.sh` for what each must report.

use rusty_aead::ChaCha20Poly1305;
use rusty_ct_check::taint::{declassify, mark_secret};
use std::hint::black_box;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let key = black_box([0x42u8; 32]);
    let mut plaintext = black_box([0x17u8; 200]);
    mark_secret(&key);
    mark_secret(&plaintext);
    let cipher = ChaCha20Poly1305::new(&key);
    let nonce = [9u8; 12];
    match mode.as_str() {
        // Key and plaintext are secret; sealing must not branch on them.
        "seal" => {
            let tag = cipher
                .seal_in_place(&nonce, b"associated data", &mut plaintext)
                .unwrap();
            declassify(&plaintext);
            declassify(&tag);
            black_box((plaintext, tag));
        }
        // Opening reveals one public bit (accept or reject) by branching on
        // the tag comparison inside the library, which cannot be declassified
        // from outside. Exactly that one report is expected, in
        // `open_in_place`; anything else is a leak. The decrypted plaintext is
        // secret and is only written after the verdict.
        "open" => {
            let mut sealed = plaintext;
            let tag = ChaCha20Poly1305::new(&[0x42u8; 32])
                .seal_in_place(&nonce, b"associated data", &mut sealed)
                .unwrap();
            declassify(&sealed);
            declassify(&tag);
            let ok = cipher.open_in_place(&nonce, b"associated data", &mut sealed, &tag);
            declassify(&[ok.is_ok() as u8]);
            black_box((ok.is_ok(), sealed));
        }
        "control" => {
            let _ = cipher.seal_in_place(&nonce, b"", &mut plaintext);
            if plaintext[0] == black_box(0) {
                black_box(1);
            }
        }
        _ => {
            eprintln!("modes: seal | open | control");
            std::process::exit(2);
        }
    }
}
