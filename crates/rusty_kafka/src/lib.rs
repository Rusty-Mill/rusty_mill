//! A hand-rolled Kafka wire-protocol client for the RustyMill ecosystem:
//! producer, consumer, and admin APIs over Kafka's binary protocol.
//!
//! Built on [`rusty_wire`]'s byte-cursor primitives for message framing
//! and [`rusty_tokio`] for the async I/O adapter, rather than wrapping
//! `librdkafka`/`rdkafka` -- the platform's own protocol-ownership
//! convention (see `rusty_http`, `rusty_tls`).
//!
//! Scaffolding only so far; see `crates/rusty_meshed/capability-manifest.md`
//! in this workspace for the capabilities this crate needs to unblock.
