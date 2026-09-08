# ADR-0055: One ordered keyset page — `Request::Page` over an orderable field, at protocol 20

- Status: **Accepted as designed** (promoted from Proposed on
  2026-09-07 — the owner's "accept as designed", option (a): one ordered
  keyset page over an orderable field, evaluated by default over the
  full scan, overridable; (b) `ORDER BY`/`LIMIT`/`OFFSET` in `Query`,
  (c) a second scannable slot first, (d) a server-side sequence, and
  (e) decline declined. Recorded in "Acceptance and implementation"
  below.) Proposed and implemented on one branch, the
  `ADR-0046`–`ADR-0054` cadence.
- Date: 2026-09-07
- Deciders: baileyrd
- Related: `docs/design/SERVER-PAGE-DESIGN.md` (the full design),
  `ADR-0034` (`Query`/`Rows`, the scan posture and kind rules reused),
  `ADR-0035` (`Aggregate` with a filter, the count-since answer),
  `docs/reports/2026-09-07-hub-spike-report.md` (gap 2).
- Supersedes/Superseded by: none. Appends `Request::Page` (29) at
  protocol 20; no new response, no new code, no format change.

## Context

Nothing on this wire has an order, so a sync puller fetches the whole
table and sorts it client-side for every page — the hub spike's second
gap, the one that grows with the table.

## Decision

Add `Request::Page { order_by, after, limit }`: every record sorted
ascending by an orderable (`U32`/`I64`) field with the id as the
tie-break, strictly after an optional `(value, id)` cursor, at most
`limit` rows, answered by the existing `Response::Rows`. Validated
before any scan; a read gated as `Query`. The order is written once
(`page_rows`); `ConnectionStore::page` defaults to it over `scan_all`
so any adapter may answer more cheaply later. No index, no descending
order, no `Str` order, no `Since` request, no `ORDER BY` in SQL, no
server-side sequence — each named.

## Consequences

- Positive: a puller walks a table in pages that are disjoint and
  complete under concurrent writes; "everything after *t*" is a cursor.
- Positive: additive; no adapter changed; a pre-20 client unaffected.
- Named, not hidden: the default scans every record per page —
  `Query`'s cost, acceptable at target scale, overridable. (Since
  v0.48.1 it materializes only the page: keys first, then the winners.)
- Named, not hidden: ascending and numeric only.

## Considered options

**(a) Accept as designed** — one ordered keyset page over the full scan,
overridable. **(b) `ORDER BY`/`LIMIT`/`OFFSET` in `Query`** — offsets
repeat and skip under writes. **(c) A second scannable slot first** — an
optimization before the request exists. **(d) A server-side sequence**
— a format change for an order `updated_at` gives. **(e) Decline.**

## Acceptance and implementation

- 2026-09-07: proposed and implemented on the same branch as
  `SERVER-001` v0.45.0 / FR-055, `SERVER-002` v0.9.0 —
  `src/server/{protocol,serve,audit,client}.rs`, `clients/python/**`,
  one vector (62 pinned); tests: `serve` +1, `server_memory_integration`
  +1, `server_dog_integration` +1, `server_protocol_version` +1,
  `server_python_client` extended; every acceptance criterion 1–3 holds.
  (PR #214.)
- 2026-09-07: accepted as designed (option (a); (b)–(e) declined). No
  change to the implementation. Next in the line: `deleted_at`/`node_id`
  in the `Memory` projection.
- 2026-09-08: `SERVER-001` v0.48.1 — measured for the first time (100K
  `Memory` records: 292 ms per page of 50), the default was changed to
  select the page by key and materialize only its rows (100 ms); the
  decision, the order, and the wire are unchanged. The remaining cost is
  the per-record decode; an index is the open question, now with a
  threshold (`RESULTS.md`).
