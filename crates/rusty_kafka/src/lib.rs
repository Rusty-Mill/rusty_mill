//! A hand-rolled Kafka wire-protocol client for the RustyMill ecosystem:
//! producer, consumer, and admin APIs over Kafka's binary protocol.
//!
//! Built on [`rusty_wire`]'s byte-cursor primitives for message framing
//! and [`rusty_tokio`] for the async I/O adapter, rather than wrapping
//! `librdkafka`/`rdkafka` -- the platform's own protocol-ownership
//! convention (see `rusty_http`, `rusty_tls`).
//!
//! # Scope of this first pass
//!
//! This crate is a prerequisite for `crates/rusty_meshed`'s migration
//! (see `../rusty_meshed/capability-manifest.md`), scoped narrowly on
//! purpose rather than attempting a full client in one pass:
//!
//! - **Implemented**: `ApiVersions` (broker capability discovery),
//!   `Metadata` (broker/topic discovery), `CreateTopics` (what
//!   `TopicManager` needs), `ListOffsets` (watermark/timestamp lookup,
//!   at v1 -- see [`protocol::list_offsets`]'s module doc for why v1
//!   specifically), `OffsetFetch` (a consumer group's committed
//!   offset, with a coordinator-routing caveat -- see
//!   [`protocol::offset_fetch`]'s module doc) -- v0 for every one of
//!   these except `ListOffsets`.
//! - **Not yet implemented**: `Produce`/`Fetch` and the rest of
//!   consumer-group coordination (`FindCoordinator`/`JoinGroup`/
//!   `SyncGroup`/`Heartbeat`/`OffsetCommit`).
//!   Modern Kafka brokers require the record-batch v2 message format
//!   (varint-encoded, CRC32C-checksummed) for current `Produce` API
//!   versions, and likely require flexible (compact/tagged-field)
//!   request encoding at the versions actually accepted by the
//!   `confluent-local` broker meshed's `compose.yaml` runs --
//!   implementing that correctly without a live broker in this
//!   environment to validate against would be shipping unverified wire
//!   code, so it's deferred to a follow-up pass with that verification
//!   available.
//! - **Single-connection, no pipelining**: [`KafkaClient`] sends one
//!   request and awaits its response before sending the next, over one
//!   connection. No multiplexing/pipelining, and no controller/leader
//!   discovery -- every request goes to whichever broker it's connected
//!   to, matching meshed's own only real deployment target (a single
//!   all-in-one KRaft node in local dev; see `meshed/compose.yaml`).
//!
//! The [`protocol`] module's request/response types are pure
//! encode/decode, independent of the network layer, and are the most
//! rigorously tested part of this crate (hand-verified byte sequences)
//! since they can't be checked against a live broker here.

pub mod client;
pub mod error;
mod frame;
pub mod protocol;
pub mod testing;
mod wire;

pub use client::{KafkaClient, DEFAULT_MAX_FRAME_LEN};
pub use error::{ClientError, CodecError};
