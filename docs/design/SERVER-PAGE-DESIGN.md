# Server Ordered Page Design (Accepted)

- Status: **Accepted** (2026-09-07, `ADR-0055` option (a), as designed —
  one ordered keyset page over an orderable field, evaluated by default
  over the full scan, overridable; `ORDER BY`/`OFFSET` in `Query`, a
  second scannable slot first, a server-side sequence, and declining
  all declined). Implemented on the same branch as `SERVER-001` v0.45.0
  / FR-055, `SERVER-002` v0.9.0, PR #214.
- Date: 2026-09-07
- Related: `ADR-0034`/`docs/design/SERVER-SQL-SELECT-DESIGN.md` (`Query`,
  `Response::Rows`, the full-scan posture and `SQL-FR-007`'s kind rules,
  all reused), `ADR-0035` (`Aggregate` with a filter — already the
  server-side answer to "count since *t*"), `ADR-0048` (`Memory`, whose
  `updated_at_unix_ms` is the field a sync puller orders by),
  `docs/reports/2026-09-07-hub-spike-report.md` (gap 2, the round's
  evidence).
- Supersedes/Superseded by: none. Appends one `Request` variant at
  protocol 20; no new response, no new error code, no file format
  change.

## Purpose and scope

Nothing on this wire has an order. `Query` truncates in whatever order
the scan produced; `ScanField` returns every value. A sync puller that
wants "the next 500 records updated after *t*, oldest first" therefore
fetches the whole table and sorts it client-side — the hub spike's
second gap, and the one that grows with the table. This round gives the
wire one ordered keyset page: every record sorted by an orderable field
with the id as the tie-break, strictly after a `(value, id)` cursor, at
most `limit` rows.

Scope, exactly: `page_ids` (v0.48.1; `page_rows` before), the one place the order is written
(`PAG-FR-002`); `ConnectionStore::page` defaulting to it over `scan_all`
(`PAG-FR-002`); validation before any scan (`PAG-FR-003`);
`Request::Page` at protocol 20 answered by the existing `Response::Rows`,
gated as `Query` (`PAG-FR-004`); both clients (`PAG-FR-005`); the pins
and `SERVER-002` v0.9.0 (`PAG-FR-006`).

## Non-goals

- **An index for the order.** The default evaluates over `scan_all` —
  every record materialized, sorted, the first `limit` kept — exactly
  `Query`'s posture (`SQL-FR-004`), milliseconds at this crate's target
  scale. `ConnectionStore::page` is a trait method with a default so an
  adapter can answer more cheaply later (a second scannable slot via
  `MmapScanned`, as `Employee` already has, would give the values
  without the records); the revisit trigger is a measured page time,
  not a guess.
- **Descending order.** Ascending only; a caller wanting the newest
  first reads the tail by a large cursor and reverses. Named so a later
  `descending: bool` is one appended field if ever needed.
- **Ordering by a `Str` field.** `U32`/`I64` only — the one kind pair
  with an order this wire already recognizes (`CompareOp::is_ordering`).
  String collation is a design of its own.
- **A `Since` request.** "Everything after *t*" is a cursor at
  `(t, RecordId::max())`; no second shape.
- **A count.** `Aggregate` with a filter already answers `COUNT(*)
  WHERE updated_at >= t` server-side (`ADR-0035`).
- **Projection.** Every field of each row comes back; a puller reads
  whole records, and `Query` exists for anything narrower.
- **`ORDER BY` in the SQL subset.** The wire primitive first; SQL
  surface is a client-side round the day it is wanted.

## Context and terminology

- **Keyset page**: a page addressed by the key of the last row seen —
  `(order_by value, id)` — not by an offset, so a concurrent insert or
  delete never shifts or repeats rows.
- **Cursor**: `after: Option<(ScanValue, RecordId)>`; `None` is the
  first page; rows with a key strictly greater are served.
- **Key**: the field's value as an `i128` (so `U32` and `I64` share one
  order) and the id; ascending.

## Requirements

- `PAG-FR-001` **Shape.** `Request::Page { order_by: FieldRef, after:
  Option<(ScanValue, RecordId)>, limit: u64 }` (29), answered
  `Response::Rows` with every field of each record in tag order.
- `PAG-FR-002` **The order.** `page_ids(keys, after, limit)`: key each
  record by `(value as i128, id)`, keep keys strictly greater than the
  cursor's, take the `limit` smallest in ascending order;
  `page_rows(rows, …)` applies it to rows already held.
  `ConnectionStore::page` defaults (v0.48.1) to `page_keys(order_by)` —
  `(id, key)` per record, from `scan_all` unless the adapter reads the
  field off its record — then `page_ids`, then `get` for the page's
  ids only; `PageRow` names the row type.
- `PAG-FR-003` **Validation**, in `dispatch` before any scan:
  `UnknownField` for an `order_by` the schema lacks; `Malformed` for a
  field that is not `U32`/`I64`, a cursor value of another kind, or a
  zero `limit`.
- `PAG-FR-004` **The request.** `PROTOCOL_VERSION` 20, table row 20;
  a read — authentication only, never overlaid by a session, never
  read-set-tracked, `Query`'s posture; server-gated `Malformed` below
  20 (rule 3, the `Join` precedent); `audit::RequestKind::Page`.
- `PAG-FR-005` **Clients.** `SchemaDrivenClient::page(order_by: &str,
  after, limit) -> Result<Vec<QueryRow>, _>` — an unknown field
  `UnknownField`, a non-orderable one `Unsupported("page order")`, a
  zero limit `Unsupported("page limit")`, all local; `Unsupported
  ("page")` below 20. Python `Client.page(order_by, after=None,
  limit=100)` returning `(id, [(name, value), …])` rows,
  `PROTOCOL_VERSION = 20`.
- `PAG-FR-006` **Pins.** One golden vector (`Request/Page`; 62 pinned);
  `SERVER-002` v0.9.0; the literal `Hello` pins moved to 20.

## Considered options

- **(a) One ordered keyset page over an orderable field, evaluated by
  default over the full scan, answered by `Rows` — proposed.** The
  smallest shape that gives a puller real pages; reuses `Rows`, the
  kind rules, and the scan posture; the default is overridable.
- **(b) `ORDER BY`/`LIMIT`/`OFFSET` in `Query`.** Offsets repeat and
  skip rows under concurrent writes, which is the puller's exact
  hazard; and it widens the SQL subset before the wire has the
  primitive.
- **(c) A second scannable slot for `updated_at` on `Memory` first.**
  An optimization of a request that does not exist yet; the trait
  default leaves room for it.
- **(d) A server-side monotonic sequence (`hub_seq`).** A new field on
  every record and a format change, for an order `updated_at` already
  gives.
- **(e) Decline** — pulls stay full scans.

## Proposed shape

`src/server/protocol.rs` (the variant, the constant, the table row, one
vector), `src/server/serve.rs` (`PageRow`, `page_rows`, `page_ids`, `page_keys`, `validate_page`,
the trait method with its default, one `dispatch` arm, one gate),
`src/server/audit.rs`, `src/server/client.rs`, `clients/python/**`. No
adapter change: every domain is served by the default.

Wire: `Page` = `[0x1d 0 0 0]` · `order_by` (`u16`) · `after` (`0x00`,
or `0x01` · `ScanValue` · `RecordId`) · `limit` (`u64`).

## Data/state and invariants

- Two consecutive pages, the second cursored by the first's last key,
  are disjoint and together cover every record whose key is greater
  than the first cursor — regardless of writes between them to records
  outside that range.
- A record's position is a function of its key alone; equal values are
  ordered by id.
- The empty page ends a walk.

## Errors, failure, recovery, and observability

Validation refusals before any scan; `Unsupported` from an adapter that
overrides `page` and declines. A read: nothing to recover. The access
log carries the request kind.

## Security, privacy, and compatibility

A read, gated as `Query`. No format change; a pre-20 client is
unaffected.

## Acceptance criteria

1. `page_rows`: rows sort by `(value, id)`; the cursor is strict; the
   tie is broken by id; `limit` truncates after the sort; a cursor at
   the maximum id is "everything after *t*".
2. Wire, `Memory`: four samples plus a fifth tying with the fourth walk
   in three pages of two, the last key of each the next cursor; the tie
   is broken by id; the empty page ends the walk; a cursor at
   `(3000, max)` serves the two later ones; every row carries eleven
   fields. `Dog` by `age` through the default: two pages disjoint and
   complete, in `(age, id)` order. The client refuses a `Str` order, a
   zero limit, and an unknown field locally; the server answers
   `Malformed` for a wrong-kind cursor, a `Str` order, and a zero
   limit, `UnknownField` for an unknown tag. `Malformed` at 19 and
   silent, served at 20, `Unsupported("page")` at 1. From Python, two
   pages walk every entity sorted and disjoint.
3. One vector added, every earlier vector byte-identical; `SERVER-002`
   v0.9.0; the Python suite passes offline and live.

## Verification plan

`cargo test --all-features` / default / `client`; clippy `-D warnings`
on the same three; `cargo fmt --check`; the Python vector suite; `cargo
doc --all-features --no-deps` at the baseline.

## Traceability

- Roadmap: `SERVER-PAGE-DESIGN`, `SERVER-PAGE`. Decision: `ADR-0055`.
- Specification: `SERVER-001` v0.45.0 / `FR-055`; `SERVER-002` v0.9.0.
- Requirements: `PAG-FR-001`–`006`.

## Open questions

- **A cheaper `page` for `Memory`** — measured (v0.48.1, `RESULTS.md`):
  after selecting by key, one page of 50 over 100K records costs
  100 ms, all but ~20 ms of it one decode per record; SQLite's indexed
  `ORDER BY … LIMIT` is 14 µs. A second scannable slot for
  `updated_at_unix_ms` (`MmapScanned`, `Employee`'s precedent) removes
  the decode; a sorted index removes the scan. Found wanting at about
  100K rows; the owner's call which, and when.
- **Descending order**, **`ORDER BY` in the SQL subset** — one appended
  field and one client-side round, if wanted.
- **`deleted_at`/`node_id` in the projection**, **directed open-label
  edges**, **a global edge count** — the spike's gaps 3–5, unchanged.
- **`ADR-0053`'s and `ADR-0054`'s open questions** — unchanged.

## Change history

- 2026-09-08: `SERVER-001` v0.48.1 — the default `page` selects by key
  before materializing (`page_keys`/`page_ids`; `Memory`/`Entity`/
  `Relation` read the key off the record). 292 → 100 ms per page of
  50 at 100K records; the order and every test unchanged. `PAG-FR-002`
  reworded to say so; no requirement changed.
- 2026-09-07: Accepted as designed (option (a)), after PR #214. No
  content change.
- 2026-09-07: Implemented as `SERVER-001` v0.45.0 / FR-055, `SERVER-002`
  v0.9.0, landed as designed (PR #214).
- 2026-09-07: Initial proposal; implementation follows on the same
  branch. The twentieth round in the `rusty_remind_me`-motivated line;
  the hub spike's second gap.
