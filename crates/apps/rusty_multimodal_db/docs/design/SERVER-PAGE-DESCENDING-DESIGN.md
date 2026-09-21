# Server Page, Descending: `ORDER BY … DESC` on the Wire (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0089`; the `ADR-0059`/`ADR-0076`–`ADR-0088` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as the
  "descending walks" open question `ADR-0055`/`ADR-0059`/`ADR-0075`
  carried and `docs/FUTURE-GROWTH.md`'s SQL-parity line names. **A
  wire change (protocol 27 → 28)**: unlike every round since
  `ADR-0076`, this PR is not self-merged — the owner's review is the
  gate, per the working agreement on public API changes.
- Date: 2026-09-21
- Related: `ADR-0055`/`docs/design/SERVER-PAGE-DESIGN.md` (`Page`, the
  ascending keyset page and its cursor contract), `ADR-0059` (the
  `Ordered` index `Page` walks — "descending order … is one walk each of
  the same set"), `ADR-0068`/`docs/design/SERVER-FILTERED-PAGE-DESIGN.md`
  (`FilteredPage`, protocol 26 — the precedent for a new request
  variant, its gate, its client and Python surface), `ADR-0061` (`ORDER
  BY` in SQL), `ADR-0043` (the Python reference client and the wire
  vectors), `SERVER-002` (the wire format, now 0.17.0), `ADR-0074`/
  `ADR-0086` (the candidate step and `plan_of`, which the filtered twin
  joins).
- Supersedes/Superseded by: none. Additive: `Request::PageDesc` (36)
  and `Request::FilteredPageDesc` (37), `PROTOCOL_VERSION` 28;
  `PageBy::page_by_desc` (generic, `Ordered`); `ConnectionStore::
  page_desc`/`filtered_page_desc` (defaulted; `Memory`/`Relation`/
  `Reminder` override the first); `page_ids_desc`/`page_rows_desc`/
  `page_by_scan_desc`/`filtered_page_by_candidates_desc`; SQL `ORDER BY
  … [ASC|DESC]`; `SchemaDrivenClient::page_desc`; Python `page_desc`/
  `filtered_page_desc`. No new `Response` variant, no new `ErrorCode`.

## Purpose and scope

Every page this server has answered since `ADR-0055` is ascending: the
"latest 50 memories" — the consumer's most natural listing — has meant
walking the whole table forward and keeping the tail, or a client-side
reverse of everything. The sorted index reads backward as cheaply as
forward (`ADR-0059` said so and did nothing about it). This round puts
the other direction on the wire: two request variants that are
`Page`/`FilteredPage` with the cursor named `before`, walked from the
greatest `(order_by, id)` downward; `ORDER BY x DESC` in SQL compiles
to them; both clients carry them.

Scope, exactly: the two variants and the version (`PGD-FR-001`/`002`);
the generic backward walk (`PGD-FR-003`); the server's evaluation
(`PGD-FR-004`); the gate (`PGD-FR-005`); SQL and the Rust client
(`PGD-FR-006`); the Python client and the wire spec (`PGD-FR-007`);
identity, proven (`PGD-FR-008`).

## Non-goals

- **A descending bounded walk** for the filtered twin (`ADR-0076`/
  `ADR-0077` read backward): `FilteredPageDesc` takes the planner's
  candidate step then the descending key selection — O(candidates), not
  O(page). Named option (b).
- **A `direction` field on `Page`/`FilteredPage`**: a retyped variant is
  a rule-1 violation; new variants are the append-only shape.
- **Multi-field `ORDER BY`**, `NULLS FIRST/LAST` (no null exists), a
  descending `Join`/`GROUP BY` output.

## Context and terminology

Read from `main` after PR #288 (`SERVER-001` v0.73.0, protocol 27) this
pass:

- **`PageDesc { order_by, before, limit }`** and **`FilteredPageDesc {
  order_by, before, limit, filter }`**, appended at indices 36/37 (the
  golden vectors pin the bytes: `Page`/`FilteredPage`'s exact layout
  under the new tags). `before: None` starts at the greatest pair; the
  last row's `(value, id)` is the next call's `before`. Answered
  `Response::Rows`.
- **`page_ids_desc`**: `page_ids` read backward — every key strictly
  less than the cursor, the greatest `limit` of them by one
  `select_nth_unstable` on the complement, sorted descending.
- **`PageBy::page_by_desc`** on `Ordered`: `index.range(..cursor).rev()`
  (or `iter().rev()`), `take(limit)`.
- **`ConnectionStore::page_desc`** default: `page_by_scan_desc`;
  `Memory`/`Relation`/`Reminder` answer their ordered field from
  `page_by_desc` and every other field from the scan, exactly as
  `page` does. **`filtered_page_desc`** default:
  `filtered_page_by_candidates_desc` — `indexed_candidates`, every
  predicate re-checked, `page_rows_desc`; no override.
- **Validation**: `validate_page`/`validate_filtered_page` unchanged,
  applied to `before` as to `after`.
- **`plan_of`**: `FilteredPageDesc` classifies as its candidate step;
  `PageDesc`, like `Page`, is not a planned read.
- **SQL**: `order_by_clause := "ORDER" "BY" ident ["ASC" | "DESC"]`;
  `ParsedQuery::descending`. The parser reserves no keywords, so `ORDER
  BY DESC` alone names a field called `DESC`, as before.
- **The Rust client**: `query_ordered` routes `descending` to the twins
  (`Unsupported("order by desc")` below 28, no frame); `page_desc`
  beside `page`.
- **The Python client**: `PageDesc`/`FilteredPageDesc` at 36/37,
  introduced at 28; `page_desc`/`filtered_page_desc` beside their
  ascending twins; the driver exercises both.

## Requirements

- `PGD-FR-001`/`002` **The variants.** As "Context"; `PROTOCOL_VERSION`
  28; the version table and the golden vectors extended.
- `PGD-FR-003` **The generic walk.** `PageBy::page_by_desc` on
  `Ordered`, exposed on `GenericProductionStore`.
- `PGD-FR-004` **The evaluation.** As "Context".
- `PGD-FR-005` **The gate.** `Malformed` below 28 (rule 3); the audit
  log's `RequestKind` names both.
- `PGD-FR-006` **SQL and the Rust client.** As "Context".
- `PGD-FR-007` **The Python client and `SERVER-002`.** As "Context";
  `SERVER-002` 0.17.0: §5 header, §5.6 rows 36/37, §7 item 25, §8 row
  28 and rule 3's list, §10.
- `PGD-FR-008` **Identity, proven.** Every descending answer is the
  ascending answer reversed — by construction (the same keys, the same
  `(key, id)` order, read the other way; the same validation; the same
  candidates) and proven in-process (`page_ids_desc` against `page_ids`
  reversed, the cursor strictly exclusive, `limit`, zero; the two arms
  through `dispatch` on the scan-backed fixture; the validation
  errors; `plan_of`), on the generic `Ordered` (`page_by_desc` against
  `page_by` reversed, the cursor, the end), in the SQL parser, over a
  real socket via SQL on `Memory` and `Reminder` (the indexed field and
  a scanned one, `DESC` the exact reverse of the ascending ids, `ASC`
  the default, `LIMIT`, `WHERE`, a two-row `page_desc` cursor walk
  disjoint and complete), and from the Python reference client (two
  descending pages greatest-first and disjoint; the filtered twin). The
  golden vectors, the version pin, and `SERVER-002`'s fixture count
  updated; every other pre-existing test unmodified.

## Considered options

- **(a) Two appended variants, `Ordered` walked backward — implemented.**
- **(b) (a) plus a descending bounded walk** for the filtered twin —
  `ADR-0076`/`0077` mirrored; a second round.
- **(c) Decline.** "Latest N" stays a full ascending walk.

The owner's shorthand: **(a)** as implemented; **(b)** (a) plus the
descending bounded walk; **(c)** decline and revert.

## Proposed shape

`src/server/protocol.rs`: the variants, the version, the table row, the
vectors. `src/generic/{query,store,production}.rs`: `page_by_desc`.
`src/server/serve.rs`: the four free functions, the two trait methods,
the two arms, the gate, `plan_of`. `src/server/audit.rs`: two kinds.
`src/server/{memory,relation,reminder}.rs`: `page_desc`.
`src/server/sql.rs`, `src/server/client.rs`: `DESC`. `clients/python/`:
`protocol.py`, `client.py`, `driver.py`. `tests/fixtures/wire-vectors.txt`.
`benches/server.rs`: `reminder-due` `due-latest-50` and
`due-latest-scan-50` rows.

## Data/state and invariants

- `page_ids_desc(keys, None, n) == reverse(page_ids(keys, None, ∞))[..n]`.
- A descending cursor walk visits every record exactly once, greatest
  first; the ascending and descending walks of one table are reverses.

## Errors, failure, recovery, and observability

No new `ErrorCode`. `Malformed` below 28. The filtered twin shows in
`dogserver_query_plans_total` under its candidate step.

## Security, privacy, and compatibility

Two appended variants; every existing vector byte-identical (rule 1);
a client below 28 never sends them (rule 4) and a server answers a
28-client's old requests unchanged. Reads, gated as `Page`.

## Acceptance criteria

1. Unit: `page_ids_desc`, `dispatch`, `plan_of`, the parser, the
   generic walk, the golden vectors, the version pin.
2. Integration: `tests/server_sql_integration.rs` on `Memory`/`Reminder`;
   `tests/server_python_client.rs` through the Python driver.
3. Measured (`RESULTS.md`): `due-latest-50` (the index backward) beside
   `due-latest-scan-50` (the scan's keys) — the same run, no
   pre-change twin exists for a new request.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the Python vectors test; the `server` bench. Independent review owed —
and, as a wire change, the owner's.

## Traceability

- Roadmap: `SERVER-PAGE-DESCENDING`.
- Decision: `ADR-0089`.
- Specification: `SERVER-001` v0.74.0 / `FR-086`; `SERVER-002` 0.17.0.
- Requirements: `PGD-FR-001`–`008`.

## Open questions

- **A descending bounded walk** — option (b).
- **`WHERE id = …`** and **contradiction as `Malformed`** — the other
  wire rounds the owner was offered.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction; the
  first wire change of the line, so the PR waits on the owner's review.
  Read from `main` after PR #288 this pass.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.74.0 /
  `FR-086`, `SERVER-002` 0.17.0, protocol 28. Acceptance criteria 1–2
  are the tests: `serve.rs` +1
  (`descending_pages_are_the_ascending_pages_read_backward`),
  `generic/memory.rs` +1 (`page_by_desc_is_page_by_read_backward`),
  `sql.rs` +1 (`parses_order_by_direction`), `protocol.rs` two new
  golden vectors and the version pin at 28,
  `tests/server_sql_integration.rs` +1
  (`order_by_desc_is_the_exact_reverse_of_the_ascending_answer`),
  `tests/server_python_client.rs` six new assertions through
  `driver.py`; `tests/fixtures/wire-vectors.txt` at 78 vectors (lib
  646, up from 643; SQL 62, up from 61); 937 tests across 39
  targets, 0 failed. Criterion 3, `RESULTS.md`: `due-latest-50`
  73.2 µs against `due-latest-scan-50` 54,154.4. Still no
  independent review — owed; the owner's review is the merge gate.
