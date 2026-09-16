//! The one place this crate names its crypto provider.
//!
//! `ServerConfig::builder()`/`ClientConfig::builder()` fall back to
//! rustls's ambient, process-level provider lookup, which only works when
//! exactly one of rustls's `ring`/`aws-lc-rs` features is active across
//! the *whole build* -- not just this crate's own `Cargo.toml`. A
//! consumer whose dependency graph also happens to pull in `aws-lc-rs`
//! (directly, or transitively through some other crate) makes that lookup
//! ambiguous, and rustls panics rather than guessing. Building every
//! config through `builder_with_provider` here removes that ambient
//! dependency entirely: this crate always gets the `ring` provider it
//! was written and tested against, regardless of what else shares the
//! process.

use std::sync::Arc;

/// This crate's own explicit choice — see the `rustls` dependency's own
/// comment in `Cargo.toml` for why `ring` over `aws-lc-rs`.
pub(crate) fn ring_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}
