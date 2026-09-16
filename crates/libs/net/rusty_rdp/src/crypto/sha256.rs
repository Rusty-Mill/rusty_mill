//! Re-exports the shared SHA-256 implementation from `rusty_rsa`, which
//! also backs `rusty_oauth`'s independent SHA-256. The two crates'
//! hand-rolled implementations were near-identical duplication --
//! extracted into `rusty_rsa` (see that crate's own docs).
//!
//! Needed here for the CredSSP (MS-CSSP) version 5+ public-key binding,
//! which hashes a nonce together with the server's TLS public key.

pub use rusty_rsa::sha256;

/// Length of a SHA-256 digest in bytes.
pub const SHA256_DIGEST_LEN: usize = 32;
