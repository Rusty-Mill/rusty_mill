# ADR-0048: `Memory` domain — a bounded projection of the consumer's `memories` table

- Status: **Accepted as designed** (promoted from Proposed on
  2026-09-06 — the owner's "Accept as designed", option (a): the
  eleven-field projection on `Reminder`'s stack, no wire change; (b)
  the whole table, (c) `memory_associations` now, (d) replacement
  first, and (e) decline all declined. Recorded in "Acceptance and
  implementation" below.) Proposed and implemented on one branch, the
  `ADR-0046`/`ADR-0047` cadence.
- Date: 2026-09-06
- Deciders: baileyrd
- Related: `docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md` (the full
  design), `ADR-0036` (scoped `Memory` out; the stack shape reused),
  `ADR-0039`/`ADR-0041` (open-string classification, `StrList`),
  `ADR-0045` (gate (ii), met by this round), `ADR-0046`/`ADR-0047`
  (runtime records and edges — the prerequisites).
- Supersedes/Superseded by: none. Adds one domain; no wire change.

## Context

`rusty_remind_me`'s `memories` table is the thing the consumer is
about, and every prior round either scoped it out (`ADR-0036`: "a
genuinely different kind of database") or pointed at it as next.
Since `ADR-0046`/`ADR-0047` a running store can gain records and
edges, so a `memories` table would no longer be a fixture nobody can
add to. Reading the consumer's source again (`schema_tables.sql`,
`add_memory`, `list_filters`, the `access_count` bump) shows that
eleven of its thirty columns are what a memory *is* at creation and
what every list reads back; the other nineteen are scoring floats,
nullable columns, sync bookkeeping, chunking, soft deletion, and two
edge tables — each blocked by a wire-level gap this crate has not
designed (float, null, directed edge, deletion) or owned by a named
later round (cross-table links, whole-record replacement).

## Decision

Add `Memory` as this crate's sixth domain and third front-door one, as
a **bounded eleven-field projection** with `Reminder`'s one-index,
one-scan stack: `category` indexed (open `String`), `access_count`
scannable and durably mutable (non-negative), `tags` a `StrList`,
`metadata_json` carried as unparsed text, everything else read-only
after insert, no relation of either kind. A `MemoryConnectionStore`
adapter, a `memory_server` binary, and integration tests proving the
consumer's read and write shapes over a socket. **No wire change**:
protocol 14, `SERVER-002` 0.3.0, every vector untouched.

Named and deferred, in order: whole-record replacement (the
consumer's `update_memory` — the next round), then the `ADR-0045`
implementation round with `memory_entities` as its cross-table link,
for which `Memory` is now the second table gate (ii) asked for.

## Consequences

- Positive: the consumer's `add_memory`/`get`/`list`/`access_count`
  paths have a backend today, built from what the library already
  has; `ADR-0045`'s gate is met, so the multi-table round can be
  scheduled.
- Positive: zero wire cost — a sixth domain and the `Hello` table
  still ends at 14.
- Named, not hidden: `update_memory` has no backend until the
  replacement round; a memory's `content` cannot change after insert.
- Named, not hidden: the scoring triple, the SPO triple,
  `superseded_by`, soft deletion, and both edge tables are outside
  this projection, each for a stated reason.
- `sensitive` is stored and filterable, not enforced — the consumer's
  policy layer, as today.

## Considered options

**(a) Accept as designed** — the eleven-field projection, `Reminder`'s
stack, no wire change. **(b) The whole table** — four wire designs
(float, null, directed edge, deletion) before one memory is stored.
**(c) Also `memory_associations` as a `MultiSymmetric` now** — cheap
alone, but changes the stack twice once `memory_entities` needs
`ADR-0045`. **(d) Replacement first** — a table that can be inserted
into and read is useful before it can be edited. **(e) Decline.**

## Acceptance and implementation

- 2026-09-06: proposed and implemented on the same branch as
  `SERVER-001` v0.38.0 / FR-048 — `src/generic/memory.rs`,
  `src/server/memory.rs`, `src/bin/memory_server.rs`,
  `tests/server_memory_integration.rs`, `CheckpointFlush` in
  `src/server/journal.rs`; one unit test under default features, four
  in the adapter, three over sockets; every acceptance criterion 1–4
  holds. (PR #200.)
- 2026-09-06: accepted as designed (option (a); (b)–(e) declined).
  No change to the implementation. The replacement round
  (`ADR-0049`) starts next, as this ADR named.
