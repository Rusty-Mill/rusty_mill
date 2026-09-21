# ADR-0092: One Server Process per Data Directory, and Durable Installs

- Status: **Proposed and implemented on one branch; the owner asked for
  the hardening** (2026-09-21). The first item of the release-readiness
  review's "do now" list, taken on the owner's word ("harden").
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-DATA-DIR-LOCK-DESIGN.md` (the full
  design), `ADR-0053` (`SERVER_DATA_DIR`), `ADR-0046`/`STORAGE-017`
  (the `O_APPEND` fix and the single-process assumption under every
  `unsafe` mapping), `ADR-0052` (`Compact`, the rewrite a second
  process could not survive), `ADR-0065` (`Backup`'s stem glob, which
  the lock file's name stays outside), `docs/FUTURE-GROWTH.md` ("broader
  multi-writer coordination wasn't needed yet, not ruled out").
- Supersedes/Superseded by: none. Additive: `server::data_lock`
  (`DataDirLock`, `DataDirLockError`, `LOCK_FILE_NAME`),
  `durability::sync_parent_dir`, one call in `memory_server`'s `main`,
  six calls after existing renames/creates. `rust-version` 1.88 →
  1.89. No wire, protocol, library-API, or client change.

## Context

Every mmap-backed store in this crate documents, under its `unsafe`
mapping, that this process alone truncates, rewrites, or compacts the
file. Inside one process the `RwLock` upholds that; across processes
nothing did. Two `memory_server`s pointed at one `SERVER_DATA_DIR`
both started, both mapped, and a `Compact` on either rewrote a file
the other still had mapped — the one failure the SAFETY comments name.
The release-readiness review rated this High. Separately, every
write-to-temp-then-rename install in the crate `sync_all`ed the file
and never the directory, so the rename itself was not on disk until
the kernel got to it.

## Decision

Implement: `DataDirLock::acquire(dir)` — `std::fs::File::try_lock`
(an `flock(LOCK_EX | LOCK_NB)`; released by the kernel however the
process ends) on `<dir>/.rusty_multimodal_db.lock`; `memory_server`
takes it before opening any store in durable mode and holds it until
`main` returns; a second server on the same directory exits non-zero
naming the lock file, before it listens. `sync_parent_dir(path)` after
every rename install and slot-file creation. The library and the
two-process diagnosis harness are untouched: the interlock is the
deployment's, not the store's. `File::try_lock` is stable since Rust
1.89, one minor above the dependency ceiling; taken over a `libc`
dependency (a new direct dependency for one syscall) and over a
pid-in-a-file scheme (stale after `SIGKILL`).

## Consequences

- Positive: the single-process assumption is enforced where a
  deployment breaks it; a rename install survives power loss at the
  directory level, not just the file's.
- Negative / tradeoffs: the MSRV moves 1.88 → 1.89 (the owner's
  toolchain call, named here rather than folded in). Advisory only —
  a process that never opens the lock file is not stopped; every
  binary honouring `SERVER_DATA_DIR` does (one, `memory_server`, today).
  One directory `fsync` per install, at open, `Compact`, and journal
  checkpoint — never on a request's hot path (`RESULTS.md`: not
  measured, nothing per request changed).
- Named, not hidden: `flock` on NFS is the mount's own story, as
  `O_APPEND`'s atomicity already was. The library-level alternative —
  the lock inside `SlotFile` — is the fork held open for the owner.

## Acceptance and implementation

- 2026-09-21: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.77.0 / `FR-089`,
  `STORAGE-017` 0.1.1. `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 650 (up from 647), `memory_server_data_dir_lock` 1 (new), 944 tests across 40 targets, 0 failed. Builder: Claude; independent Codex
  inspection owed.
