# ADR-0086: Query Plan Metrics

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0085` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  the "plan metrics" open question every planner design since
  `ADR-0073` carried; the fork below is held open for the owner at
  review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLAN-METRICS-DESIGN.md` (the full
  design), `ADR-0064` (`ServerMetrics`, the bounded fixed set, the one
  post-dispatch call site), `ADR-0069` (the HTTP scrape), `ADR-0073`–
  `ADR-0085` (the plans and walks it names), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `metrics::PlanKind`, one
  seven-counter family on `ServerMetrics` rendered as
  `dogserver_query_plans_total{plan="…"}`, `serve::plan_of` (pure), one
  classification and one increment in `handle_connection`. No wire,
  protocol, adapter, or client change; `dispatch` untouched.

## Context

Every planner design since `ADR-0073` ended its open questions with
"plan metrics — still named, still not bundled": `dispatch(store, req)`
is pure and reaches no metrics, so counting the path taken seemed to
need every planner signature threaded. It does not. The path is a pure
function of the request and the adapter's declarations, decided by
predicates that are already free functions — `plan_query`,
`bounded_walk_applies`, `counted_walk_applies`, `keyed_walk_applies` —
so it can be classified where metrics already live, before dispatch,
and recorded after, when the response is ok.

## Decision

Implement: `PlanKind` (`FullScan`, `IndexEq`, `IndexRange`,
`IndexIntersect`, `BoundedWalk`, `CountedWalk`, `KeyedWalk`) with stable
snake_case labels; `ServerMetrics.plans` and `record_plan`; the render
gains one `HELP`/`TYPE` pair and seven lines before `uptime_seconds`
(still the last line),
every label present at zero; `plan_of(store, &req)` classifies `Query`,
`Aggregate`, `FilteredPage`, and `Join` (the rest `None`) by the arms'
own predicates; `handle_connection` classifies before the match and
records iff the outcome is `Ok`. Two approximations named: a read
refused at validation is never counted; a walk-only aggregate whose
adapter refuses the walk (no shipped adapter) counts as the walk it
asked for. `MET-FR-002`'s "bounded, fixed set" is amended by one fixed
family. Proven on the fixture, the render, and over a socket.

The fork, held for the owner:

- **(a) As implemented.** Classified before dispatch, pure.
- **(b) Thread a `&PlanCounters` through `dispatch`** — exact for the
  two approximations, at every planner signature.
- **(c) Decline and revert.**

## Consequences

- Positive (a): an operator's scrape now shows which path each planned
  read took; `dispatch` and every planner function unchanged; the
  classification costs one `describe()` and a pure plan per read
  (measured: the planner rows within noise, every row moves in both directions by the container's usual run-to-run spread — `index-eq` `query` 1,195.6 → 1,412.8 µs, `eq-range` `query` 256.2 → 103.0, `range-tight` `count(*)` 48.2 → 53.9, `index-range` `fpage-50` 132.5 → 127.1, `due-window` 1,104.7 → 347.2 — with no row moving consistently one way and the sub-100 µs rows, where a per-read `describe()` would show first, within ~6 µs).
- Named, not hidden: the two approximations above; no per-table label;
  no "walk abandoned" counter yet; the `Metrics` text gains a family
  (its grammar, not its line set, was the contract).

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.71.0 /
  `FR-083`; see the design's change history for the proof and the
  measurement. Builder: Claude, under the host-takeover convention;
  independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 640 (up from 638), `server_metrics_integration`
  4 (up from 3), every other target unchanged and green, 928
  tests across 39 targets, 0 failed, every pre-existing test
  unmodified. Measured (`benches/server.rs`'s existing planner rows,
  100K records over a real loopback socket, an idle 4-core Linux
  container, both binaries back to back): every row within noise
  (every row moves in both directions by the container's usual run-to-run spread — `index-eq` `query` 1,195.6 → 1,412.8 µs, `eq-range` `query` 256.2 → 103.0, `range-tight` `count(*)` 48.2 → 53.9, `index-range` `fpage-50` 132.5 → 127.1, `due-window` 1,104.7 → 347.2 — with no row moving consistently one way and the sub-100 µs rows, where a per-read `describe()` would show first, within ~6 µs) — `RESULTS.md`. The fork above remains the owner's at
  review; (a) is what merges if the PR merges unchanged.
