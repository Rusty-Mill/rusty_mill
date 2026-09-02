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
//!   specifically), `OffsetFetch`/`OffsetCommit` (a consumer group's
//!   committed offsets, read and write -- `OffsetCommit` at v2, see
//!   [`protocol::offset_commit`]'s module doc for why; both share
//!   [`protocol::offset_fetch`]'s coordinator-routing caveat), `Produce`
//!   (at v3) and `Fetch` (at v4) -- the versions
//!   [`protocol::produce`]/[`protocol::fetch`] each explain, both
//!   needing the record batch v2 wire format (see [`record_batch`]) --
//!   and the rest of consumer-group coordination, `FindCoordinator`/
//!   `JoinGroup`/`SyncGroup`/`Heartbeat`/`LeaveGroup`, every one at v0.
//! - **What "consumer-group coordination" covers here, and what it
//!   doesn't**: this crate implements the full *wire protocol* for
//!   joining, syncing, heartbeating, leaving a group, and committing/
//!   fetching offsets within one -- including the embedded
//!   `ConsumerProtocolSubscription`/`ConsumerProtocolAssignment`
//!   payload format `JoinGroup`/`SyncGroup` carry
//!   ([`protocol::consumer_protocol`]). It does **not** include the
//!   *partition-assignment decision* a group's elected leader must
//!   make in `SyncGroup` (deciding which member gets which partition,
//!   the way `librdkafka`'s "range"/"roundrobin" assignors do) --
//!   that's policy for whichever caller drives the join/sync/
//!   heartbeat/fetch loop (`rusty-meshed-sdk`'s future consumer poll
//!   loop) to supply, the same layering [`crate::protocol::create_topics`]'s
//!   raw `CreateTopics` vs. `rusty-meshed-sdk::TopicManager`'s naming-
//!   convention enforcement already draws.
//! - **`Produce`/`Fetch`/record batch v2 have no live broker to
//!   validate against** in this environment (see [`record_batch`]'s
//!   own module doc for the verification approach used instead --
//!   hand-checking every field against the published spec, plus a
//!   CRC-32C implementation cross-checked against the standard
//!   Castagnoli test vector). Treat this path, and the rest of the
//!   consumer-group coordination layered on top of it, with more
//!   caution than the rest of the crate until it's been run against a
//!   real broker at least once.
//! - **Single-connection, no pipelining**: [`KafkaClient`] sends one
//!   request and awaits its response before sending the next, over one
//!   connection. No multiplexing/pipelining, and no controller/leader
//!   discovery -- every request goes to whichever broker it's connected
//!   to (including `FindCoordinator`'s own result -- see
//!   [`protocol::find_coordinator`]'s module doc), matching meshed's
//!   own only real deployment target (a single all-in-one KRaft node
//!   in local dev; see `meshed/compose.yaml`).
//!
//! The [`protocol`] module's request/response types are pure
//! encode/decode, independent of the network layer, and are the most
//! rigorously tested part of this crate (hand-verified byte sequences)
//! since they can't be checked against a live broker here.

pub mod client;
pub mod error;
mod frame;
pub mod protocol;
pub mod record_batch;
pub mod testing;
mod wire;

pub use client::{KafkaClient, DEFAULT_MAX_FRAME_LEN};
pub use error::{ClientError, CodecError};
