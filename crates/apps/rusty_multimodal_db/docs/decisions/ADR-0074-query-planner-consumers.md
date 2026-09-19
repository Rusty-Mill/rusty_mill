# ADR-0074: Query Planner Step Two — The Equality-Index Candidate Step for `Aggregate`, `FilteredPage`, and `Join`

- Status: **Accepted as designed and implemented** (2026-09-19 — the
  owner picked option (a): all three consumers through one shared
  helper; (b) `FilteredPage` only and (c) decline both declined).
  Proposed, accepted, and implemented the same day — `SERVER-001`
  v0.59.0 / `FR-071`, no deviation (see "Acceptance and
  implementation").
- Date: 2026-09-19
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLANNER-CONSUMERS-DESIGN.md` (the
  full design), `ADR-0073`/`docs/design/SERVER-QUERY-PLANNER-DESIGN.md`
  (step one, `Request::Query` only — its own option (b) is this round),
  `ADR-0035` (`Aggregate`), `ADR-0068` (`FilteredPage` — whose design
  named "a future round can override it for a domain that can narrow
  candidates more cheaply"), `ADR-0044`/`ADR-0050` (`Join`, within and
  across tables), `docs/FUTURE-GROWTH.md` (SQL-parity item 1: "`Query`
  only (`Aggregate`/`FilteredPage`/`Join` still scan)").
- Supersedes/Superseded by: none. Server-evaluation only — no wire,
  protocol-version, trait-signature, adapter, client, or storage change.

## Context

`ADR-0073` (merged as PR #243, 2026-09-19) gave `Request::Query` a real
first planner step: an `Eq` on a `describe()`-declared indexed field
reads the index's bucket instead of the table, every predicate is
re-checked, and the result set is identical by construction. Measured:
576 µs vs. 104.5 ms for the same 1,000-row answer over 100K records.
Its Non-goals and its option (b) named the three other consumers that
still start from `scan_all()` and apply the same `predicate_matches`
filter — `Aggregate`, the `filtered_page` default, and `Join`'s left
side — and deferred them "so this round changes one request's
evaluation and proves it, not four."

Read against the code this pass, the deferred step is mechanically the
same in all three places (each already re-applies its filter to every
row it receives, which is the property that makes the index safe to
trust as a *superset*), and the consumer that matters most to the real
user of this crate — `rusty_remind_me`'s paged, filtered listings via
`FilteredPage` — is also the one with **no observable difference at
all**, because `page_rows` sorts the filtered set by `(key, id)` before
returning it. `Aggregate` and `Join` inherit the same two order-level
differences `Query` already has (order within the response; which items
a `limit` keeps), both inside contracts their own design documents
already word as "unspecified order… `LIMIT` truncates that same
unspecified order" (`Aggregate`) and "`scan_all` order then relation
order" (`Join` — a sentence this round rewords to "candidate order").

Codex's sandbox still cannot spawn processes on this machine (its runner
logs on as the sandbox user and never connects its pipe — investigated
2026-09-19, unresolved), so this design, like `ADR-0073`'s, is written
by Claude under the host-takeover convention with independent review
owed.

## Decision

Propose one private helper, `indexed_candidates(store, schema, filter)`
= `query_candidates(store, plan_query(schema, filter), filter)`, and
three call-site substitutions of `scan_all()` for it: the `Aggregate`
dispatch arm, the `ConnectionStore::filtered_page` default body
(signature unchanged, no adapter override), and `evaluate_join`'s left
loop. Each consumer's existing per-row filter re-check stays and does
the correctness work. `Query`'s arm is refactored onto the same helper
with no behavior change.

The fork, held for the owner:

- **(a) All three consumers (recommended).** One helper, three
  substitutions, each proven per consumer and per shipped indexed field
  over a real socket the way step one was. Cost: `Aggregate` and `Join`
  gain the same two order-level differences `Query` has (named, inside
  existing contracts); three evaluation paths change in one round.
- **(b) `FilteredPage` only.** The zero-observable-difference consumer
  and the hub's hottest path. Smallest change; `Aggregate`/`Join` wait.
- **(c) Decline.** The planner stays `Query`-only.

## Consequences

- Positive (a): every filtered read shape the server offers — rows,
  groups, pages, joins — reads *k* records instead of *n* when the
  filter carries an equality on a declared index, with no wire, trait-
  signature, adapter, or client change; `docs/FUTURE-GROWTH.md` can drop
  "`Aggregate`/`FilteredPage`/`Join` still scan."
- Named, not hidden: still no cost model (step one's Non-goal,
  inherited); `Join`'s right side is not narrowed because there is
  nothing to narrow (rows arrive by id from the relation).
- Named, not hidden: an existing test that pins `Aggregate` group order
  or `Join` pair order on an indexed-field filter would be pinning an
  order its own design calls unspecified; it is relaxed to a set
  comparison and the relaxation recorded here, rather than bending the
  planner to preserve accidental `all_ids` order.
- Named, not hidden: `Join` is not measured this round — its left side
  is the fetch `Query` already measured, and its cost is dominated by
  the untouched right-side lookups.

## Acceptance and implementation

- 2026-09-19: proposed, design only.
- 2026-09-19: the owner picked option (a) the same session, recorded in
  the design PR before merge (the `ADR-0061`/`ADR-0073` precedent).
  Implementation not yet started — it follows on its own branch,
  extending `SERVER-001` at its next minor (`FR-038`/`FR-045`/`FR-068`
  on `FR-070`), with `QPC-FR-005`'s per-consumer proofs and acceptance
  criterion 4's measurement as its exit gate. Builder: Claude directly,
  under the host-takeover convention (Codex's sandbox still cannot spawn
  processes); independent Codex inspection owed, not claimed.
- 2026-09-19: implemented on `claude/planner-consumers-impl` —
  `SERVER-001` v0.59.0 / `SERVER-001-FR-071`, `src/server/serve.rs`
  alone, **no deviation** from the accepted design: `indexed_candidates`;
  the `Query` arm refactored onto it; the `Aggregate` arm, the
  `filtered_page` default body, and `evaluate_join`'s left loop
  substituted; `PlannerFixture` gains a fixed `neighbors` edge. Proven:
  3 new `serve.rs` unit tests (lib 607, up from 604), 3 new real-socket
  `tests/server_sql_integration.rs` tests (47, up from 44), 1 new
  cross-table `tests/server_memory_integration.rs` test (14, up from
  13); every pre-existing test passed unmodified — the anticipated
  order-pin relaxation (`QPC-FR-006`) was not needed; `fmt`/`clippy
  --features server,research --all-targets -D warnings` clean.
  Measured (`benches/server.rs`'s `memory-planner` rows extended to
  `count(*)` and `fpage-50`, `RESULTS.md`): on the same 100K `Memory`
  table and 1%-selective equality as step one, `COUNT(*)` 419.9 µs
  through the `category` index vs. 99,583.2 µs through the unindexed
  `source` control (~237×), `FilteredPage` `LIMIT 50` 535.7 µs vs.
  104,109.9 µs (~194×), and `Query` re-measured in the same run 546.6
  µs vs. 100,524.0 µs (~184×), over a real loopback socket. One harness
  fix: the raw bench connection now sends `Hello` first — `FilteredPage`
  is gated at protocol 26 while `Query`/`Aggregate` have no server-side
  gate. `SERVER-SQL-JOIN-DESIGN.md`'s one
  sentence reworded; `SERVER-FILTERED-PAGE-DESIGN.md`'s Consequences
  qualified. Still no independent review — Codex's sandbox cannot
  spawn processes; a fresh Codex inspection of the design and this diff
  is owed and must not be reported as done.
