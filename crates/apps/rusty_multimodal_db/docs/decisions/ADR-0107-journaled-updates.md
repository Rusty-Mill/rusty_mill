# ADR-0107: `UpdateField` Through the Journal, Opt-In

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "3" — `ADR-0097`'s option (c), held open). No
  wire change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0097`/`docs/design/SERVER-SYNCED-UPDATES-DESIGN.md`
  (`SYU-FR-002`, `msync` before the acknowledgement), `ADR-0025`/
  `ADR-0026` (the journal and its group commit), `ADR-0063` (the
  journal's entry kinds), `RESULTS.md` ("`UpdateField` through the
  journal — measured").
- Supersedes/Superseded by: amends `SYU-FR-002` with a second way to
  make an in-place update durable before its acknowledgement.
  Additive: `with_journaled_updates` on the three adapters, one
  private helper each, `SERVER_JOURNAL_UPDATES`.

## Context

`ADR-0097` made an in-place update durable by `msync`ing the table's
mapping before acknowledging, ≈100–150 µs per update on a small
table. The ranged-`msync` probe (PR #306) showed that cost is the
filesystem's per-file work and grows with the mapping (≈1.6 ms at
1 GB), and that a narrower `msync` changes nothing. The journal
already makes a batch durable with one sequential append and one
`fsync` that a group of concurrent commits shares. Whether that path
is cheaper for an in-place update was a measurement, not an argument
— the previous round's reasoning said "no gain for one connection",
which turned out to be true only on a small table.

## Decision

Implement, opt-in: `with_journaled_updates(true)` on a journaled
`Memory`/`Entity`/`Relation` adapter makes `update_field` validate
the value as before and then commit `TransactionOp { id, field,
value }` through `apply_transaction`'s journaled arm — the entry
appended and `fsync`ed by `CommitGroup`, the slot written under the
exclusive section, the MVCC index recorded, a checkpoint when due —
with `update_field`'s own answers (`Ok(false)` for a missing record,
the validation errors before anything is journaled, `Journal` when
the entry could not be made durable). `memory_server` reads
`SERVER_JOURNAL_UPDATES` (presence-gated, off by default, an error
without `SERVER_TXN_JOURNAL_PATH`). Measured before shipping, release
build, 2,000 updates: one writer 219.7 vs 144.5 µs at 1K rows, 202.4
vs 147.1 at 100K, 242.1 vs 313.7 at 1M; eight writers sharing the
adapter 91.0 vs 176.8, 87.8 vs 178.6, 94.8 vs 280.1 (journal vs
`msync`). The setting is opt-in because the crossover is real: a
single writer on a small table keeps `msync`; concurrent writers or a
large table take the journal.

## Consequences

- Positive: a journaled multi-writer deployment halves its per-update
  cost; a large table's updates no longer pay for its mapping's
  `msync`; a journaled update is replayed after a crash like any
  batch, and recorded in the MVCC index (which the in-place path does
  not do — see below).
- Negative / tradeoffs: two settings for one property (durable before
  the acknowledgement), the operator choosing by table size and
  writer count; the previous round's "no gain" reasoning is corrected
  in `RESULTS.md` rather than deleted.
- Named, not hidden: the in-place `update_field` path (no journal, or
  the setting off) has never recorded its write in the MVCC version
  index — an ordinary write invisible to `mvcc_get`'s history, a
  pre-existing gap found while building this and left for the
  independent review, not widened or narrowed here. *Closed by
  `ADR-0108` (2026-09-23).*

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.87.0 / `FR-099`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 970 tests across 44 targets, 0 failed. Builder: Claude; independent Codex inspection owed.
