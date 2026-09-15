# STORAGE-020 — Restore tooling (`restore_backup` CLI)

- Version: 0.1.0
- Status: Accepted, Implemented, Verified
- Owners: baileyrd
- Depends on: `ADR-0065`/`STORAGE-014`–`016` (`Request::Backup`, the
  portable companion-file format this unit restores unchanged),
  `ADR-0066`/`STORAGE-019` (schema migration tooling — the
  "documented pattern plus one real, tested CLI, zero new `src/`
  library code, zero wire change" tier and the `examples/support/`
  placement precedent this unit follows exactly)
- Supersedes: none. Adds no new on-disk format, changes no existing
  one.

## Purpose and scope

`docs/FUTURE-GROWTH.md`'s Backup/restore bullet named the gap: *"a
backup is a portable, copy-safe directory ..., so 'restore' today
means manually pointing a server's data directory at the backup and
restarting it, not a request or tool that does that for you."*
`ADR-0065`'s own Decision text already asserted restore needs no new
server code — the produced directory is exactly what
`open`/`open_portable` already read — but that claim had never been
exercised by a real tool. This unit closes it: a real CLI,
`examples/restore_backup.rs`, that copies a `Request::Backup`-produced
directory into a fresh target, crash-safely, refusing an existing
target outright, and verifies the result via a real reopen through the
matching domain's own production constructor.

## Non-goals

Full list and reasoning: `docs/design/SERVER-RESTORE-DESIGN.md`'s own
Non-goals. In short: no `Request::Restore` wire operation or
`PROTOCOL_VERSION`/`SERVER-002` change (architecturally unsound for
this crate's `Arc`-shared, mmap-backed server model — see the design
doc's Context for why); no automatic/scheduled restore (an
operator-invoked CLI only); no corrupt-backup repair (a failed
verification reopen reports the real error and leaves the copied files
in place); no generic, record-type-agnostic engine (a `--domain`
argument names which portable constructor verifies, the same
per-migration specificity `STORAGE-019` already chose); no `Dog`
support (no shipped binary gives it a durable data directory today).

## Requirements

`RST-FR-001`–`006`, as designed in
`docs/design/SERVER-RESTORE-DESIGN.md`'s own "Requirements" section —
unchanged by implementation, no deviation:

- `RST-FR-001` a `main`-less `examples/support/restore_backup_lib.rs`
  plus a thin `examples/restore_backup.rs` CLI wrapper, no new `src/`
  library module.
- `RST-FR-002` copies every file in the backup directory into the
  target's parent, preserving each file's own name unchanged.
- `RST-FR-003` crash-safe per file: every file stages into one fresh
  temporary directory first; only once every file has staged does the
  tool move each staged file into place individually (per-file atomic,
  not one whole-group atomic swap — a named limitation, not hidden,
  since a restore's target may already hold sibling tables' own files
  and so has no single fresh name to rename onto as a group the way a
  backup's own target does).
- `RST-FR-004` refuses to touch a target that already has any file
  matching the target stem's own file-name prefix.
- `RST-FR-005` verifies the restored directory by actually opening it
  through the named domain's own portable production constructor,
  leaving copied files in place on failure.
- `RST-FR-006` prints the exact next step (the `SERVER_DATA_DIR`-shaped
  directory an operator now points a server binary at, and that it
  must be restarted).

## Architecture and interfaces

- `examples/support/restore_backup_lib.rs` (new; not a direct child of
  `examples/`, matching `STORAGE-019`'s own placement so Cargo's
  example autodiscovery does not also try to build this `main`-less
  file as its own example target): `Domain { Memory, Entity, Relation
  }` (`FromStr`, exact lowercase match); `RestoreReport { files: u64,
  records: usize }`; `RestoreError { TargetExists { path },
  StagingIo { path, source }, InstallIo { path, source },
  Verification(DurabilityError) }` (`Display`/`std::error::Error`
  impls); `restore(backup_dir: &Path, target_stem: &Path, domain:
  Domain) -> Result<RestoreReport, RestoreError>` — the whole pattern:
  refuse if any file already matches `target_stem`'s own prefix in its
  parent directory; stage every `backup_dir` entry into a fresh,
  process-unique temporary directory (PID plus an invocation counter,
  collision-free even across concurrent in-process calls, as the test
  suite's parallel test threads exercise); on full staging success,
  `create_dir_all` the target parent and `std::fs::rename` each staged
  file into place one at a time; verify via
  `open_{memory,entity,relation}_production_stack_portable(target_stem)`
  and count records via `AllIds::all_ids().len()`.
- `examples/restore_backup.rs` (new): `#[path = "support/
  restore_backup_lib.rs"]`-includes the file above; `main()` parses
  three CLI arguments (`backup_dir`, `target_stem`, `domain`), calls
  `restore`, prints a one-line summary plus the `SERVER_DATA_DIR`/
  restart instruction on success, the error on failure, `exit(1)` on
  any usage or runtime error.
- `tests/restore_backup.rs` (new, 10 tests): `#[path = "../examples/
  support/restore_backup_lib.rs"]`-includes the same file, so the test
  exercises the exact code the CLI runs. Five offline tests need no
  Cargo feature (domain parsing, target-prefix refusal before reading
  a missing backup, missing-backup cleanup, empty-backup verification
  failure for all three domains, a target with no file name). Five
  `#[cfg(feature = "server")]` tests drive a real `Request::Backup`
  over a real socket via `SchemaDrivenClient::backup` for each domain,
  then independently reopen the restored directory and assert every
  original record's every field and relation/`mentions` edge matches
  exactly — including a sibling-table-coexistence case (Entity
  restored beside an unrelated `memories.mmap` file, left untouched)
  and a real staging-copy-failure case (an uncopyable directory entry
  inside the backup) proving no target file and no leftover temporary
  directory remain.
- `Cargo.toml`: `[[example]] name = "restore_backup"`, `[[test]] name
  = "restore_backup"` — neither needs `required-features`
  (`crate::generic` is unconditionally compiled).

## Data/state and invariants

- The tool reads the backup directory and writes only under the
  target stem's parent directory plus one process-unique temporary
  directory beside it — it never touches `SERVER_BACKUP_ROOT` itself,
  which stays exactly what the prior `Request::Backup` produced.
- No new on-disk format, no new file, no new state persisted anywhere
  beyond what a restored table's own files already are.

## Errors, failure, recovery, and observability

- A target-prefix conflict: `RestoreError::TargetExists`, refused
  before any copy.
- A staging failure (an unreadable or uncopyable backup entry): no
  real target name is ever touched; the temporary directory is removed
  on a best-effort basis.
- A final-rename/directory failure after staging succeeded:
  `RestoreError::InstallIo`; some final files may already be in place
  — the target-prefix refusal on a subsequent run makes this safe to
  detect and retry after the operator clears what is there.
- A verification-reopen failure: `RestoreError::Verification` carries
  the domain's own real `DurabilityError`; copied files are retained,
  not deleted, for inspection.

## Security, privacy, and compatibility

No new attack surface: an offline, operator-run CLI, not
network-reachable, gated behind no feature. No wire/protocol change;
`PROTOCOL_VERSION` stays 26, `SERVER-002` untouched, `clients/python/`
untouched.

## Acceptance criteria

Numbered as in `docs/design/SERVER-RESTORE-DESIGN.md`'s own
"Acceptance criteria":

1. A real `Request::Backup`-produced `Memory` directory restores with
   every field and `mentions` edge intact, reopened via
   `open_memory_production_stack_portable`. ✔
2. The same for `Entity`/`Relation`, each against a real backup of
   that domain's own table, edges included. ✔
3. A second restore attempt against the same target refuses outright;
   no file is overwritten. ✔
4. A pre-staging failure leaves the real target names completely
   untouched and removes the temporary directory. ✔
5. A `backup_dir` whose contents do not form a valid stack for the
   named domain copies, then fails verification with a real
   `DurabilityError`; files are not deleted. ✔
6. `PROTOCOL_VERSION`/`SERVER-002`/`clients/python/` byte-for-byte
   unaffected — every existing test passes unmodified. ✔

## Verification plan

`cargo test -p rusty_multimodal_db --all-features --no-fail-fast`:
535 lib tests (unchanged — no new library code) + every integration
target green, `tests/restore_backup.rs` included (10/10, new — 5
offline, 5 live). 779 total, up from 769, 0 failed. `cargo fmt -p
rusty_multimodal_db -- --check` clean; `cargo clippy -p
rusty_multimodal_db --all-features --tests --bins --lib --example
restore_backup -- -D warnings` clean — `--example restore_backup`
added explicitly (`--tests --bins --lib` alone does not lint example
targets), the identical extension `STORAGE-019`'s own implementation
already used. Verified against a real fixture, not just the automated
suite: `cargo run --example restore_backup -- <backup_dir>
<target_stem> <domain>` against a genuine `server_backup_integration`-
produced backup restored the real record/file counts and printed the
`SERVER_DATA_DIR`/restart instruction; missing arguments, an
unrecognized domain, and a repeated target each exit 1 with a useful
message.

## Traceability

Implements: `ADR-0070` / `docs/design/SERVER-RESTORE-DESIGN.md`
(`RST-FR-001..006`). Resolves: `docs/FUTURE-GROWTH.md`'s Backup/restore
bullet's "any `RESTORE` request or documented restore procedure" line;
`ADR-0065`'s own "restore needs no new code" claim, now proven by a
real tool rather than left as an assertion.

No deviation from the design as accepted — every requirement landed
exactly as `docs/design/SERVER-RESTORE-DESIGN.md`'s "Proposed shape"
described.

## Open questions

None outstanding — every open question the design raised (`Dog`
support; a record-count diff; example vs. installed binary) is
resolved; see the design doc's own "Open questions — resolved during
implementation" section.
