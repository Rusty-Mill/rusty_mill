//! ChaCha20-Poly1305 (RFC 8439).
//!
//! Portable Rust: no `unsafe`, no allocation. ChaCha20 is add-rotate-xor and
//! Poly1305 is 5x26-bit multiplication, so neither branches or indexes on
//! secret data; the tag is compared with
//! [`rusty_crypto_key::constant_time_eq`]. That is the design, checked with
//! valgrind taint runs, a pinned disassembly budget and a timing test (see
//! `scripts/ct_check.sh`). It is **not proven**, and nothing here has been
//! independently reviewed.
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

mod aead;
mod chacha20;
mod poly1305;

pub use aead::{ChaCha20Poly1305, Error, KEY_LEN, NONCE_LEN, TAG_LEN};
pub use poly1305::Poly1305;
