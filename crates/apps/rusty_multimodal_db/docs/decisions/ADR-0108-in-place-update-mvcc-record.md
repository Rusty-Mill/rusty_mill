# ADR-0108: An In-Place `UpdateField` Is Recorded in the MVCC Index

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "2 and 3" — the gap `ADR-0107` named). No wire
  change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0072` (`MVCC2-FR-008`: every write path records into
  the version index inside the exclusive section that applied it),
  `ADR-0097` (the in-place path's `msync`), `ADR-0107` (which routed
  the journaled variant through `apply_transaction` and so recorded
  it, and named the in-place path as the one that did not).
- Supersedes/Superseded by: closes the gap `ADR-0107` named. Changes
  the in-place arm of `update_field` on `Memory`/`Entity`/`Relation`;
  nothing else.

## Context

`MVCC2-FR-008` says every write records into the version index so a
snapshot opened before it still reads the old value and a later
session's commit sees the conflict. `Insert`/`Replace`/`Delete`, the
session commit, the atomic `WriteBatch`, and (since `ADR-0107`) a
journaled `UpdateField` all do. The plain in-place `UpdateField` —
`self.store.update` then, optionally, `msync` — never did: it wrote
the slot and told the index nothing. On an MVCC-active table a
snapshot held across such an update read the *new* value (the index
had no entry, so the read fell through to the store), and a session
whose staged write raced it saw no conflict. The consumer's most
frequent write, `access_count`, is exactly this path.

## Decision

Implement: the in-place arm runs the slot write and the MVCC record
in one `with_exclusive` section — `UpdateField::update` on the inner
stack, then `mvcc_record_transaction` with the one op, exactly as
`apply_transaction`'s non-journaled arm does — before the optional
`msync`; a missing record records nothing and answers `Ok(false)` as
before. Proven in `memory.rs`: with a snapshot held, the update adds
one history entry, the snapshot still reads the old value, the
current value reads the new one, and a missing record adds nothing.

## Consequences

- *Amended by `ADR-0110`: the record was not flushed to `.mvcc`, so it held only until a restart; fixed.*
- Positive: `MVCC2-FR-008` holds on every write path; the two
  in-process reads (`mvcc_get` at a snapshot, the conflict check at
  commit) are correct against the consumer's most frequent write.
- Negative / tradeoffs: one more index entry per in-place update on an
  MVCC-active table — bounded by `ADR-0105`'s automatic reclaim; the
  write now takes the exclusive section explicitly rather than
  through `store.update`'s own lock, the same section every other
  write path takes.
- Named, not hidden: on a table that never goes MVCC-active,
  `mvcc_record_transaction` returns at once and nothing changes.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.88.0 / `FR-100`, in one PR with `ADR-0109`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 973 tests across 44 targets, 0 failed. Builder: Claude; independent Codex inspection owed.
