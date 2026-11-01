# ADR-0139: Paged record storage in log-structured segments; codec ships first

- Status: **Proposed** (2026-11-01)
- Date: 2026-11-01
- Deciders: baileyrd
- Related: `ADR-0138` decision 2 (storage before features; amended here),
  `docs/design/STORAGE-PAGED-RECORDS-DESIGN.md` (the design),
  `ADR-0066` (migration to a fresh path), `ADR-0136` (chunked snapshots)

## Context

ADR-0138 put paged storage next, so records no longer have to fit in RAM.
It also planned to ship the `bincode` 1 replacement (RUSTSEC-2025-0141)
with it, "as one format migration rather than two".

Three designs were drafted and reviewed (two Elephant Goldfish passes and
an expert storage review). The reviews found that:

- Fixed-size pages with a buffer pool cost far more than the engine's
  access pattern needs. Records are read by id and written whole.
- The codec can be replaced without changing a single byte, so it needs
  no migration at all.

## Decision

1. **Record bodies move into append-only segment files** with an in-RAM key
   directory. Compaction rewrites one segment at a time. This is option C
   in the design. Edges, derived indexes and the slot file are unchanged.
2. **No new abstraction up front.** The engine keeps a private
   `Bodies { Map, Segments }` enum, with no heap trait, record cache or
   builders. Each of these is added only if a measured gate fails.
3. **Opt-in, then a default flip.** Consumers opt in one PR at a time
   through `create_segments`/`open_segments` (or the `_segments` helpers),
   migrating to a fresh path. The plain helpers switch to segments only
   once every in-repo consumer has migrated.
4. **Measure first (phase M).** Every consumer's footprint is measured
   before rollout, and the measurements set the order. The same numbers
   decide whether remind-me's full-text index is paged next (owner's
   choice, 2026-11-01).
5. **Amends ADR-0138 decision 2.** The `bincode` replacement ships first,
   on its own (phase 0), as a first-party `serde` codec. It is
   byte-identical to bincode 1.3.3 with the engine's options, which golden
   vectors and a differential test against bincode prove. It needs no
   format migration, so there is no second migration to merge it with.

## Consequences

- One format migration (the segment layout), done per consumer. Until the
  consumer's first write on the new layout, rollback is swapping back to the
  untouched source path; after it, rollback is the reverse tool
  `migrate_to_map`, which ships alongside the forward one.
- Segment stores never panic on a read failure: `get` poisons the store and
  returns `None`, and consumers move to the fallible `try_get` before they
  opt in.
- `bincode` leaves the normal dependency graph in phase 0, and remains only
  as a dev-only test oracle.
- The root ADR-0009 "Replace `bincode`" row is updated in the phase 0 PR.
- Full-text and other derived indexes still rebuild in RAM on open. Paging
  them gets its own design, timed by phase M.
