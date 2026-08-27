//! Fuzz Uri normalization and conversions: arbitrary strings must never
//! panic, and normalization must be idempotent.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_lsp::lsp::Uri;

fuzz_target!(|data: &str| {
    let uri = Uri::new(data);
    // Idempotence: normalizing a normalized URI is a no-op.
    assert_eq!(Uri::new(uri.as_str()), uri);
    let _ = uri.scheme();
    let _ = uri.to_file_path();
    let _ = uri.parent();
    let _ = uri.join("segment");
});
