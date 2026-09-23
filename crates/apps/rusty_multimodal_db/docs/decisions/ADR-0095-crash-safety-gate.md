# ADR-0095: The Crash-Safety Trials Run in CI

- Status: **Proposed and implemented on one branch; the owner asked for
  the hardening** (2026-09-21). The fourth "do now" item of the
  release-readiness review, after `ADR-0092`–`ADR-0094`.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/STORAGE-CRASH-SAFETY-GATE-DESIGN.md` (the full
  design), `STORAGE-021` (the registered unit),
  `src/bin/crash_safety_harness.rs`/`src/bin/crash_writer.rs` (the
  diagnosis round whose four trials these are), `STORAGE-017` (the
  trailing `COMMITTED` marker the torn-write trial proves),
  `docs/PROJECT-STATUS.md` (where the trials' findings lived, as prose).
- Supersedes/Superseded by: none. Additive: `tests/crash_safety.rs`
  (`required-features = ["research"]`, so `cargo test --all-features`
  — CI's `test` job — runs it). No `src/` change; the harness binary is
  untouched.

## Context

Crash safety was real and reproduced — a subprocess `SIGKILL`ed at a
chosen line, the file reopened cold — but only by a hand-run
diagnostic binary whose verdicts were printed, never asserted, and
whose findings were recorded in prose. A regression in the commit
marker, the reopen reconciliation, or `Flush` would have been
invisible to `cargo test`. The release-readiness review rated this
High.

## Decision

Implement: the harness's four trials as one integration test target,
each asserted — every flushed update survives; a slot torn after its
id or after its value is excluded on reopen and reseeded from the
caller's record, while the uninterrupted control keeps the attempted
value; a torn in-place update reads as exactly one of the two written
patterns; an unflushed kill leaves no record with a value it was never
given (the survivor count printed, not asserted — that is the page
cache's promise, not this crate's). Three repeats, not eight: the
kills land on sync lines, so a regression is deterministic, and the
gate runs on every push. The harness stays as the diagnostic it is.

## Consequences

- Positive: the crate's one empirical crash-safety claim is a gate,
  not a report; a regression fails CI in about two seconds.
- Negative / tradeoffs: the test duplicates the harness's trial
  mechanics (~150 lines) rather than sharing them through a library
  module — the harness is a closed diagnostic and the test is the
  gate; folding both onto one research-gated module is option (b).
- Named, not hidden: `SIGKILL` leaves the page cache intact, so this
  proves process-crash survival, never power-loss durability — the
  harness's own caveat, unchanged. Unix only (`#![cfg(unix)]`; the
  crate's servers are Linux-only in CI).

## Acceptance and implementation

- 2026-09-21: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `STORAGE-021` v0.1.0. `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — `crash_safety` 4 (new), 958 tests across 43 targets, 0 failed. Builder:
  Claude; independent Codex inspection owed.
