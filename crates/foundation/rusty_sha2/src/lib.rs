//! SHA-256, SHA-384, SHA-512, HMAC and HKDF.
//!
//! Portable Rust: no `unsafe`, no allocation, no dependencies beyond the
//! workspace's wipe routine. Hash and MAC state is wiped on drop; tags are
//! compared with [`rusty_crypto_key::constant_time_eq`].
//!
//! # What this is not
//!
//! The compression functions have no data-dependent branches or indexes by
//! construction (round constants are indexed by round number, never by data).
//! That has been checked by reading and with the vectors; it has **not** been
//! shown constant-time by any tool. It is not a replacement for a reviewed
//! library, and SHA-2 itself is not a password hash.
//!
//! ```
//! use rusty_sha2::{Hash, Sha256};
//! let digest = Sha256::digest(b"abc");
//! assert_eq!(digest[0], 0xba);
//! ```

#![no_std]
#![forbid(unsafe_code)]

mod block;
mod constants;
mod hkdf;
mod hmac;
mod sha256;
mod sha512;

pub use hkdf::{extract, ExpandError, Prk};
pub use hmac::{Hmac, HmacSha256, HmacSha384, HmacSha512};
pub use sha256::Sha256;
pub use sha512::{Sha384, Sha512};

/// A streaming hash function with a fixed block and output size.
pub trait Hash: Clone {
    /// Input block size in bytes (the HMAC block length).
    const BLOCK_LEN: usize;
    /// Digest size in bytes.
    const OUTPUT_LEN: usize;
    /// The digest, as a fixed-size array.
    type Output: Copy + AsRef<[u8]> + AsMut<[u8]>;

    /// A fresh hasher.
    fn new() -> Self;
    /// Absorbs `data`.
    fn update(&mut self, data: &[u8]);
    /// Finishes and returns the digest.
    fn finalize(self) -> Self::Output;

    /// One-shot digest of `data`.
    fn digest(data: &[u8]) -> Self::Output {
        let mut hasher = Self::new();
        hasher.update(data);
        hasher.finalize()
    }
}
