//! `rusty_stream` — single-node durable log for RustyMill.
//!
//! Phase 1 scope: append-only segment log + in-memory offset index
//! (`segment`), on `rusty_tokio`'s `thread-per-core`/`io-uring-fs` runtime
//! (ADR-0002 D3), with storage I/O built directly on its `OpDriver`/
//! `SimDriver` seam rather than a parallel hand-rolled trait (D4). See
//! `docs/phase1-scope.md` for the full scope and `docs/adr/0002-*.md` for
//! why each foundational decision landed where it did.
//!
//! Explicitly out of scope for Phase 1 (`docs/phase1-scope.md` §2):
//! multi-broker replication, Kafka wire-protocol compatibility, WASM
//! transforms, consumer-group rebalancing. `retention` and `consumer` below
//! are present as module stubs — their real shape is scoped separately, not
//! implied by this scaffold.

pub mod offset;
pub mod record;
pub mod segment;

pub mod consumer;
pub mod retention;

pub use offset::{CommittedOffset, DurableOffset, Epoch, Offset};
pub use segment::Segment;
