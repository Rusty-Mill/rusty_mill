//! `rusty_stream` — single-node durable log for RustyMill.
//!
//! Phase 1 scope: append-only segment log + in-memory offset index
//! (`segment`), rolled and retained by size/time (`retention`), on
//! `rusty_tokio`'s `thread-per-core`/`io-uring-fs` runtime (ADR-0002 D3),
//! with storage I/O built directly on its `OpDriver`/`SimDriver` seam rather
//! than a parallel hand-rolled trait (D4), and its own wire protocol
//! (`protocol`) built on `rusty_wire` rather than Kafka compatibility (D1).
//! See `docs/phase1-scope.md` for the full scope and `docs/adr/0002-*.md`
//! for why each foundational decision landed where it did.
//!
//! Explicitly out of scope for Phase 1 (`docs/phase1-scope.md` §2):
//! multi-broker replication, Kafka wire-protocol compatibility, WASM
//! transforms, consumer-group rebalancing.

pub mod clock;
pub mod consumer;
pub mod offset;
pub mod protocol;
pub mod record;
pub mod retention;
pub mod segment;
pub mod server;

pub use clock::{Clock, SimClock, SystemClock};
pub use consumer::ConsumerOffsets;
pub use offset::{CommittedOffset, DurableOffset, Epoch, Offset};
pub use protocol::{ProtocolError, Request, Response};
pub use retention::{Log, RetentionPolicy};
pub use segment::Segment;
pub use server::serve;
