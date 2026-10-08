//! Coverage-guided fuzzing of the TLS 1.2 record layer.
//!
//! Two properties, both checked on every input:
//!
//! 1. **Opening arbitrary bytes never panics.** The remote can send anything,
//!    so a panic here is a denial of service reachable by any peer. A random
//!    record is overwhelmingly refused at the framing checks; the fuzzer's
//!    coverage guidance is what gets inputs past them to the AEAD.
//! 2. **Seal then open is the identity.** Whatever fragment, content type and
//!    sequence number the fuzzer picks, a record this crate seals must open
//!    under the same key, to the same type and bytes, and the sequence number
//!    must advance by exactly one on each side.
//!
//! Property 2 is what makes the target worth running for hours: it explores
//! lengths and sequence numbers no hand-written test thought to try.
//!
//!   RUSTFLAGS='--cfg rusty_tls_handrolled' cargo +nightly fuzz run record12_open

#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_tls::handrolled::record::{Aead, ContentType};
use rusty_tls::handrolled::record12::{fixed_iv_len, Opener, Sealer};

const ALGS: [Aead; 3] = [Aead::Aes128Gcm, Aead::Aes256Gcm, Aead::ChaCha20Poly1305];

fuzz_target!(|data: &[u8]| {
    // Byte 0 picks the algorithm, 1..9 the sequence number, 9 the content type.
    let [selector, s0, s1, s2, s3, s4, s5, s6, s7, typ, rest @ ..] = data else {
        return;
    };
    let alg = ALGS[usize::from(*selector) % ALGS.len()];
    let seq = u64::from_be_bytes([*s0, *s1, *s2, *s3, *s4, *s5, *s6, *s7]);
    let key = vec![0x42u8; alg.key_len()];
    let fixed = vec![0x17u8; fixed_iv_len(alg)];

    // 1. Hostile bytes: must be an `Err` or an `Ok`, never a panic.
    if let Ok(mut opener) = Opener::new_at(alg, &key, &fixed, seq) {
        let _ = opener.open(rest);
    }

    // 2. Round trip. Skip lengths the sealer is documented to refuse.
    let Ok(mut sealer) = Sealer::new_at(alg, &key, &fixed, seq) else {
        return;
    };
    let typ = ContentType::from_u8(*typ);
    let Ok(record) = sealer.seal(typ, rest) else {
        assert!(rest.len() > 1 << 14, "sealer refused an in-bounds fragment");
        return;
    };
    let mut opener = Opener::new_at(alg, &key, &fixed, seq).expect("same key builds");
    let opened = opener.open(&record).expect("a record we sealed must open");
    assert_eq!(opened.typ, typ);
    assert_eq!(opened.fragment, rest);
    assert_eq!(sealer.sequence(), seq.checked_add(1));
    assert_eq!(opener.sequence(), seq.checked_add(1));
});
