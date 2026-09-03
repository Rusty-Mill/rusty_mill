//! Re-exports the shared Base64 implementation from `rusty_base64`, which
//! also backs `rusty_acp`, `rusty-mcp`, and `rusty_a2a`. Those three crates
//! were each pulling the external `base64` crate for functionality this
//! module already had complete (encode/decode, standard and URL-safe
//! alphabets) -- extracted into `rusty_base64` (see that crate's own docs)
//! rather than have each crate keep reaching for the external one, or
//! reimplement this module's coverage locally.

pub use rusty_base64::{
    decode_standard, decode_url_safe, encode_standard, encode_url_safe, encode_url_safe_no_pad,
    DecodeError,
};
