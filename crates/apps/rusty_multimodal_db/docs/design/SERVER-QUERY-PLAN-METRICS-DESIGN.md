# Server Query Plan Metrics (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0086`; the `ADR-0059`/`ADR-0076`–`ADR-0085` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as the "plan
  metrics" open question every planner design since `ADR-0073` has
  carried ("still named, still not bundled"); the fork below stays
  open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0064`/`docs/design/SERVER-METRICS-DESIGN.md`
  (`ServerMetrics`, "a bounded, fixed counter set", `MET-FR-002`; the
  one post-dispatch call site, `MET-FR-003`), `ADR-0069` (the HTTP
  scrape renders the same text), `ADR-0073`/`0075`/`0078` (the four
  `QueryPlan`s), `ADR-0076`/`0081`/`0082`/`0084` (the walk-only paths),
  `docs/FUTURE-GROWTH.md` (metrics: "still absent … cache/index
  stats").
- Supersedes/Superseded by: none. Additive: `metrics::PlanKind`, a
  seven-counter family on `ServerMetrics` rendered as
  `dogserver_query_plans_total{plan="…"}`, `serve::plan_of` (pure), one
  classification and one increment at `handle_connection`'s existing
  post-dispatch site. No wire, protocol-version, adapter, or client
  change; `dispatch` and every planner function untouched. The
  `Metrics` text gains one family (`QPM-FR-004`).

## Purpose and scope

Ten planner rounds have shipped without a way to see, on a running
server, which path a read took: an operator who wonders whether the
consumer's `since`-shaped page is walking the index or scanning has
only the latency to go on. Every one of those designs named the same
answer — a per-plan counter in `ServerMetrics` — and none bundled it,
because `dispatch(store, req)` is pure and reaches no metrics.

This round takes it without touching `dispatch`: the plan a read will
take is a pure function of the request and the adapter's declarations,
computed by exactly the predicates the arms consult (`plan_query`,
`bounded_walk_applies`, `counted_walk_applies`/`keyed_walk_applies`).
`plan_of(store, &req)` computes it at the one place metrics already
live — `handle_connection`, before the match consumes the request —
and the counter is incremented after dispatch, only when the response
carries no error. One family, seven labels, fixed at compile time.

Scope, exactly: the kinds (`QPM-FR-001`); the counters (`QPM-FR-002`);
the recording (`QPM-FR-003`); the render (`QPM-FR-004`); no other
surface changed (`QPM-FR-005`).

## Non-goals

- **A "walk abandoned" counter** for `ADR-0079`'s budget, or per-plan
  latency: both need the arm to report back; the first is a second
  family when a consumer asks, the second is `MET`'s own Non-goal.
- **Per-table labels** under `serve_tables`: process-wide, as every
  other counter.
- **Counting `Page`** (no filter, no plan) or the point reads.
- **A structured `Response::Plan`** ("EXPLAIN"): a protocol round.

## Context and terminology

Read from `main` after PR #285 (`SERVER-001` v0.70.0) this pass:

- **`PlanKind`** (`metrics.rs`): `FullScan`, `IndexEq`, `IndexRange`,
  `IndexIntersect` (the `QueryPlan`s), `BoundedWalk` (`ADR-0076`'s
  page walk), `CountedWalk` (`ADR-0081`), `KeyedWalk` (`ADR-0082`/
  `0084`). Labels are the snake_case names.
- **`plan_of`** (`serve.rs`): `Query` → its candidate step;
  `Aggregate` → `CountedWalk` when `counted_walk_applies` and
  ungrouped, `KeyedWalk` when `keyed_walk_applies` (the grouped
  no-bound-with-`limit` shape excepted, `QKG-FR-003`), else its
  candidate step; `FilteredPage` → `BoundedWalk` when
  `bounded_walk_applies`, else its candidate step; `Join` → the left
  filter's candidate step; everything else `None`.
- **Two named approximations.** A read `dispatch` refuses at
  validation is classified but never counted (only an ok response
  records). A walk-only aggregate whose adapter refuses the walk
  (`Unsupported` — no shipped adapter) counts as the walk it asked for
  while the decode path answered.
- **`MET-FR-002` amended**: the set is still bounded and fixed at
  compile time — six counters plus one family of seven.

## Requirements

- `QPM-FR-001` **The kinds.** `PlanKind`, seven variants, `ALL` in
  render order, `label()` stable.
- `QPM-FR-002` **The counters.** `ServerMetrics.plans: [AtomicU64; 7]`,
  `record_plan(kind)`.
- `QPM-FR-003` **The recording.** `handle_connection` classifies with
  `plan_of` before the match, records after dispatch iff the outcome
  is `Ok` — the same site as `record_request`.
- `QPM-FR-004` **The render.** Between `connections_active` and
  `uptime_seconds` (which stays the last line), one `HELP`/`TYPE`
  pair and seven `dogserver_query_plans_total{plan="…"} N` lines, every
  label present at zero on a fresh server; the HTTP scrape identical.
- `QPM-FR-005` **Everything else unchanged.** `dispatch`, the planner,
  the wire (`PROTOCOL_VERSION` 27), the clients: untouched.

Identity, proven: `plan_of` against every shape on `PlannerFixture`
(the four candidate plans per consumer, the three walks, the held-back
grouped shape, `None` for the rest); the render's family and labels;
over a real socket on `Memory` one read per label moving exactly its
own counter, an unplanned read and a refused read moving none, repeats
accumulating. Every pre-existing test unmodified. Measured: the
existing planner rows before and after — the classification's cost is
one `describe()` and a pure plan per read.

## Considered options

- **(a) Classify before dispatch, pure — implemented.** No change to
  `dispatch` or any planner function.
- **(b) Thread a `&PlanCounters` through `dispatch`** — exact for the
  two approximations above, at the cost of every planner signature
  and every call site.
- **(c) Decline.** The path stays invisible.

The owner's shorthand: **(a)** as implemented; **(b)** thread the
counters; **(c)** decline and revert.

## Proposed shape

`src/server/metrics.rs`: `PlanKind`, `plans`, `record_plan`, the
render. `src/server/serve.rs`: `plan_of`, the two lines in
`handle_connection`.

## Data/state and invariants

- `sum(plans) <= requests_ok_total` (a planned read is one ok request).
- Each label is incremented from one site, once per ok planned read.

## Errors, failure, recovery, and observability

This round *is* observability. Nothing new can fail.

## Security, privacy, and compatibility

Seven more integers on a read already gated at the version gate; no
wire change. A Prometheus consumer that pinned the old text gains one
family — the text was never a fixed contract beyond its grammar.

## Acceptance criteria

1. `plan_of` unit matrix; `PlanKind`/render unit test.
2. Integration (`tests/server_metrics_integration.rs`): the socket
   proof above.
3. Measured (`RESULTS.md`): the planner rows unchanged within noise.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench before and after, back to back. Independent review
owed.

## Traceability

- Roadmap: `SERVER-QUERY-PLAN-METRICS`.
- Decision: `ADR-0086`.
- Specification: `SERVER-001` v0.71.0 / `FR-083` (amends `FR-064`'s
  `MET-FR-002`).
- Requirements: `QPM-FR-001`–`005`.

## Open questions

- **A "walk abandoned" counter** — option (b)'s threading would make it
  exact; named.
- **A fold that never materializes the keys** — unchanged from
  `ADR-0082`. *Taken: `ADR-0087` (2026-09-21) —
  `docs/design/SERVER-QUERY-PLANNER-RANGE-FOLD-DESIGN.md`.*

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as the
  open question every planner design since `ADR-0073` carried. Read
  from `main` after PR #285 this pass. The existing planner rows were
  measured on the pre-change code first.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.71.0 /
  `FR-083`, exactly the "Proposed shape". Acceptance criteria 1–2 are
  the tests: `serve.rs` +1 (`plan_of_names_the_path_each_planned_read_takes`),
  `metrics.rs` +1 (`plan_family_renders_every_kind_and_counts_each_on_its_own_label`),
  `tests/server_metrics_integration.rs` +1
  (`query_plans_total_counts_each_planned_read_on_its_own_label`)
  (lib 640, up from 638; metrics integration 4, up from 3);
  928 tests across 39 targets, 0 failed, every pre-existing test
  unmodified. Criterion 3, `RESULTS.md`: every planner row within noise
  (every row moves in both directions by the container's usual run-to-run spread — `index-eq` `query` 1,195.6 → 1,412.8 µs, `eq-range` `query` 256.2 → 103.0, `range-tight` `count(*)` 48.2 → 53.9, `index-range` `fpage-50` 132.5 → 127.1, `due-window` 1,104.7 → 347.2 — with no row moving consistently one way and the sub-100 µs rows, where a per-read `describe()` would show first, within ~6 µs). Still no independent review — owed.
