# Server Ordered Index Design (Accepted)

- Status: **Accepted** (2026-09-08, `ADR-0059` option (a), as designed —
  a memory-only sorted index layer over one field, rebuilt at open,
  exact through every write; a second scannable slot, a persisted
  index, and declining all declined). Implemented on the same branch
  as `SERVER-001` v0.49.0 / FR-059, PR #225.
- Date: 2026-09-08
- Related: `ADR-0055`/`docs/design/SERVER-PAGE-DESIGN.md` (`Page`, the
  order, the "a cheaper `page`" open question this closes), `ADR-0040`
  (`NameIndex`, the memory-only index layer precedent, rebuilt at
  open), `ADR-0056`/`ADR-0058` (the two stacks this wraps),
  `RESULTS.md` "Regression check through v0.48.0" (the measurement
  that forced it).
- Supersedes/Superseded by: none. No wire, protocol, format, or file
  change.

## Purpose and scope

`Page` was measured for the first time this week: after selecting by
key (v0.48.1), one page of 50 over 100K `Memory` records still cost
100 ms — one decode per record per page — against 14 µs for SQLite's
indexed `ORDER BY … LIMIT`, flat in table size. The hub's sync loop
walks the whole table in pages, so at 100K rows a walk was 50 s. The
cost of a page must be the page, not the table.

Scope, exactly: an `OrderedField<Marker>` record trait (`ORD-FR-001`);
a `PageBy<R, Marker>` query trait (`ORD-FR-002`); an `Ordered<S, R,
Marker>` store layer — a memory-only `BTreeSet<(key, id)>` built from
the records at open and kept exact through every write (`ORD-FR-003`);
`Memory`'s and `Relation`'s stacks wrapped in it over
`updated_at_unix_ms` (`ORD-FR-004`); `ConnectionStore::page` on those
two adapters answering that field from the index and every other field
from the scan (`ORD-FR-005`).

## Non-goals

- **A durable index** — rebuilt from the records at open, one decode
  each (81 ms per 100K here); the file format is untouched, as
  `NameIndex`'s is. A persisted B-tree is the day open time is the
  problem.
- **An index per orderable field** — one field per stack, the one the
  consumer walks by. A second is a second `Ordered` wrap.
- **`Entity`** — no `updated_at`; the hub does not page it by time.
- **Descending order, `ORDER BY` in SQL, a range filter on the wire** —
  unchanged from `SERVER-PAGE-DESIGN.md`'s open questions; the index
  would serve all three.

## Context and terminology

- **Memory-only layer**: `NameIndex` (ADR-0040) established the shape —
  a wrapper over any store that reads every record once at wrap time,
  writes nothing, and keeps its own structure exact through `Insert`,
  `Replace`, `Delete`, forwarding everything else. `Ordered` is the
  same shape with a `BTreeSet` instead of a `HashMap`.
- **One marker per field**: a field that is both scannable
  (`UpdateField`) and ordered must use the same marker type for both,
  because the layer decides whether an `update` re-keys the record by
  comparing the two markers' `TypeId`s at run time — the one place the
  "is this marker that marker" question `traits.rs` explains coherence
  cannot ask is answered. `Relation`'s `UpdatedAtField` is both;
  `Memory`'s `updated_at_unix_ms` is not scannable, so it gets its own
  `UpdatedAtOrder`.

## Requirements

- `ORD-FR-001` **Trait.** `OrderedField<Marker>: Record { type Key:
  Ord + Copy; fn order_key(&self) -> Key; }` in `generic::traits`.
- `ORD-FR-002` **Query.** `PageBy<R, Marker>::page_by(&self, after:
  Option<(Key, Id)>, limit) -> Vec<Id>`: the ids whose `(key, id)` is
  strictly greater than `after`'s, ascending, the first `limit` — the
  order `page_ids` writes for the wire. `GenericProductionStore::page_by`
  reads it under the lock.
- `ORD-FR-003` **Layer.** `Ordered<S, R, Marker>`: `new(inner)` builds
  the set from `AllIds` + `GetById`; `Insert` adds the pair after the
  inner insert succeeds; `Replace` swaps old for new after; `Delete`
  removes after; `UpdateField` re-keys when `ScanMarker` is `Marker`,
  forwards untouched otherwise; `Compact`, `Detach`, `Flush`, `GetById`,
  `AllIds`, `FilterEq`, `ScanField`, `MultiNeighbors`, `Neighbors`,
  `Link`, `MultiLink` forwarded. `inner()` and `indexed_len()` exposed.
- `ORD-FR-004` **Stacks.** `MemoryProductionStack` =
  `Ordered<MultiSymmetric<…>, Memory, UpdatedAtOrder>`;
  `RelationProductionStack` = `Ordered<GenericMmapStore<…>, Relation,
  UpdatedAtField>`; every constructor wraps; the aliases keep every
  caller unchanged.
- `ORD-FR-005` **Adapter.** `MemoryConnectionStore::page` and
  `RelationConnectionStore::page`: `order_by == FIELD_UPDATED_AT` →
  `page_by` then `get` per id; any other field → `page_by_scan`, the
  trait default as a free function. `Malformed` for a cursor of another
  kind (unreachable past `validate_page`).

## Considered options

- **(a) A memory-only sorted index layer over one field, rebuilt at
  open — proposed.** `NameIndex`'s shape; no format change; the page
  costs the page; the consumer's walk is flat.
- **(b) A second scannable slot for `updated_at_unix_ms`** —
  `SERVER-PAGE-DESIGN.md`'s original suggestion. Removes the decode but
  keeps the scan: still linear per page (~10 ms at 100K, SQLite's own
  un-indexed number), and a layout change with `ADR-0056`'s migration
  cost. The wrong half of the gap.
- **(c) A persisted ordered index** — a new file and its crash-safety
  story for a structure that rebuilds in 81 ms per 100K records.
- **(d) Decline** — the walk stays linear per page.

## Proposed shape

`src/generic/traits.rs` (`OrderedField`), `src/generic/query.rs`
(`PageBy`), `src/generic/store.rs` (`Ordered` and its forwards),
`src/generic/production.rs` (`page_by`), `src/generic/memory.rs`
(`UpdatedAtOrder`, the stack), `src/generic/relation.rs` (the stack);
`src/server/serve.rs` (`page_by_scan`), `src/server/{memory,relation}.rs`
(`page`). No wire change.

## Data/state and invariants

- The set holds exactly one `(order_key(record), id)` per live record
  of `R`, before and after any insert, replace, guarded replace,
  update, delete, cascade, or compaction, and after any open.
- `page_by` over the set and `page_ids` over the same records' keys
  return the same ids in the same order for the same cursor and limit.

## Errors, failure, recovery, and observability

A memory structure: nothing to recover; a crash costs nothing but the
rebuild at the next open. A write that fails in the inner store leaves
the set untouched (every mutation happens after the inner call
succeeds). No new error code.

## Security, privacy, and compatibility

A read. No format, wire, or protocol change; every existing client and
directory unaffected; the aliases keep every constructor's signature.

## Acceptance criteria

1. Store, `Memory`: five seeded records with one tie page in
   `(updated_at, id)` order from a strict cursor; an insert lands by its
   stamp, a replace moves, an `access_count` update leaves the order,
   a delete removes; a portable reopen rebuilds the same order.
2. Store, `Relation`: an `update` of `updated_at_unix_ms` (the
   scannable slot) re-keys the index; the order survives the reopen.
3. Wire: every existing `Page` test passes unchanged over the index;
   a page by `created_at_unix_ms` (the scan path) agrees with one by
   `updated_at_unix_ms`.
4. Measured: one page of 50 over `updated_at_unix_ms` costs the same at
   1K, 10K, and 100K `Memory` records, and a full walk at 100K is under
   a second (`RESULTS.md`).

## Verification plan

`generic/memory` +1, `generic/relation` +1, `server_memory_integration`
extended; the full sweep; the scratch timing example re-run and its
table recorded.

## Traceability

- Roadmap: `SERVER-ORDERED-INDEX-DESIGN`, `SERVER-ORDERED-INDEX`.
  Decision: `ADR-0059`.
- Specification: `SERVER-001` v0.49.0 / `FR-059`.
- Requirements: `ORD-FR-001`–`005`.

## Open questions

- **Descending order and a range on the wire** — both are one walk of
  the same set (`range(..cursor).rev()`, `range(a..b)`); a `Request`
  field each, if the consumer asks.
- **Open-time rebuild at scale** — 81 ms per 100K records here; a
  persisted index the day a directory is large enough to notice.
- **`Employee`/`Order`/`Dog`** — the reference domains keep the scan
  path; a wrap each, if a caller wants one.

## Change history

- 2026-09-08: Accepted as designed (option (a)), after PR #225. No
  content change.
- 2026-09-08: Implemented as `SERVER-001` v0.49.0 / FR-059, landed as
  designed (PR #225).
- 2026-09-08: Initial proposal; implementation follows on the same
  branch. The twenty-fourth round in the `rusty_remind_me`-motivated
  line; the answer to `SERVER-PAGE-DESIGN.md`'s "a cheaper `page`",
  measured and found wanting at 100K rows.
