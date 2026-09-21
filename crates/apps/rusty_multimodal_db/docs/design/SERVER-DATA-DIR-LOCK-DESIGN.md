# Server Data Directory: One Process per Directory, Durable Installs (Proposed and implemented)

- Status: **Proposed and implemented on one branch; the owner asked
  for it** (2026-09-21, `ADR-0092`). The first "do now" item of the
  release-readiness review, taken on the owner's word ("harden").
- Date: 2026-09-21
- Related: `ADR-0053`/`docs/design/SERVER-DATA-DIR-DESIGN.md`
  (`SERVER_DATA_DIR`), `ADR-0046`/`STORAGE-017` (the `O_APPEND` slot
  append — the one multi-process case the library handles — and the
  single-process SAFETY assumption under every `MmapMut::map_mut`),
  `ADR-0052` (`Compact` rewrites a slot file in place via rename),
  `ADR-0065` (`Backup` copies `<stem>*`; the lock file is not a stem),
  `src/bin/multiprocess_harness.rs` (two processes on one file, by
  design, unchanged), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `src/server/data_lock.rs`,
  `durability::sync_parent_dir`, one call site in `memory_server`,
  six after existing renames/creates, `rust-version` 1.89.

## Purpose and scope

The mmap stores assume one process owns the file. Nothing enforced it
at the deployment boundary: a second `memory_server` on the same
`SERVER_DATA_DIR` opened cleanly and could `Compact` a file the first
still had mapped. And every temp-then-rename install `sync_all`ed the
file, never its directory, so the rename was not durable until the
kernel's own write-back.

Scope, exactly: the lock (`DDL-FR-001`); the wiring (`DDL-FR-002`);
the refusal, proven across real processes (`DDL-FR-003`); the
directory `fsync` (`DDL-FR-004`); no other surface changed
(`DDL-FR-005`).

## Non-goals

- **A lock inside the library.** `GenericProductionStore`/`SlotFile`
  are unchanged; the two-process harness still races two writers on
  one file, as the `O_APPEND` regression check it is. Option (b).
- **The update path's own `msync`.** An acknowledged `UpdateField`
  outside the journal stays in the page cache until `Flush`, a
  checkpoint, or write-back — the documented loss window
  (`crash_safety_harness`'s "unflushed loss" trial); the journal
  (`SERVER_TXN_JOURNAL_PATH`) is the durable-ack setting. Not this
  round.
- **NFS.** `flock` there is the mount's promise, as `O_APPEND`'s was.
- **A stale-lock tool.** There is no stale lock: the kernel releases
  it with the descriptor.

## Context and terminology

Read from `main` after PR #290 this pass:

- **`DataDirLock::acquire(dir)`**: `create_dir_all(dir)`; open
  `<dir>/.rusty_multimodal_db.lock` read/write, create, no truncate;
  `File::try_lock()` — `Ok` holds it; `WouldBlock` is
  `DataDirLockError::Held { path }`; any other error
  `DataDirLockError::Io { path, source }`. `Drop` unlocks; process exit
  of any kind releases it.
- **`sync_parent_dir(path)`**: `File::open(parent)?.sync_all()` on
  Unix; a no-op elsewhere (a directory is not openable as a file on
  Windows, and this crate's servers are Linux-only in CI).
- **Install points**: `durability::record_blob::RecordBlob::write`,
  `MmapAgeStore::write_via_rename`, `SlotFile::create` (after its
  first flush) and `SlotFile::rewrite`, `insert_log::upgrade_if_version_1`,
  `MvccIndex::flush`'s history write — all at open, `Compact`, or a
  journal checkpoint; none per request.

## Requirements

- `DDL-FR-001` **The lock.** As "Context"; a second claim in the same
  process is `Held` (the lock is per open file description); a
  dropped claim frees it; a missing directory is created; a path that
  is a file is `Io`.
- `DDL-FR-002` **The wiring.** `memory_server` in durable mode claims
  `SERVER_DATA_DIR` before `open_stores` and holds the claim in
  `main`'s scope; scratch mode (a per-process temp path) takes none.
- `DDL-FR-003` **The refusal, proven across processes.** Against the
  compiled binary: the running server's lock file exists; a second
  server on the same directory exits non-zero with "held by another
  process" and the lock file's name on stderr and never listens; after
  the first exits a third starts.
- `DDL-FR-004` **Durable installs.** `sync_parent_dir` after each
  install point named in "Context".
- `DDL-FR-005` **Everything else unchanged.** Library, harness, wire
  (protocol 29), clients; `rust-version` 1.88 → 1.89 for
  `File::try_lock`, the CI `msrv` pin with it.

## Considered options

- **(a) A server-level lock through `std`, plus the directory
  `fsync` — implemented.**
- **(b) The lock inside `SlotFile`.** Every open of every store, the
  harness's premise broken, and a stance ("not ruled out") reversed
  in the library rather than at the deployment.
- **(c) `libc::flock`.** A new direct dependency for one call the
  standard library has had since 1.89.
- **(d) A pid file.** Stale after `SIGKILL`; needs a tool to clear.
- **(e) Decline.**

The owner's shorthand: **(a)** as implemented; **(b)** push the lock
into the library; **(e)** decline and revert.

## Proposed shape

`src/server/data_lock.rs` (new); `src/server/mod.rs` (`pub mod
data_lock`); `src/durability/mod.rs` (`sync_parent_dir`); the six
install points; `src/bin/memory_server.rs`; `Cargo.toml`
(`rust-version`, the `[[test]]`); `.github/workflows/ci.yml` (the pin);
`tests/memory_server_data_dir_lock.rs` (new).

## Data/state and invariants

- While a `DataDirLock` on `dir` lives, no other `acquire(dir)` in any
  process succeeds.
- After `sync_parent_dir` returns, the directory entry the preceding
  rename or create made is on disk.

## Errors, failure, recovery, and observability

`Held` names the lock file; `memory_server` panics with it before
listening (its "misconfiguration is a startup error" convention).
Recovery is the holder exiting — nothing to clean.

## Security, privacy, and compatibility

No wire change. The lock file is empty, under the data directory,
outside `Backup`'s stem glob. MSRV 1.89.

## Acceptance criteria

1. Unit: `DDL-FR-001` (three tests in `data_lock.rs`).
2. Integration: `DDL-FR-003` against the binary.
3. Not measured: nothing per request changed; one `fsync` per install
   at open/`Compact`/checkpoint.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`.
Independent review owed.

## Traceability

- Roadmap: `SERVER-DATA-DIR-LOCK`.
- Decision: `ADR-0092`.
- Specification: `SERVER-001` v0.77.0 / `FR-089`; `STORAGE-017` 0.1.1.
- Requirements: `DDL-FR-001`–`005`.

## Open questions

- **The library-level lock** — option (b), if the owner wants the
  store itself to refuse a second mapping.
- **The update path's durability** — the review's remaining half of
  this item: `msync` per `UpdateField`, or the journal as the answer,
  documented. Next round.

## Change history

- 2026-09-21: proposed and implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` on the owner's word
  ("harden"), the first "do now" item of the release-readiness review.
- 2026-09-21: implemented as `SERVER-001` v0.77.0 / `FR-089`, `STORAGE-017`
  0.1.1. Acceptance criteria 1–2 are the tests: `data_lock.rs` +3,
  `tests/memory_server_data_dir_lock.rs` +1. `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 650 (up from 647), `memory_server_data_dir_lock` 1 (new), 944 tests across 40 targets, 0 failed. Still no
  independent review — owed.
