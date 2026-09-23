# Server Synced Updates: `msync` Before an In-Place Update Is Acknowledged (Proposed and implemented)

- Status: **Proposed and implemented on one branch; the owner asked
  for it** (2026-09-21, `ADR-0097`, "durability").
- Date: 2026-09-21
- Related: `ADR-0046` (insert log, `sync_data` per entry),
  `ADR-0025`/`ADR-0063` (the journal), `STORAGE-017`
  (`SlotFile::write_value`), `ADR-0092`'s design (which named this as
  the round's remaining half), `RESULTS.md` (the per-write mmap flush
  row, 55.3 µs, the number this round re-measures at the adapter).
- Supersedes/Superseded by: none. Additive: one builder, one private
  helper, one check per non-journaled batch arm, per adapter; one
  `memory_server` setting.

## Purpose and scope

Close the last write path whose acknowledgement preceded its
durability, at the operator's choice and at a measured cost.

Scope, exactly: the setting (`SYU-FR-001`); `UpdateField`
(`SYU-FR-002`); the non-journaled batches (`SYU-FR-003`); the binary
(`SYU-FR-004`); measured (`SYU-FR-005`); everything else unchanged
(`SYU-FR-006`).

## Non-goals

- **A ranged `msync`.** `Flush` on the stack flushes every mapped
  slot file; the kernel writes only dirty pages. Option (b).
- **Journaling `UpdateField`.** Option (c): the journal's redo entry
  would give the same durability with group commit; a larger change.
- **Defaults on.** The fork for the owner.
- **The research domains.** `Dog`/`Order`/`Employee` unchanged.
- **The MVCC history store.** Already flushed at its own boundaries.

## Context and terminology

Read from `main` after PR #296 this pass:

- **The in-place paths**: `update_field` (one `self.store.update` per
  adapter — `access_count`, `mention_count`, `updated_at_unix_ms`);
  `apply_transaction` and `apply_transaction_mvcc`'s `None` (no
  journal) arms, which `apply_batch` inside `with_exclusive`.
  `write_batch` carries no field update (`WriteOp` is
  `Insert`/`Replace`/`ReplaceIf`/`Delete`/`Link`, all through the
  `fsync`ed insert log).
- **`sync_update_ack`**: `Ok(())` unless `sync_updates`; else
  `self.store.flush()` (the stack's `Flush` under the write lock),
  `Err(Storage)` on failure. Called after a successful `update`.
- **In the exclusive arms**: `inner.checkpoint_flush()` (the same
  `Flush`, on the already-held store) between `apply_batch` and the
  MVCC bookkeeping; a failure is `(0, Storage)` and nothing is
  acknowledged.
- **`SERVER_SYNC_UPDATES`**: `1` sets `with_synced_updates(true)` on
  all three tables; anything else leaves the default.

## Requirements

- `SYU-FR-001` **The setting.** `with_synced_updates(bool)`; `false`
  in every constructor; the answer to every request identical either
  way.
- `SYU-FR-002` **`UpdateField`.** A successful in-place update is
  followed by the stack's `Flush` before `Ok(true)`; a `NotFound`
  answers `Ok(false)` with no flush.
- `SYU-FR-003` **Non-journaled batches.** `apply_transaction` and
  `apply_transaction_mvcc` without a journal flush after `apply_batch`
  and before acknowledging; the journaled arms are unchanged.
- `SYU-FR-004` **The binary.** `SERVER_SYNC_UPDATES=1` on all three
  tables; the ready banner says so.
- `SYU-FR-005` **Measured.** `update_field` with and without the
  setting at 1K and 100K rows, in `RESULTS.md`.
- `SYU-FR-006` **Everything else unchanged.** Wire, library, formats,
  the research domains, the journaled arms.

## Considered options

- **(a) `msync` the stack after each in-place update — implemented.**
- **(b) A ranged `msync` of the one slot.** Needs a path from the
  adapter through every layer to `SlotFile`; the kernel already
  limits the whole-mapping flush to dirty pages.
- **(c) Journal `UpdateField`.** Group commit amortizes the `fsync`;
  a change to the journal's entry kinds and replay.
- **(d) Document the window and decline.** The honest floor; the
  owner asked for more.

The owner's shorthand: **(a)** as implemented; **(b)** ranged; **(c)**
journal it; **(d)** decline and revert.

## Proposed shape

`src/server/{memory,entity,relation}.rs`: the field, the builder,
`sync_update_ack`, `update_field`, the two `None` arms.
`src/bin/memory_server.rs`: the setting, the banner, the module docs.

## Data/state and invariants

- With the setting, a `true`/`Ok(())` acknowledgement of an in-place
  update implies `msync` returned for every mapped slot file of the
  table.
- Without it, every behaviour is byte-for-byte the previous one.

## Errors, failure, recovery, and observability

An `msync` failure is `Storage`; the value is in the mapping and will
still reach disk by write-back, but the client was not told it did.

## Security, privacy, and compatibility

No wire change; no format change.

## Acceptance criteria

1. Unit (`memory.rs`): `SYU-FR-001`–`003` — the synced adapter's
   answers, and a reopen from the files alone.
2. `RESULTS.md`: `SYU-FR-005`.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`.
Independent review owed.

## Traceability

- Roadmap: `SERVER-SYNCED-UPDATES`.
- Decision: `ADR-0097`.
- Specification: `SERVER-001` v0.81.0 / `FR-093`.
- Requirements: `SYU-FR-001`–`006`.

## Open questions

- **Ranged `msync`** — option (b), if the measured cost at 100K rows
  says the whole-mapping flush is paying for page-table walks.
  *Measured and declined (2026-09-22, the owner's "2"): whole-mapping
  and one-page `msync` are the same number from 100 KiB to 3.8 GiB
  (`RESULTS.md`, "Ranged `msync` — measured and declined"); the
  kernel already flushes only the dirty pages, so the range changes
  nothing and the plumbing was not built.*
- **Journal `UpdateField`** — option (c). *Not taken with (b): a
  journal commit is one `fsync` of its own, the same floor for a
  single connection; group commit amortizes it only across concurrent
  writers, which the consumer is not. Open for a multi-writer
  deployment.* *Taken: `ADR-0107` (2026-09-23), the owner's "3" —
  opt-in `with_journaled_updates(true)` / `SERVER_JOURNAL_UPDATES=1`
  on a journaled adapter; measured: slower for one writer on a small
  table (≈220 vs ≈145 µs), faster at 1M rows (≈240 vs ≈310 µs), and
  half the cost under eight writers (≈90 vs ≈180 µs) — `RESULTS.md`.*
- **Defaults on** in `memory_server`. *Taken: `ADR-0099` (2026-09-21) —
  on unless `SERVER_SYNC_UPDATES=0`.*

## Change history

- 2026-09-21: proposed and implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` on the owner's word
  ("durability"), the review's remaining High finding.
- 2026-09-21: implemented as `SERVER-001` v0.81.0 / `FR-093`. Acceptance
  criterion 1 is `memory.rs` +1
  (`synced_updates_answer_exactly_as_unsynced_and_a_reopen_sees_them`);
  criterion 2 measured: `update_field` on `Memory`, release build, this container, 2,000 updates each: 0.1 → 105.4 µs per update at 1K rows, 0.2 → 99.7 µs at 100K rows (unsynced → synced); the `msync` costs what the insert log's per-entry `sync_data` costs, and does not scale with the table. `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 658 (up from 657), 962 tests across 43 targets, 0 failed. Still no independent review —
  owed.
