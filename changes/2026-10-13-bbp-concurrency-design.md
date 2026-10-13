---
category: Added
changelog: **BBP concurrency design note.** Splits BBP into a serialized control plane and a concurrent content plane, classifies each message and artifact kind, and proposes multi-writer append (2a) before an in-tree CRDT for co-edited specs (2b); design only.
---
## 2026-10-13 - BBP concurrency design note

- **Added:** `docs/bbp-concurrency-design.md`. Control plane stays serialized by the log's revision check; content plane allows concurrent writers. Covers several live turns (one per principal) and their effect on R14, deadlines and the `turns` budget, a snapshot-hash rule that voids verdicts when a co-edited artifact changes, reserve-before-spend budget slices, and op-based CRDT ops stored in the existing event log, using a shared first-party `rusty_crdt` (RGA only, zero-dependency core) seeded from `nexus-crdt`; includes a crate API sketch and an ADR draft. No new Role field.
- **Known limitations:** design only, awaiting approval; contention, log growth and line-granularity sufficiency are unmeasured and labelled as such in the note.
