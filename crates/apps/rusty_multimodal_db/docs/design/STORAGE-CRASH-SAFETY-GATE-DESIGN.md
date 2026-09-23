# Storage Crash-Safety Gate: The Harness's Trials as CI Tests (Proposed and implemented)

- Status: **Proposed and implemented on one branch; the owner asked
  for it** (2026-09-21, `ADR-0095`). The fourth "do now" item of the
  release-readiness review.
- Date: 2026-09-21
- Related: `src/bin/crash_safety_harness.rs` (the diagnosis; its
  module docs are the authoritative account of what each trial can
  and cannot prove), `src/bin/crash_writer.rs` (the child), `STORAGE-017`
  (the `COMMITTED` marker), `STORAGE-012` (`GenericMmapStore::open`'s
  reconciliation), `ADR-0092` (`sync_parent_dir`, the round before
  last).
- Supersedes/Superseded by: none. Additive: one test target.

## Purpose and scope

Turn the four reproduced crash-safety findings into assertions
`cargo test --all-features` runs on every push, using the same real
`crash_writer` subprocess and `SIGKILL` the harness uses, so that the
commit marker, the reopen reconciliation, and `Flush` cannot regress
silently.

Scope, exactly: the flushed trial (`CSC-FR-001`); the torn-write trial
with its control (`CSC-FR-002`); the torn-update trial (`CSC-FR-003`);
the unflushed trial (`CSC-FR-004`); registration so CI runs it
(`CSC-FR-005`); nothing else changed (`CSC-FR-006`).

## Non-goals

- **Power-loss durability.** `SIGKILL` leaves the page cache intact;
  no block-layer fault injection is attempted.
- **Changing the harness.** It stays the hand-run diagnostic with its
  eight repeats and its printed report.
- **Sharing code with the harness.** Option (b); the trial mechanics
  are ~150 lines and the two have different jobs.
- **Windows.** `#![cfg(unix)]`.

## Context and terminology

Read from `main` after PR #294 this pass:

- **`crash_writer` modes**: `unflushed-updates <path> <count>`
  (prints `WROTE <i>` per update, never flushes), `flushed-updates
  <path> <count>` (updates, `Flush`, prints `FLUSHED`, sleeps),
  `torn-write <path> <existing> <id> <value>` (prints `ID_WRITTEN`,
  then `VALUE_WRITTEN`, then the marker), `torn-update <path> <id>
  <iterations>` (alternates two 8-byte patterns in place).
- **Kill on a line**: read the child's stdout until the sync line,
  `Child::kill` (`SIGKILL` on Unix), reap; the child must not have
  exited on its own.
- **Kill after a delay**: for the torn update, 2 ms then kill — a sync
  line would only ever let the kill land between updates.
- **Reopen**: `GenericMmapStore::<Order, Status, Amount>::open(records,
  path)` — the real public API, records supplied as the harness does.

## Requirements

- `CSC-FR-001` **Flushed.** Killed right after `FLUSHED`: all 500
  updates visible after reopen, each of 3 repeats.
- `CSC-FR-002` **Torn write.** The control run (never killed) reads
  the attempted value; killed after `ID_WRITTEN` and after
  `VALUE_WRITTEN`, 3 repeats each, the new id reads the reseed value —
  the torn slot excluded, the caller's record seeded.
- `CSC-FR-003` **Torn update.** Killed 2 ms into a 500 M-iteration
  burst, the value is exactly pattern A or pattern B, 3 repeats.
- `CSC-FR-004` **Unflushed.** Killed after 250 of 500 `WROTE` lines:
  every record reads its seed or its update; the survivor count is
  printed, not asserted.
- `CSC-FR-005` **In CI.** `[[test]] name = "crash_safety"`,
  `required-features = ["research"]`; the crate's `cargo test
  --all-features` job runs it.
- `CSC-FR-006` **Everything else unchanged.** No `src/` change; the
  harness and writer binaries untouched.

## Considered options

- **(a) A self-contained test target — implemented.**
- **(b) Extract the trial mechanics into a research-gated library
  module** shared by the harness and the test. Cleaner; touches the
  closed diagnostic.
- **(c) Run the harness binary from CI and grep its report.** A
  printed `PASS` is not an assertion, and the report's shape would
  become an interface.
- **(d) Decline.**

The owner's shorthand: **(a)** as implemented; **(b)** share the
mechanics; **(d)** decline and revert.

## Proposed shape

`tests/crash_safety.rs` (new); `Cargo.toml` (the `[[test]]`).

## Data/state and invariants

- Each trial's temp directory is fresh and removed on drop.
- A child that exits cleanly before the kill fails the trial: the kill
  must land mid-work or the trial proves nothing.

## Errors, failure, recovery, and observability

A failed assertion names the trial, the repeat, and the observed
value. The unflushed survivor count is on the test's stdout.

## Security, privacy, and compatibility

None: a test target.

## Acceptance criteria

1. The four tests, green.
2. Runtime: about two seconds for the target.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`.
Independent review owed.

## Traceability

- Roadmap: `STORAGE-CRASH-SAFETY-GATE`.
- Decision: `ADR-0095`.
- Specification: `STORAGE-021` v0.1.0.
- Requirements: `CSC-FR-001`–`006`.

## Open questions

- **Sharing the mechanics** — option (b).
- **Power loss** — a block-layer fault-injection round, if ever.

## Change history

- 2026-09-21: proposed and implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` on the owner's word ("4"),
  the fourth "do now" item of the release-readiness review.
- 2026-09-21: implemented as `STORAGE-021` v0.1.0. Acceptance criterion 1 is
  the four tests. `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — `crash_safety` 4 (new), 958 tests across 43 targets, 0 failed. Still no independent review — owed.
