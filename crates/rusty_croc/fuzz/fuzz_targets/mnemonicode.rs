#![no_main]
//! Fuzz the mnemonic encoder: any byte slice must encode without panicking
//! and produce exactly the required number of words.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let words = rusty_croc::mnemonicode::encode_word_list(data);
    assert_eq!(words.len(), rusty_croc::mnemonicode::words_required(data.len()));
});
