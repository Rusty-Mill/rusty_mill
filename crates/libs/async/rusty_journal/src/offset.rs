//! Offset and epoch primitives required by ADR-0002 D2, independent of
//! whether Phase 2 eventually picks VSR or Raft: both protocols need a
//! durable/committed split and an epoch-style fencing token, so those exist
//! from Phase 1 rather than being retrofitted onto the on-disk format later.

/// A record's position in a segment, in units of "records since the
/// segment's base offset" — matches Kafka's own offset semantics (see
/// `docs/phase1-scope.md` §5.2), not a byte position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Offset(pub u64);

/// An offset acknowledged as fsynced to disk (or, once a fault-injection
/// [`rusty_tokio::io::SimDriver`] is in play, acknowledged by whatever
/// durability the current fsync policy actually provides). Distinct from
/// [`CommittedOffset`] even in single-node mode — see ADR-0002 D2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DurableOffset(pub Offset);

/// The high-watermark offset: visible to consumers. In Phase 1 (single node,
/// no replication) this only ever trails [`DurableOffset`] by at most one
/// fsync batch — there's no replica to wait on yet — but the type exists
/// separately now so Phase 2 consensus can attach without a storage-format
/// migration (ADR-0002 D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommittedOffset(pub Offset);

/// A VSR view-number or a Raft term are both instances of this one
/// primitive (ADR-0002 D2) — stored in segment metadata so a future
/// consensus layer can attach without a format migration. Phase 1 (no
/// consensus yet) only ever has `Epoch(0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Epoch(pub u64);

impl Epoch {
    /// The only epoch that exists before Phase 2 clustering lands.
    pub const INITIAL: Epoch = Epoch(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_never_exceeds_durable_in_practice() {
        // Not an invariant this module enforces (that's the storage
        // engine's job once it exists) -- just documents the relationship
        // these two types encode, so a future reader doesn't have to
        // rediscover it from ADR-0002 D2.
        let durable = DurableOffset(Offset(10));
        let committed = CommittedOffset(Offset(10));
        assert!(committed.0 <= durable.0);
    }
}
