# ADR-0085: A Hashed Bucket for `Aggregate`'s Decode Path

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0084` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  `ADR-0084`'s own option (b); the fork below is held open for the
  owner at review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-AGGREGATE-HASHED-BUCKET-DESIGN.md` (the
  full design), `ADR-0084` (whose option (b) this is, and whose
  measurement exposed the quadratic), `ADR-0035` (`evaluate_aggregate`),
  `ADR-0041` (`StrList` refused as a key), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Internal to `src/server/serve.rs`:
  a private `BucketKey` and a `HashMap` slot index in
  `evaluate_aggregate`. No planner, generic, adapter, wire, protocol,
  or client change.

## Context

`evaluate_aggregate` (`ADR-0035`) found each decoded row's bucket by a
linear search over the buckets so far — quadratic in the groups.
`ADR-0084` measured it (1,000 groups 2,491.3 µs, 5,000 groups
38,638.8) and moved the pure-range `GROUP BY` onto the walk; every
other `GROUP BY` still decodes and still searches. Measured before this
round, the same 5,000-stamp histogram with `status = pending` beside
the key — 1,250 groups the walk cannot answer — cost 4,543.6 µs.

## Decision

Implement: a private `BucketKey` mirroring `ScanValue` with `Eq +
Hash` (`F64` by bits; neither `F64` nor `StrList` can be a group key,
so equality is `ScanValue`'s on every key that occurs, and the
conversion stays total); `evaluate_aggregate` hashes each row's key to
its bucket's slot, opening a new bucket at the end for a new key. The
buckets, their first-seen order, their values, and `limit`'s
truncation are unchanged; the whole-table bucket takes no hash.
Proven in-process and over a socket.

The fork, held for the owner:

- **(a) As implemented.** A private key, a slot map.
- **(b) `Hash + Eq` on `ScanValue` itself** — a wire-type decision over
  `F64`; declined here.
- **(c) Decline and revert.**

## Consequences

- Positive (a): `due-hist-5k-pending` 4,543.6 → 2,572.7 µs; every
  decoded `GROUP BY` is linear in its rows, whatever the group count.
- Named, not hidden: one key clone per row (the linear search cloned
  the key too); the decode path still decodes — the walk (`ADR-0084`)
  is still the answer where it applies.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.70.0 /
  `FR-082`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 638 (up from 637), `server_sql_integration`
  60 (up from 59), every other target unchanged and green, 925
  tests across 39 targets, 0 failed, every pre-existing test
  unmodified. Measured (`benches/server.rs`'s new `reminder-due`
  `due-hist-5k-pending` row — 1,250 groups through the decode path,
  100K reminders over a real loopback socket, an idle 4-core Linux
  container, added and measured on the pre-change code first):
  4,543.6 → 2,572.7 µs — `RESULTS.md`. The fork above remains the
  owner's at review; (a) is what merges if the PR merges unchanged.
