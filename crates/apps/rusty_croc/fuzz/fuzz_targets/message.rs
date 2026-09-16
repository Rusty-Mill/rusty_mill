#![no_main]
//! Fuzz the control-message envelope decoder (decompress + JSON), both with
//! and without an encryption key. Must never panic.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Unencrypted path: arbitrary bytes → DEFLATE → JSON.
    let _ = rusty_croc::message::decode(None, data);

    // Encrypted path with a fixed key: exercises the AES-GCM open + the
    // above. Wrong-key/short inputs must error, not panic.
    let (key, _) = rusty_croc::crypt::new_key(b"pass123", Some(b"saltsalt")).unwrap();
    let _ = rusty_croc::message::decode(Some(&key), data);
});
