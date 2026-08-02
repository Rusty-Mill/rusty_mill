//! The hand-rolled TLS engine — **not** the engine this crate uses.
//!
//! Everything in this module is an alternative implementation behind the
//! seam, permanently opt-in and permanently non-default. `rustls` is what
//! [`TlsStream`](crate::TlsStream), [`TlsConnector`](crate::TlsConnector),
//! and [`TlsAcceptor`](crate::TlsAcceptor) use, and nothing here changes
//! that. See `docs/adr/0002-handrolled-engine-behind-a-permanently-non-default-seam.md`
//! for the binding version of that promise, including why it does not expire
//! once the tests pass.
//!
//! # Why this is behind two gates
//!
//! Reaching this module needs both the `handrolled-engine` cargo feature and
//! `--cfg rusty_tls_handrolled`. That is not belt-and-braces: cargo features
//! are *unified* across a dependency graph, so any crate in a consumer's tree
//! could otherwise enable this for everyone else in that build. A `--cfg`
//! flag comes from `RUSTFLAGS` — set by whoever runs `cargo`, unreachable
//! from a dependency. The cfg is what makes "you cannot end up here by
//! accident" true rather than aspirational.
//!
//! # What is here
//!
//! - [`record`] — the TLS 1.3 record layer (stage 1): AEAD protection and
//!   framing for an already-established connection.
//! - [`der`] — a strict DER reader (stage 2a), the foundation everything
//!   certificate-shaped sits on.
//! - [`x509`] — certificate parsing (stage 2a). **Parsing only**: it reports
//!   what a certificate says and decides nothing about whether to believe it.
//!
//! Path validation (stage 2b), the 1.3 handshake (3), TLS 1.2 (4), and the
//! server side (5) are not built. The ADR carries the order and the bar each
//! must clear.
//!
//! # What is deliberately *not* here
//!
//! Cryptographic primitives. AES-GCM, ChaCha20-Poly1305, X25519, SHA-2, and
//! P-256 come from `ring`. Hand-rolling those is a different project with a
//! correctness property — constant-time execution — that none of this
//! module's testing strategy can see.

pub mod der;
pub mod record;
pub mod x509;
