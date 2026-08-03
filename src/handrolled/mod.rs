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
//! - [`verify`] — certificate signature verification (stage 2b-i). The first
//!   piece here that decides anything: whether a certificate was signed by a
//!   given key. Authorship, never authority — it builds no chain, reads no
//!   clock, and checks no constraint.
//! - [`path`] — certification path building and validation (stage 2b-ii),
//!   plus `verify_peer_certificate`, the combined entry point a TLS client
//!   wants.
//! - [`name`] — server name matching and name constraints (stage 2b-iii).
//!   The piece that turns "a trusted CA issued this" into "this certificate
//!   is for the server I asked for".
//!
//! - [`schedule`] — the TLS 1.3 key schedule (stage 3a). Where every key the
//!   protocol uses comes from, and where they get bound to the handshake
//!   transcript that produced them.
//!
//! - [`wire`] — TLS's presentation language (stage 3b): fixed-width integers
//!   and length-prefixed vectors, read strictly and written so a length
//!   prefix can never disagree with what follows it.
//! - [`handshake`] — handshake messages and the transcript hash (stage 3b).
//!   Parses and encodes, and the two are inverses on the wire bytes, because
//!   the transcript covers what arrived rather than a re-encoding of it.
//!
//! - [`kx`] — ephemeral key exchange (stage 3c-i): X25519 and the two NIST
//!   curves, wrapped so a key can be used exactly once and the agreed secret
//!   never outlives the closure it is handed to.
//! - [`verify`] gains the TLS `SignatureScheme` namespace in 3c-i, alongside
//!   the X.509 one it has carried since 2b-i. They follow different rules —
//!   most sharply on where an ECDSA key's curve comes from — and that module's
//!   docs set the two side by side.
//!
//! The client state machine (3c-ii), TLS 1.2 (4), and the server side (5) are
//! not built. The ADR carries the order and the bar each must clear.
//!
//! # What is deliberately *not* here
//!
//! Cryptographic primitives. AES-GCM, ChaCha20-Poly1305, X25519, SHA-2, and
//! P-256 come from `ring`. Hand-rolling those is a different project with a
//! correctness property — constant-time execution — that none of this
//! module's testing strategy can see.

pub mod der;
pub mod handshake;
pub mod kx;
pub mod name;
pub mod path;
pub mod record;
pub mod schedule;
pub mod verify;
pub mod wire;
pub mod x509;
