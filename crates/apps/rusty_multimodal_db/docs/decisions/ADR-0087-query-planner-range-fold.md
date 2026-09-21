# ADR-0087: Query Planner Step Ten — A Fold That Materializes No Key

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0086` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  the "fold that never materializes the keys" open question `ADR-0082`
  named from its own measurement; the fork below is held open for the
  owner at review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLANNER-RANGE-FOLD-DESIGN.md` (the
  full design), `ADR-0082` (whose open question this is), `ADR-0081`
  (`range_count`, the walk that keeps nothing), `ADR-0084` (the grouped
  walk, unchanged), `ADR-0075`/`ADR-0079` (`RangeBy`, extended by one
  method), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `RangeBy::range_fold`
  (generic, `Ordered` the one implementation), `server::KeyStats`, a
  defaulted `ConnectionStore::range_stats` (`Memory`/`Relation`/
  `Reminder`); `keyed_walk`'s ungrouped shape over it. No wire,
  protocol, client, or file-format change.

## Context

`ADR-0082` reduced the range field from the walked keys and measured
what remained: the `Vec<i64>` of every key in range (`due-next` ~600 µs
against `due-count`'s ~290 for the same walk). Each reduction is a
fold, so the keys need never exist — `range_count` already walks
without keeping anything.

## Decision

Implement: `RangeBy::range_fold(lower, upper, init, f)` on `Ordered`
over the shared `guarded_pairs` walk, exposed on
`GenericProductionStore`; `KeyStats { count, sum, min, max }` with
`with(&key)` as the fold step; `ConnectionStore::range_stats` with an
`Unsupported` default, implemented by the three range-indexed adapters;
`keyed_walk` reduces every column from a `KeyStats` — the ungrouped
shape from one `range_stats`, the grouped shape from each run of the
still-materialized keys. Eligibility, `plan_query`, `counted_walk`, and
the wire are untouched. Every answer is `ADR-0082`'s, proven on the
fixture, the generic index, and over a socket.

The fork, held for the owner:

- **(a) As implemented.** A generic fold and a `KeyStats`.
- **(b) (a) plus a run-emitting fold for the grouped walk** — so
  `GROUP BY` the key materializes nothing either. A second round.
- **(c) Decline and revert.**

## Consequences

- Positive (a): `due-next` 610.3 → 581.6 µs, `due-mean`
  705.9 → 574.0; no allocation proportional to the range for
  an ungrouped reduction.
- Named, not hidden: the grouped walk still materializes its keys; a
  generic-layer addition (`range_fold`) registered under this FR as the
  rounds before did.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.72.0 /
  `FR-084`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 642 (up from 640), `server_sql_integration`
  61 (up from 60), every other target unchanged and green, 931
  tests across 39 targets, 0 failed, every pre-existing test
  unmodified. Measured (`benches/server.rs`'s `reminder-due`
  `due-next`/`due-mean` rows, 100K reminders over a real loopback
  socket, an idle 4-core Linux container, both binaries back to back):
  `MIN(due_at_unix_ms) WHERE due_at_unix_ms > 50000` 610.3 →
  581.6 µs; `AVG(due_at_unix_ms) WHERE due_at_unix_ms <= 50000`
  705.9 → 574.0 — `RESULTS.md`. The fork above remains the
  owner's at review; (a) is what merges if the PR merges unchanged.
