//! ChaCha20-Poly1305 (RFC 8439), with opt-in AES-128/256-GCM.
//!
//! Portable Rust: no `unsafe`, no allocation. ChaCha20 is add-rotate-xor and
//! Poly1305 is 5x26-bit multiplication, so neither branches or indexes on
//! secret data; the tag is compared with
//! [`rusty_crypto_key::constant_time_eq`]. That is the design, checked with
//! valgrind taint runs, a pinned disassembly budget and a timing test (see
//! `scripts/ct_check.sh`). It is **not proven**. The owner attested on 2026-10-23 that the primitive
//! reviews and AES integration review were complete; see the repository
//! `docs/research/CRYPTO-REPLACEMENT-PLAN.md` section 11b.
//!
//! # Features
//!
//! `aes-gcm` (off by default) exposes `Aes128Gcm`, `Aes256Gcm`,
//! `GCM_NONCE_LEN` and `GCM_TAG_LEN` and compiles their AES/GHASH implementation.
//! Enable it with `rusty_aead = { version = "0.1", features = ["aes-gcm"] }`.
//! ChaCha20-Poly1305 and Poly1305 remain available without it.
//! This is an API opt-in, not a security boundary or an unfinished implementation:
//! Cargo features are additive, so another dependency (including `rusty_tls`'s
//! `native-crypto` feature) can enable AES-GCM for the whole resolved crate.
//!
//! ```
//! use rusty_aead::ChaCha20Poly1305;
//! let cipher = ChaCha20Poly1305::new(&[7u8; 32]);
//! let nonce = [1u8; 12];
//! let mut buf = *b"hello";
//! let tag = cipher.seal_in_place(&nonce, b"header", &mut buf).unwrap();
//! assert!(cipher.open_in_place(&nonce, b"header", &mut buf, &tag).is_ok());
//! assert_eq!(&buf, b"hello");
//! ```

#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate std;

mod aead;
#[cfg(feature = "aes-gcm")]
mod aes;
mod chacha20;
#[cfg(feature = "aes-gcm")]
mod gcm;
#[cfg(feature = "aes-gcm")]
mod ghash;
mod poly1305;

pub use aead::{ChaCha20Poly1305, Error, KEY_LEN, NONCE_LEN, TAG_LEN};
#[cfg(feature = "aes-gcm")]
pub use gcm::{Aes128Gcm, Aes256Gcm, GCM_NONCE_LEN, GCM_TAG_LEN};
pub use poly1305::Poly1305;
