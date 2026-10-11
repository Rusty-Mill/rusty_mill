//! Valgrind taint run for AES-GCM: key and plaintext are secret; no branch or address may
//! depend on them. Modes: `gcm128-seal`, `gcm256-seal`, `gcm128-open` (exactly one report,
//! the public tag verdict), `control-table` (a secret-indexed table lookup; must be caught).

use rusty_aead::{Aes128Gcm, Aes256Gcm};
use rusty_ct_check::taint::{declassify, mark_secret};
use std::hint::black_box;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let key128 = black_box([0x42u8; 16]);
    let key256 = black_box([0x42u8; 32]);
    let mut plaintext = black_box([0x17u8; 2000]);
    mark_secret(&key128);
    mark_secret(&key256);
    mark_secret(&plaintext);
    let nonce = [9u8; 12];
    let aad = b"associated data";
    match mode.as_str() {
        "gcm128-seal" => {
            let tag = Aes128Gcm::new(&key128)
                .seal_in_place(&nonce, aad, &mut plaintext)
                .unwrap();
            declassify(&plaintext);
            declassify(&tag);
            black_box((plaintext, tag));
        }
        "gcm256-seal" => {
            let tag = Aes256Gcm::new(&key256)
                .seal_in_place(&nonce, aad, &mut plaintext)
                .unwrap();
            declassify(&plaintext);
            declassify(&tag);
            black_box((plaintext, tag));
        }
        "gcm128-open" => {
            let mut sealed = plaintext;
            let tag = Aes128Gcm::new(&[0x42u8; 16])
                .seal_in_place(&nonce, aad, &mut sealed)
                .unwrap();
            declassify(&sealed);
            declassify(&tag);
            let ok = Aes128Gcm::new(&key128).open_in_place(&nonce, aad, &mut sealed, &tag);
            declassify(&[ok.is_ok() as u8]);
            black_box((ok.is_ok(), sealed));
        }
        // The leak class table-based AES has: an S-box lookup indexed by a secret byte.
        "control-table" => {
            let mut table = [0u8; 256];
            for (i, t) in table.iter_mut().enumerate() {
                *t = (i as u8).wrapping_mul(7).wrapping_add(1);
            }
            let idx = black_box(&key128)[0] ^ black_box(&plaintext)[0];
            black_box(table[idx as usize]);
        }
        _ => {
            eprintln!("modes: gcm128-seal | gcm256-seal | gcm128-open | control-table");
            std::process::exit(2);
        }
    }
}
