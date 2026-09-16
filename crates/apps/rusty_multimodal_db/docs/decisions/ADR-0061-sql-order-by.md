# ADR-0061: Compile SQL `ORDER BY` to the Existing Ordered Page

- Status: **Accepted as designed** (2026-09-14 — the owner picked option
  (a): the scoped proposal, `WHERE`/`JOIN`/`GROUP BY`/aggregate
  combinations refused; (b) a filter-bearing `Page` and (c) decline both
  declined). Proposed and then implemented in the same session.
- Date: 2026-09-14
- Deciders: baileyrd
- Related: `docs/design/SERVER-SQL-ORDER-BY-DESIGN.md` (the full design),
  `ADR-0034`/`docs/design/SERVER-SQL-SELECT-DESIGN.md` (the `SELECT`
  subset this extends — `ORDER BY` named as a Non-goal there), `ADR-0055`/
  `docs/design/SERVER-PAGE-DESIGN.md` (`Request::Page`, protocol 20 —
  its own "Non-goals" names "`ORDER BY` in the SQL subset... a
  client-side round the day it is wanted" as this round's direct
  trigger), `ADR-0059` (`Ordered<S, R, Marker>`, the memory-only sorted
  index `Page` already answers `Memory`/`Relation` through), `ADR-0044`/
  `ADR-0045` (`JOIN`, whose own grammar already excludes `ORDER BY`),
  `docs/FUTURE-GROWTH.md` ("Path to SQLite/DuckDB parity," item 1:
  "Still absent: `ORDER BY` in the SQL text, and any planner or
  optimizer").
- Supersedes/Superseded by: none. Client-side only — no wire, storage,
  or protocol-version change; reuses `Request::Page` (protocol 20)
  exactly as it exists today.

## Context

`docs/PROJECT-STATUS.md`'s reconciliation this pass found every
roadmap/spec/traceability row `Implemented`/`Verified` through
`SERVER-WRITE-BATCH` (`ADR-0060`, `SERVER-001` v0.50.0) — no unit is
currently queued for the outer loop (`WORKFLOW.md`'s "next" step) to
resume. Asked to keep maturing this crate "toward a full-blown DBMS,"
this round picks the cheapest, most concretely pre-scoped item still
open on `docs/FUTURE-GROWTH.md`'s own SQL-parity checklist rather than
inventing new scope: **`ORDER BY` is still absent from the SQL text**,
even though the wire primitive it would need already exists and is
already fast. `ADR-0034` (`SERVER-SQL-SELECT-DESIGN.md`) named it a
Non-goal when the `SELECT` subset was first built, before `Page`
existed. `ADR-0055` (`SERVER-PAGE-DESIGN.md`) then built the wire
primitive — one ordered keyset page over a `U32`/`I64` field, race-safe
under concurrent writes — and named "`ORDER BY` in the SQL subset" as
its own explicit, not-yet-taken revisit trigger. `ADR-0059` made that
primitive fast for `Memory`/`Relation` (a memory-only sorted index,
~155 µs/page regardless of table size). The gap left is exactly: SQL
text has no grammar for it, and `SchemaDrivenClient::query` has no
compile step that reaches `Request::Page` at all.

Unlike every prior round in the `rusty_remind_me`-motivated line, this
gap has no named live consumer request — `docs/FUTURE-GROWTH.md`'s own
"still absent" line and `ADR-0055`'s own revisit trigger are the
motivation, not a hub spike finding. Named plainly, not disguised as
consumer-driven.

## Decision

Propose adding `ORDER BY <field>` to the client-side `SELECT` grammar
(`src/server/sql.rs`) for a plain query only — no `JOIN`, no `GROUP BY`/
aggregate, no `WHERE` — compiled to one or more `Request::Page` calls
instead of `Request::Query`. All three exclusions are refused at parse
time, client-side, no frame sent:

- `ORDER BY` + `WHERE` → `OrderByWithFilter`. `Request::Page` takes no
  filter argument; giving it one is real, separate design work (does a
  filter run before or after `Memory`'s ordered index walk? neither
  `Ordered` nor `page_by_scan` evaluates a predicate today) — not
  bundled here.
- `ORDER BY` + `JOIN` → `OrderByWithJoin`. `JOIN` already lists `ORDER
  BY` as a Non-goal (`SERVER-SQL-JOIN-DESIGN.md`).
- `ORDER BY` + `GROUP BY`/an aggregate column → `OrderByWithAggregate`.
  Grouped/aggregated output is not the raw per-record field `Page`
  walks.

The fork, held for the owner:

- **(a) As scoped above — one field, `U32`/`I64` only, ascending only,
  refuse `WHERE`/`JOIN`/aggregate combinations (recommended).** The
  smallest shape that gives SQL text a real, race-safe order; reuses
  `Page`/`Ordered` completely unchanged; zero wire, server, or storage
  change — a pure client-side grammar-and-compile round, the cheapest
  and lowest-risk pick available on the current roadmap.
- **(b) Also thread a `WHERE` filter through to a new, filter-bearing
  `Page` variant (or an optional field on the existing one).** Closes
  the gap fully. Cost: a protocol bump, and a real evaluation-strategy
  design question this crate has never answered (filter-then-index-walk
  vs. index-walk-then-filter, and what happens when the ordered field
  itself is unindexed for a domain) — a legitimate follow-on, not
  bundled here since no consumer has asked for it yet.
- **(c) Decline.** `ORDER BY` stays unavailable in SQL text; a caller
  wanting an ordered walk uses `SchemaDrivenClient::page`/Python
  `Client.page` directly (both already exist, unchanged either way).
  `docs/FUTURE-GROWTH.md`'s gap stays open.

## Consequences

- Positive (a): closes a doc-named gap with no wire, storage, or
  protocol change — `Page`/`Ordered` already do the real work; this
  round is a client-side grammar + compile step only, the smallest unit
  of real progress toward `docs/FUTURE-GROWTH.md`'s SQL-parity item
  available today.
- Named, not hidden: (a) cannot order a filtered result. A query
  needing both `WHERE` and `ORDER BY` is refused with a clear parse
  error, never silently degraded to an unordered or an unfiltered
  result.
- Named, not hidden: an `ORDER BY` with no `LIMIT` must loop `Page`
  calls internally (a full ordered walk, one round trip per chunk)
  rather than the single round trip every other `SELECT` gets — a real,
  different cost shape, stated in the design doc's own Consequences
  rather than glossed over.
- No live consumer named this gap the way every prior round in this
  line was named one; the trigger is two governing documents'-own
  explicit "still open" language, not a spike finding. Recorded plainly
  so the owner can weigh whether it is worth doing at all right now —
  option (c) is a fully legitimate answer.

## Acceptance and implementation

- 2026-09-14: proposed, design only.
- 2026-09-14: the owner picked option (a). Implemented on the same
  branch: `src/server/sql.rs` (`order_by_clause` grammar,
  `ParsedQuery::order_by`, `OrderByWithFilter`/`OrderByWithJoin`/
  `OrderByWithAggregate`, `validate_order_by`), `src/server/client.rs`
  (`query_ordered`, `fetch_page`, the `query` dispatch branch) —
  `SERVER-001` v0.51.0 / `SERVER-001-FR-061`. No wire, protocol-version,
  or server-side (`src/server/mod.rs`/`serve.rs`) change; `Request::Page`
  reused exactly as it existed. Proven: 5 new `sql.rs` unit tests (the
  grammar plus each of the three exclusions and the reserved keyword),
  6 new `tests/server_sql_integration.rs` integration tests over a real
  socket — a `LIMIT`-bearing round trip matching `SchemaDrivenClient::
  page`'s own result, named-column projection despite `Page` returning
  every field, a 2,500-record no-`LIMIT` walk across several internal
  `Page` calls with no duplicate/missing row across a chunk seam, every
  client-side refusal, and the protocol-20 gate. `cargo fmt --all --
  check` clean; `cargo clippy --all-features -- -D warnings` clean
  (lib/bins/tests/examples — `--all-targets` was not usable in this
  session's Windows environment due to a pre-existing, unrelated,
  documented Linux-only bench target, `benches/scan_ages_crossover.rs`);
  `cargo test --all-features` 521 lib tests (520 passing + 1 pre-existing
  failure, `generic::insert_log::tests::a_version_1_log_reads_as_items_
  and_upgrades_on_append`, a Windows file-locking `PermissionDenied` in
  an unrelated module this round never touched, reproduced identically
  under default features with zero code from this round even compiled —
  named here, not fixed, as out of this round's scope); `cargo test`
  (default features) 186 lib tests (185 + the same pre-existing
  failure) + 2 doctests; `cargo test --features server,research --test
  server_sql_integration` 34/34 (28 prior + 6 new).
