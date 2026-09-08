# ADR-0059: A memory-only sorted index behind `Page`

- Status: **Accepted as designed** (promoted from Proposed on
  2026-09-08 — the owner's "accept as designed", option (a): a
  memory-only sorted index layer; (b) a second scannable slot, (c) a
  persisted index, and (d) decline declined. Recorded in "Acceptance
  and implementation" below.) Proposed and implemented on one branch,
  the `ADR-0046`–`ADR-0058` cadence.
- Date: 2026-09-08
- Deciders: baileyrd
- Related: `docs/design/SERVER-ORDERED-INDEX-DESIGN.md` (the full
  design), `ADR-0055` (`Page`, whose open question this answers),
  `ADR-0040` (`NameIndex`, the memory-only layer precedent),
  `RESULTS.md` "Regression check through v0.48.0" (the measurement).
- Supersedes/Superseded by: none. No wire, format, or file change.

## Context

`Page` was accepted (`ADR-0055`) as a full scan per page, "acceptable
at target scale, overridable", with "a cheaper `page` for `Memory`"
named as the round after a page time was measured and found wanting.
It was measured: 292 ms per page of 50 at 100K `Memory` records, 100 ms
after selecting by key (v0.48.1), two minutes then fifty seconds to
walk the table the way the hub's sync loop does — against 14 µs, flat,
for SQLite's indexed `ORDER BY … LIMIT`. The remaining cost was one
decode per record per page, which no scan-path change removes.

## Decision

A memory-only sorted index layer, `Ordered<S, R, Marker>`, over one
`OrderedField` of a record: a `BTreeSet<(key, id)>` built from the
records at open (as `NameIndex` builds its map), kept exact through
every write, answering `PageBy::page_by` as a range walk from the
cursor. `Memory`'s and `Relation`'s stacks wrap in it over
`updated_at_unix_ms`; their adapters answer a `Page` by that field from
the index and every other field from the scan. The file format, the
wire, and every constructor signature are unchanged.

## Consequences

- Positive: a page costs the page. Measured 152–160 µs per page of 50
  at 1K, 10K, and 100K records (from 0.63 ms / 5.1 ms / 100 ms); the
  100K walk from 50 s to 0.44 s.
- Positive: no migration, no new file, no wire change; `NameIndex`'s
  shape, so nothing new to reason about at open or on crash.
- Named, not hidden: rebuilt at every open, one decode per record
  (81 ms per 100K here); a persisted index is the day that matters.
- Named, not hidden: one field per stack, ascending only; descending
  and a range are one walk each of the same set, unrequested.
- Named, not hidden: a field that is both scannable and ordered must
  share one marker type, or an `update` leaves the index stale — a
  rule the layer's docs state and `Relation` follows.

## Considered options

**(a) Accept as designed** — a memory-only sorted index layer.
**(b) A second scannable slot for `updated_at`** — removes the decode,
keeps the scan (linear per page), and is a layout change. **(c) A
persisted ordered index** — a new file and crash story for a structure
that rebuilds in 81 ms. **(d) Decline.**

## Acceptance and implementation

- 2026-09-08: proposed and implemented on the same branch as
  `SERVER-001` v0.49.0 / FR-059 —
  `src/generic/{traits,query,store,production,memory,relation}.rs`,
  `src/server/{serve,memory,relation}.rs`; tests: `generic/memory` +1,
  `generic/relation` +1, `server_memory_integration` extended; every
  acceptance criterion 1–4 holds.
  (PR #225.)
- 2026-09-08: accepted as designed (option (a); (b)–(d) declined). No
  change to the implementation. The page question is closed; what the
  design leaves open is its own: descending order and a range on the
  wire, a persisted index when the open-time rebuild is noticed.
