---
category: Added
changelog: New `rusty_crdt` foundation crate, a dependency-free sequence CRDT (RGA) with property-tested CRDT laws.
---
## 2026-10-26 - rusty_crdt: a sequence CRDT (RGA)

- **Added:** `crates/foundation/rusty_crdt`, an RGA seeded from `nexus-crdt`'s algorithm. It is sync, has no I/O, has no dependencies (ADR-0002 Tier S) and is generic over id and element types. Fallible calls return `Result`. A local insert whose id would render in the wrong place is refused (`NotFresh`). An id reused with a different parent or value is refused (`IdReused`). This detects the conflict that `nexus-crdt` let pass silently; it cannot reconcile replicas that each accepted a different first op, so convergence still assumes unique ids. An op naming an unknown element is reported as `Pending` and is not dropped. Tombstones keep their value, so a replay is told apart from a reuse after a delete.
- **Added:** proptest laws over random three-site histories: convergence under any delivery order with duplicates, idempotence, commutativity of ready ops, batch associativity, tombstone replay, a single-site `Vec` model, and no panics on arbitrary ops.
- **Known limitations:** nothing depends on it yet. Re-pointing `rusty_bbp` and `nexus-crdt` are separate changes, and the Nexus one is blocked while `FROZEN.md` stands. Positional lookups are O(n). There is no `rusty_serde` feature until a consumer needs one.
