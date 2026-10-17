# ADR-0138: Unbounded append-only event history — what the engine would need, and why to keep it out

- Status: **Proposed** (no code; asks the owner to choose option (b) or (a)).
- Date: 2026-10-17
- Deciders: baileyrd
- Related: `docs/FUTURE-GROWTH.md` (open, close and idle memory are linear in
  the records; stores are pooled app-side), `ADR-0005` (WAL/snapshot
  durability), `ADR-0025`/`0026` (redo journal), `ADR-0096` (`Compact`),
  `ADR-0136` (chunked snapshots), `ADR-0124` (engine in libs).
- Supersedes/Superseded by: none.

## Context

`agent_cloud` (`baileyrd/agent_cloud`) needs a per-agent session history: an
append-only stream of events (tool calls, model messages, desktop actions) with
no natural upper bound, read by offset or tail and trimmed by age. It also needs
a small agent registry. The registry fits the engine. The history is the
question: could `rusty_multimodal_db_engine` hold it?

What the engine does today (read from `generic/mmap_store.rs` and
`generic/record_blob.rs`; the insert-log internals were not audited):

1. **Whole records in RAM.** `GenericMmapStore` keeps
   `records: HashMap<Id, Box<R>>` plus the index maps, so resident memory grows
   with the number of records and their size.
2. **The record blob is rewritten whole.** The companion blob is one write-once
   `Vec<R>` installed by temp-file-and-rename, so a rewrite costs the whole set.
3. **Retention is not expressible.** `delete` and `compact` operate on records
   one at a time; there is no segment or age-based drop.
4. **One writer.** `GenericProductionStore` serialises writers behind one
   `RwLock`, so a busy append stream contends with registry reads.
5. **Ids must be `Copy`** (`u32`, `i64`, `Uuid`), and `Record::get` returns an
   in-memory record.

## Options

**(a) Extend the engine with a cold tier.** Needed changes:
- resident entry = id + indexed/scan fields + `(segment, offset)`; bodies read
  from disk on demand (changes the `get` contract in `generic/traits.rs`);
- immutable, size-rotated segment files replacing the single blob, with
  per-segment compaction;
- age/size retention that drops whole segments;
- a bounded or per-segment `fulltext` index (it is fully in memory today);
- a separate append lock so history writes do not queue behind registry reads.
This is a rewrite of the store core, shared with `rusty_remind_me`'s hub
(`rusty_remind_me` ADR-0021). Risk and review cost are high.

**(b) Keep the engine as is; put history in a purpose-built log (recommended).**
The consumer owns a small append-only segment log (length-prefixed records, one
directory per stream, rotation by size, retention by deleting oldest segments,
sparse offset index). The engine keeps only bounded state: the registry and a
per-stream summary (last offset, counts). The `fulltext` index may cover bounded
summaries, not the stream.

**(c) Do nothing.** Acceptable only if history is bounded by policy (cap per
agent); then the existing engine suffices and `compact()` on a schedule applies.

## Decision requested

Adopt (b). Do not add a cold tier to the engine until two independent consumers
need unbounded, offset-addressed data; (a) then becomes a design of its own with
this ADR as input. Document the RAM-bound constraint prominently (the engine
README already lists it) so embedders choose deliberately.

## Consequences

- No change to engine code, file formats or `rusty_remind_me`.
- Consumers needing unbounded history carry their own log; the first is
  `agent_cloud`'s `ac-history` crate.
- Revisit trigger: a second consumer reimplements the same segment log, at which
  point hoist it into `libs/storage` as its own crate (not into the engine).

## Unverified

- Insert-log growth and replay cost under a sustained append workload.
- Whether `Record::get` could return a lazily loaded body without breaking
  `Indexed`/`Scanned` layers.
