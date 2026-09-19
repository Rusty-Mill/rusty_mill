# ADR-0073: Query Planner Step One — Route `Request::Query` Through an Equality Index

- Status: **Accepted as designed and implemented** (2026-09-18 — the
  owner picked option (a): `Request::Query` only, as scoped; (b) the
  four-consumer generalization and (c) decline both declined). Proposed,
  accepted, and implemented in the same session — `SERVER-001` v0.58.0
  / `FR-070`, one recorded deviation (see "Acceptance and
  implementation").
- Date: 2026-09-18
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLANNER-DESIGN.md` (the full
  design), `ADR-0034`/`docs/design/SERVER-SQL-SELECT-DESIGN.md`
  (`Request::Query`, whose own Non-goals and Open questions name this
  exact step as "the first real step toward the query planner"),
  `ADR-0068`/`docs/design/SERVER-FILTERED-PAGE-DESIGN.md` (the most
  recent "shared default vs. index" evaluation-strategy call, and its
  trait-method-with-default shape this round considered and did not
  need), `ADR-0059` (`Ordered`, the range index this round deliberately
  does not reach), `ADR-0011` (`DomainSchema`/`FieldCapabilities::
  filter_eq`, the planner's only input), `ADR-0040` (`Entity`'s
  normalized `label` lookup, the one shipped index that is a superset
  of exact equality), `docs/FUTURE-GROWTH.md` ("Path to SQLite/DuckDB
  parity", item 1: "Still absent: any query planner or optimizer").
- Supersedes/Superseded by: none. Server-evaluation only — no wire,
  protocol-version, trait, adapter, client, or storage change.

## Context

`Request::Query` (protocol 8, `ADR-0034`) has had one evaluation
strategy since it was built: `scan_all()` every record, then filter,
project, and truncate centrally. `SERVER-SQL-SELECT-DESIGN.md` said so
plainly in its own Non-goals — "even a `WHERE <indexed-field> = ...`
predicate `Request::FilterEq` could answer via its existing index pays
the same O(n) cost as every other `Query`" — and named "whether `Query`
should ever consult an existing index... the first real step toward the
query planner" as an open question. Four SQL rounds later
(`ADR-0035`, `ADR-0044`, `ADR-0061`, `ADR-0068`) that question is still
open and `docs/FUTURE-GROWTH.md` still reads "Still absent: any query
planner or optimizer."

Asked to "tackle SQL" after the `ADR-0072` MVCC rollout closed, the
owner chose this slice over three alternatives (an `OR`/parenthesized
`WHERE` expression tree; a server-side `Request::Sql { text }` for
non-Rust clients; `ORDER BY` combined with `JOIN`/`GROUP BY`) as the
smallest step every prior SQL document had already pre-scoped. Read
against the code this pass, the step is smaller than the SELECT
design's own wording suggested: its second cheap path, "`WHERE id = …`
via `GetById`," is not expressible — `Predicate` addresses a field tag
and no adapter exposes the record id as a field — so the equality index
is the whole of step one.

Two facts read from the code make the step safe rather than merely
fast. First, `scan_all` is not one snapshot: every adapter's
implementation is `all_ids()` followed by a per-id `get()`, each its
own lock acquisition, so a `filter_eq()`-then-per-id-`get()` path has
the identical consistency class — no new anomaly. Second, every
adapter's `describe()` `filter_eq: true` flag has a real `filter_eq`
arm behind it and the generic `HashMap` index is maintained on runtime
insert/replace/delete; the one shipped index that is not exact
(`Entity::label`, case-/whitespace-insensitive with aliases) is a
*superset* of exact equality, which a re-check over the fetched rows
corrects and a subset would not.

Codex's sandbox could not spawn processes during this session
(re-confirmed: `exec_command … timed out connecting runner pipe-in`),
so this design was written directly by Claude under this crate's
host-takeover convention. It has not had independent review; a fresh
Codex session should review it once its sandbox is restored, and the
implementation round's inspection must not cite this session as having
covered it.

## Decision

Propose one new step in `dispatch`'s `Request::Query` arm, between the
unchanged `validate_query` and the unchanged `evaluate_query`: choose
a plan from the schema alone (`IndexEq(i)` for the first `Eq` predicate
on a `filter_eq: true` field, else `FullScan`), fetch candidates
accordingly (`filter_eq` + per-id `get`, or `scan_all`), fall back to
`scan_all` on any `filter_eq` error, and re-check **every** predicate
over the fetched rows so the index only narrows what is read, never
what is returned. Zero trait, adapter, wire, or client change.

The fork, held for the owner:

- **(a) `Request::Query` only, as scoped above (recommended).** The
  smallest shape that makes the planner real: one private function
  beside `evaluate_query`, one request's evaluation changed, its
  result-set equality proven per shipped indexed field over a real
  socket before and after runtime writes. Two observable differences,
  named rather than hidden: row order and the `LIMIT`-selected subset
  may differ from before, both within the existing "unspecified order"
  contract.
- **(b) The same step for every `scan_all`-then-filter consumer —
  `Query`, `Aggregate`, the default `filtered_page`, and `Join`'s left
  side — in one round.** More win per line, but four evaluation paths
  change together, `filtered_page` is an overridable trait method, and
  each consumer needs its own per-domain cross-check. A legitimate
  follow-on once (a) has proven the step on the simplest consumer.
- **(c) Decline.** `Query` stays an unconditional full scan; the
  `FUTURE-GROWTH.md` line stays as written. A caller can already
  hand-roll the plan client-side (`FilterEq`, then `GetById` per id) at
  the cost of round trips.

## Consequences

- Positive (a): a `WHERE <indexed-field> = …` `Query` reads *k*
  records instead of *n* on every domain with a declared equality
  index (`Memory::category`, `Entity::kind`/`label`,
  `Relation::subject`, `Order::status`, `Employee::department`,
  `Reminder::due_at_unix_ms`), with no wire, protocol, trait, adapter,
  or client change and an identical result set. `docs/FUTURE-GROWTH.md`
  can stop saying "any" planner is absent.
- Named, not hidden: no cost model. A `Query` whose indexed equality
  value nearly every record shares reads the same *n* records plus one
  hash lookup — never slower in reads, but not a win either. Choosing
  *whether* to use an index is the cost-based optimizer
  `FUTURE-GROWTH.md` names, deliberately not started here.
- Named, not hidden: two order-level behavior differences a client
  relying on accidental `all_ids` order would notice (`QPL-FR-006`),
  recorded in `SERVER-001`'s change history at implementation time.
- Named, not hidden: the "`WHERE id = …`" path the SELECT design
  mentioned is a grammar/schema question, not a planner one, and stays
  open; so does the `Ordered` range index as step two.
- No live consumer named this gap; the trigger is four design
  documents' own repeated "first step toward the planner" language and
  the owner's pick. Option (c) is a fully legitimate answer.

## Acceptance and implementation

- 2026-09-18: proposed, design only.
- 2026-09-18: the owner picked option (a) the same session. Recorded in
  the design PR before merge (the `ADR-0061` precedent). Implementation
  not yet started — it follows on its own branch, extending `SERVER-001`
  (`FR-037`'s `Request::Query`) at its next minor, with `QPL-FR-005`'s
  per-domain result-set-equality tests and acceptance criterion 5's
  measurement as its exit gate. Builder: Claude directly, under this
  crate's host-takeover convention (Codex's sandbox still cannot spawn
  processes); independent Codex inspection of both this design and the
  implementation is owed once restored, and neither may cite this
  session as having provided it.
- 2026-09-18: implemented on `claude/sql-planner-impl` — `SERVER-001`
  v0.58.0 / `SERVER-001-FR-070`. `src/server/serve.rs` only:
  `QueryPlan`/`plan_query`/`query_candidates` beside `evaluate_query`,
  the `Request::Query` dispatch arm as validate → plan → candidates →
  evaluate; `QPL-FR-007` held literally (no trait, adapter, wire,
  protocol, `sql.rs`, client, or Python change). **One recorded
  deviation**: `QPL-FR-004`'s `debug_assert!` on the fallback branch is
  not in the code — the fixture test that proves the fallback would trip
  it; the shipped adapters' `describe()`/`filter_eq` agreement is proven
  by `QPL-FR-005`'s per-domain integration tests instead. Proven: 5 new
  `serve.rs` unit tests (lib 604, up from 599 — a `PlannerFixture` with
  an exact, superset, or refusing index and `get`/`scan_all` counters),
  4 new `tests/server_sql_integration.rs` tests over real sockets (44,
  up from 40 — all seven shipped indexed fields plus the `Dog` control
  against the full scan; `Entity::label` variants matched by `FilterEq`
  but not `Query`; the two-predicate conjunction in both orders; runtime
  `Insert`/`Replace`/`Delete`), every pre-existing test unmodified and
  green; `cargo fmt`/`clippy --features server,research --all-targets
  -D warnings` clean. Measured (`benches/server.rs`'s `memory-planner`
  rows, `RESULTS.md`): at 100K `Memory` records the same 1%-selective
  equality (1,000 rows returned either way) is 576.5 µs per query
  through the declared `category` index vs. 104,492.7 µs through the
  equal-selectivity unindexed `source` control — ~181×, over a real
  loopback socket. Still no independent review — Codex could
  not spawn processes throughout this session; a fresh Codex inspection
  of the design and this diff is owed and must not be reported as done.
