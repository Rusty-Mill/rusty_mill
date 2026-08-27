//! SHA-1 (RFC 3174 / FIPS 180-1) for git's own object-hashing scheme.
//!
//! Git's object model is content-addressed by SHA-1 — this is a
//! compatibility requirement for reading/writing real git repositories,
//! not a claim of cryptographic security. Do not reuse this for anything
//! security-sensitive.
//!
//! Re-exported from `rusty_sha1`, shared with `rusty_term`'s WebSocket
//! handshake — both crates needed the same hand-rolled algorithm for
//! unrelated compatibility reasons, so it lives in one place.

pub use rusty_sha1::{hex, sha1, Sha1, SHA1_DIGEST_LEN};
