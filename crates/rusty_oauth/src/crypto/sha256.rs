//! Re-exports the shared SHA-256 implementation from `rusty_rsa`, which
//! also backs `rusty_rdp`'s independent SHA-256. The two crates'
//! hand-rolled implementations were near-identical duplication --
//! extracted into `rusty_rsa` (see that crate's own docs).

pub use rusty_rsa::{sha256, Digest, Sha256};
