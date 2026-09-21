# ADR-0084: Query Planner Step Nine — `GROUP BY` the Walked Key

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0083` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  `ADR-0082`'s own option (b); the fork below is held open for the
  owner at review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLANNER-KEYED-GROUPS-DESIGN.md`
  (the full design), `ADR-0082` (whose option (b) this is), `ADR-0035`
  (`Aggregate`'s keyed buckets and `limit`), `ADR-0074` (the group
  order contract), `ADR-0083` (the tightest bounds), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive to `src/server/serve.rs`:
  `keyed_walk_applies` admits `group_by == [range_field]`; `keyed_walk`
  yields one group per run of equal keys. No generic, adapter, wire,
  protocol, client, or file-format change.

## Context

`ADR-0082` reduced the four aggregates of the range field over the
walked keys and named `GROUP BY` that field as the next shape. The walk
yields the keys in order, so a group per distinct key is a run of equal
adjacent keys: one pass, no decode, no bucket search. The decode path
buckets each decoded row by a linear search over the groups so far, so
it is quadratic in the number of groups — measured before this round,
1,000 due-stamp groups cost 2,201.8 µs and 5,000 cost 38,647.5.

## Decision

Implement: `keyed_walk_applies` accepts a `group_by` of exactly the
range field (the rest as `ADR-0082`/`ADR-0083`); `keyed_walk`, grouped,
splits the walked keys by runs of equality and builds one
`AggregateGroup` per run — the key in the field's schema kind, each
column reduced over the run exactly as `evaluate_aggregate` reduces a
bucket (`COUNT` the length, `SUM` the sum, `AVG` the key as `F64`,
`MIN`/`MAX` the key), none over an empty walk, `limit`-truncated. Under
a bound the decode path's candidates are the same walk, so its groups
are sequence-identical; with no bound they are the same set in scan
order (within `ADR-0074`'s "unspecified order"), so the one shape
where a `limit` would truncate a different set — grouped, no bound, a
`limit` — takes the decode path. Proven on the fixture, the eligibility
matrix, and over a socket on `Memory` and `Reminder`.

The fork, held for the owner:

- **(a) As implemented.** Runs over the walked keys.
- **(b) (a) plus a `HashMap` bucket in `evaluate_aggregate`** — the
  decode path's quadratic search fixed for every `GROUP BY`; a
  separate, non-planner round.
- **(c) Decline and revert.**

## Consequences

- Positive (a): `due-hist` (1,000 groups) 2,491.3 → 299.8 µs;
  `due-hist-5k` (5,000 groups) 38,638.8 → 1,437.1; the grouped
  answer is linear in the range, not quadratic in the groups.
- Named, not hidden: `GROUP BY` any other field, or the key beside
  another, still decodes and still searches; the decode path's own
  bucket search is left as it was; one shape (grouped, no bound, a
  `limit`) is held back so that truncation never picks a different set.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.69.0 /
  `FR-081`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 637 (up from 635), `server_sql_integration`
  59 (up from 58), every other target unchanged and green, 923
  tests across 39 targets, 0 failed, every pre-existing test
  unmodified. Measured (`benches/server.rs`'s new `reminder-due`
  `due-hist`/`due-hist-5k` rows — `due_at_unix_ms, COUNT(*) … GROUP BY
  due_at_unix_ms` over a 1,000- and a 5,000-stamp window, every stamp
  distinct, 100K reminders over a real loopback socket, an idle 4-core
  Linux container, added and measured on the pre-change code first):
  2,491.3 → 299.8 µs and 38,638.8 → 1,437.1 —
  `RESULTS.md`. The fork above remains the owner's at review; (a) is
  what merges if the PR merges unchanged.
- 2026-09-21: option (b) taken the same day as `ADR-0085` — the decode
  path's bucket hashed.
