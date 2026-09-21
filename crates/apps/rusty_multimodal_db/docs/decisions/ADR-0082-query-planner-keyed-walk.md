# ADR-0082: Query Planner Step Seven — Aggregates Over the Walked Key

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0081` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  `ADR-0081`'s own option (b); the fork below is held open for the
  owner at review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLANNER-KEYED-WALK-DESIGN.md` (the
  full design), `ADR-0081` (whose option (b) — `MIN`/`MAX` of the range
  field — this widens to the four reductions), `ADR-0035` (`Aggregate`'s
  reductions and empties, reproduced exactly), `ADR-0075`/`ADR-0079`/
  `ADR-0081` (`RangeBy`, extended by one method), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `RangeBy::range_keys`
  (generic, `Ordered` the one implementation), a defaulted
  `ConnectionStore::range_keys` (`Memory`/`Relation`/`Reminder`),
  `keyed_walk_applies`/`keyed_walk` over `ADR-0081`'s `counted_walk`;
  no wire, protocol, client, or file-format change.

## Context

`ADR-0081` counted a pure range from the sorted index without a decode
and named `MIN`/`MAX` of the range field as the next slice — the walk's
first and last key. Read against the index, the walk yields every key
in range, so `SUM` and `AVG` of that field are reductions over the same
keys, no decode either. The consumer's "next due" is `MIN(due_at) WHERE
due_at > now`; measured before this round it decoded every reminder in
range: 26,800.6 µs.

## Decision

Implement: `RangeBy::range_keys(lower, upper) -> Vec<Key>` on `Ordered`
over a `guarded_pairs` iterator `guarded_range` now shares, exposed on
`GenericProductionStore`; `ConnectionStore::range_keys` with an
`Unsupported` default, implemented by the three range-indexed adapters;
`keyed_walk_applies` (`counted_walk_applies`'s shape with every
aggregate `COUNT(*)` or `SUM`/`AVG`/`MIN`/`MAX` of the range field) and
`keyed_walk` (count-only → `counted_walk`; otherwise one `range_keys`
and each column reduced over it exactly as `evaluate_aggregate` reduces
rows, empties included, in the field's schema kind); the `Aggregate` arm
asks it first. Every answer is the decode path's, proven on the
fixture, the generic index, and over a socket.

The fork, held for the owner:

- **(a) As implemented.** The four reductions of the range field.
- **(b) (a) plus `GROUP BY` the range field** — buckets by equal key
  over the walked keys. A second round.
- **(c) Decline and revert.**

## Consequences

- Positive (a): `due-next` 26,800.6 → 624.7 µs, `due-mean`
  30,822.6 → 679.5; a wide range materializes 8 bytes per key
  instead of a decoded record.
- Named, not hidden: an aggregate over any other field still decodes;
  `GROUP BY` still decodes; a generic-layer addition (`range_keys`)
  registered under this FR as the rounds before did.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.67.0 /
  `FR-079`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 633 (up from 630), `server_sql_integration`
  57 (up from 56), every other target unchanged and green, 917
  tests across 39 targets, 0 failed, every pre-existing test
  unmodified. Measured (`benches/server.rs`'s new `reminder-due`
  `due-next`/`due-mean` rows, 100K reminders over a real loopback
  socket, an idle 4-core Linux container, added and measured on the
  pre-change code first): `MIN(due_at_unix_ms) WHERE due_at_unix_ms >
  50000` 26,800.6 → 624.7 µs; `AVG(due_at_unix_ms) WHERE
  due_at_unix_ms <= 50000` 30,822.6 → 679.5 — `RESULTS.md`. The
  fork above remains the owner's at review; (a) is what merges if the
  PR merges unchanged.
- 2026-09-21: `ADR-0083` (bound tightening) the same day widened
  `keyed_walk` to any number of bounds per side, walking between the
  tightest two.
