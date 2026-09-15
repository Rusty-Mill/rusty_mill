# ADR-0070: Server Restore Tooling — a Real `restore_backup` CLI

- Status: **Accepted as designed and implemented** (2026-09-15) — the
  owner picked option (a) as recommended. See
  `docs/design/SERVER-RESTORE-DESIGN.md` for the full design.
- Date: 2026-09-15
- Deciders: baileyrd
- Related: `docs/FUTURE-GROWTH.md`'s "Operational maturity" section,
  Backup/restore bullet (names this exact gap: "Still absent: any
  `RESTORE` request or documented restore procedure"), `ADR-0065`
  (`Request::Backup`, protocol 24 — its own Decision text already
  asserts restore needs no new server code; this round tests and
  builds on that claim), `ADR-0066` (schema migration tooling — the
  exact "documented pattern plus one real, tested CLI" precedent tier
  this round follows), `ADR-0067` (`Request::FetchSnapshot` — the "no
  automated daemon shipped, an operator's own script does the rest"
  non-goal precedent this round also follows).
- Supersedes/Superseded by: none proposed. Completes (does not
  supersede) `ADR-0065`'s own left-open "restore needs no new code"
  claim with the operator-facing tool that actually exercises it.

## Context

`docs/FUTURE-GROWTH.md`'s Backup/restore bullet names the gap
directly: *"a backup is a portable, copy-safe directory ..., so
'restore' today means manually pointing a server's data directory at
the backup and restarting it, not a request or tool that does that for
you."* `ADR-0065` built `Request::Backup`/`Response::BackedUp`
(protocol 24) and its own Decision text already claims the restore
half needs no new code at all: *"the produced directory is exactly
what `open`/`open_portable` already read."* That claim has never been
exercised by a real tool — an operator today has to know it, trust it,
and perform the copy/verify/restart sequence by hand, every time.

**Why this is not a wire operation.** Every `ConnectionStore` this
crate ships is `Arc`-shared across every connection thread
`serve_tables` spawns, and every durable adapter holds an open `mmap`
over its own slot file for the process's whole lifetime. Swapping the
underlying files out from under a live `mmap` while other connections
may be reading or writing through it has no safe in-process answer —
and Windows specifically restricts overwriting a file that is actively
mapped. A `Request::Restore` could, at most, stage files for the
*next* start; the process still has to be restarted afterward for the
restore to take effect, which is the identical two-step "copy, then
restart" sequence a plain offline tool already performs, plus a new
authenticated write surface whose effect is invisible until that
restart happens.

## Decision

**Accepted and implemented: option (a)** — a new example CLI,
`examples/restore_backup.rs`, invoked `cargo run --example
restore_backup -- <backup_dir> <target_stem> <memory|entity|relation>`.
Copies every file from a `Request::Backup`-produced directory into a
fresh target directory: every file stages into one temp directory
first, and only once every file has staged successfully is each moved
individually into its real name via `std::fs::rename` (per-file
atomic, mirroring `STORAGE-014`'s own companion-file write discipline
— see the design doc's own named limitation on group atomicity),
refusing outright if any file already exists at the target (the same
"an existing target is refused, never silently overwritten" posture
`handle_backup` already has). Verifies the result by actually
domain's own `open_{memory,entity,relation}_production_stack_portable`
— a real production code path, not a byte-count proxy — and prints the
exact next step (point a server binary's `SERVER_DATA_DIR` at the
restored directory and restart it). Zero new `src/` library code, zero
new dependency, zero wire/protocol change — the identical tier
`ADR-0066`'s schema migration tooling already established and the
owner already accepted for an adjacent "offline, directory-level
maintenance operation" question.

Full reasoning, the two alternatives (a wire `Request::Restore`; declining
again), every requirement, and every acceptance criterion are in
`docs/design/SERVER-RESTORE-DESIGN.md`.

## Consequences

- Positive: closes `docs/FUTURE-GROWTH.md`'s named gap with a real,
  runnable, tested tool rather than leaving `ADR-0065`'s own "restore
  needs no new code" claim as an untested assertion — the exact
  correctness proof `ADR-0066`'s own worked migration example already
  established as this crate's bar for "a documented pattern" versus
  "a proven one."
- Positive: zero new `src/` library surface, zero new dependency, zero
  wire/protocol version bump — this round is entirely additive and
  entirely below the network layer, identical in shape to `ADR-0066`.
- Named, not hidden: this is a local, operator-run tool, not a
  networked or automated one — no scheduling, no daemon, no wire
  reachability. An operator (or their own external automation) invokes
  it by hand, once per restore, matching `ADR-0067`'s own explicit
  "no replica-refresh daemon shipped" non-goal.
- Named, not hidden: restore is a two-step "copy, then restart"
  operation, not a live one — the tool's own output says so rather
  than leaving that implicit, and Context above records exactly why a
  hot, in-process restore is not architecturally available for this
  crate's mmap-backed server model.
- This is a bounded, additive operator tool, sized like `ADR-0066`
  itself, not `ADR-0010`'s original protocol-defining tier — the class
  of decision `WORKFLOW.md` requires design-first, owner-accepted
  before implementation; the owner accepted option (a) before this
  implementation round.

## Considered options

**(a) A real CLI, `examples/restore_backup.rs`, as scoped above** —
recommended; **(b)** a `Request::Restore { name }` wire operation,
gated like `Backup`/`Compact` — cannot actually take effect without a
subsequent process restart either, so it gains nothing over (a) except
wire reachability, at the cost of a new authenticated write surface
whose effect is invisible until an unrelated future restart and a
`PROTOCOL_VERSION`/`SERVER-002` bump; **(c)** decline again, the
manual copy-and-restart procedure stays the whole story, `ADR-0065`'s
own "restore needs no new code" claim stays untested.

The owner's shorthand: **(a)** the CLI, as proposed; **(b)** a wire
`Request::Restore` instead; **(c)** decline.

## Acceptance and implementation

- 2026-09-15: proposed, design only.
- 2026-09-15: the owner picked option (a) — the `restore_backup` CLI,
  as recommended. Implementation delegated to Codex (`codex-build`),
  independently inspected by Claude before merge.
- 2026-09-15: implemented in the frozen Codex work-order round;
  advisory handoff for independent review, with no commit, push, or
  publication. Files: `examples/restore_backup.rs`,
  `examples/support/restore_backup_lib.rs`, `tests/restore_backup.rs`,
  `Cargo.toml`, `docs/design/SERVER-RESTORE-DESIGN.md`, and
  `docs/decisions/ADR-0070-server-restore.md`.
  - `RST-FR-001`–`006`: the thin example parses exactly three
    positional arguments, stages every backup file, renames each file
    separately, verifies through the selected public portable opener,
    and prints the file count, real `AllIds` record count, target stem,
    `SERVER_DATA_DIR`, and restart instruction. Existing target-prefix
    entries are refused before copying; failed verification retains
    the copied files and carries the original `DurabilityError`.
  - Implementation details: an additional `InstallIo` variant names
    final-rename/directory failures separately from staging failures.
    Temporary names use PID plus an invocation counter and exclusive
    `create_dir`; a stale name is skipped for a fresh one rather than
    deleting a pre-existing directory. Per-file atomicity is unchanged.
    The test target has no `required-features`; its five live-socket
    tests use `#[cfg(feature = "server")]`, while five offline tests
    also compile and run without a Cargo feature. The example needs
    no feature, and no dependency, lockfile, library, existing test,
    protocol, specification, or Python-client file was changed.
  - `tests/restore_backup.rs`: **10/10 passed**. Three domain round
    trips seed real production stores, start live servers with a
    `SERVER_BACKUP_ROOT`-equivalent option, request `Backup` through
    `SchemaDrivenClient`, and independently reopen the restored files.
    Whole-record `PartialEq` checks every field of every original
    record (including Unicode, ordered string lists, sync fields, and
    soft-delete timestamps); Memory mentions and Entity relation
    edges are also checked. Each round trip retries and proves refusal
    with unchanged target bytes and a fresh record reopen; source
    backup bytes remain unchanged. Entity covers sibling-table
    coexistence; Relation covers a nested, initially missing target.
    Two more live fixtures prove staging-copy failure removes staging
    and preserves the target, and wrong-domain verification carries
    the real schema-tag `DurabilityError` while retaining every copied
    file. Five offline tests cover exact domain parsing, companion-only
    prefix refusal before reading a missing backup, missing-backup
    cleanup, empty backups for all three domains, and a target without
    a filename.
  - Proof from the monorepo root on Windows, all exit codes **0**:
    `cargo fmt -p rusty_multimodal_db -- --check`;
    `cargo clippy -p rusty_multimodal_db --all-features --tests --bins --lib --example restore_backup -- -D warnings`;
    `cargo test -p rusty_multimodal_db --all-features --no-fail-fast`.
    Baseline: **769 passed**; final: **779 passed**; both **0 failed,
    0 ignored**. Library **535 → 535**, integrations **227 → 237**,
    binary tests **4 → 4**, doctests **3 → 3**. Every existing target's
    count is unchanged (table below). Cargo emitted existing ignored
    non-root-profile notices for five workspace manifests; Clippy
    emitted no lint diagnostics. Bundled DuckDB compiled successfully
    for both test and Clippy builds; no proof command was narrowed.
  - Real by-hand verification: `cargo run -p rusty_multimodal_db
    --example restore_backup -- <real-backup>/nightly
    <fresh-target>/memories.mmap memory`, with no Cargo features,
    restored **4 files, 2 records** from a baseline
    `server_backup_integration` wire-produced fixture and printed the
    actual target stem, `SERVER_DATA_DIR`, and restart instruction.
    A separate `CARGO_TARGET_DIR` under this checkout's `target/`
    avoided Clippy's build lock; no source configuration changed.
    The compiled CLI also returned **1** for missing arguments,
    case-mismatched `Memory`, and a repeated target, printing useful
    usage/refusal errors. Repeated against a nested fresh destination
    after the final source edit, again **4 files, 2 records**, exit **0**.

### Exact proof counts

The baseline command discovered targets before the new files were added;
every pre-existing source and integration test stayed unmodified. Counts
are passing tests, with zero failures or ignored tests in every row. The
new target did not exist in the baseline.

| Target | Before | After |
| --- | ---: | ---: |
| `src/lib.rs` | 535 | 535 |
| `src/bin/crash_safety_harness.rs` | 0 | 0 |
| `src/bin/crash_writer.rs` | 0 | 0 |
| `src/bin/dog_server.rs` | 4 | 4 |
| `src/bin/entity_server.rs` | 0 | 0 |
| `src/bin/memory_server.rs` | 0 | 0 |
| `src/bin/multiprocess_harness.rs` | 0 | 0 |
| `src/bin/multiprocess_writer.rs` | 0 | 0 |
| `src/bin/reminder_server.rs` | 0 | 0 |
| `tests/cross_backend.rs` | 7 | 7 |
| `tests/generic_production_integration.rs` | 1 | 1 |
| `tests/mmap_record_identity_keying.rs` | 4 | 4 |
| `tests/production_integration.rs` | 1 | 1 |
| `tests/restore_backup.rs` | — | 10 |
| `tests/schema_migration.rs` | 3 | 3 |
| `tests/server_auth_integration.rs` | 14 | 14 |
| `tests/server_backup_integration.rs` | 5 | 5 |
| `tests/server_client_only.rs` | 2 | 2 |
| `tests/server_dog_integration.rs` | 13 | 13 |
| `tests/server_employee_integration.rs` | 5 | 5 |
| `tests/server_entity_integration.rs` | 19 | 19 |
| `tests/server_hub_differential.rs` | 7 | 7 |
| `tests/server_memory_integration.rs` | 13 | 13 |
| `tests/server_metrics_http_integration.rs` | 5 | 5 |
| `tests/server_metrics_integration.rs` | 3 | 3 |
| `tests/server_order_integration.rs` | 3 | 3 |
| `tests/server_protocol_version.rs` | 18 | 18 |
| `tests/server_python_client.rs` | 1 | 1 |
| `tests/server_reminder_integration.rs` | 11 | 11 |
| `tests/server_replication_integration.rs` | 5 | 5 |
| `tests/server_schema_driven_client.rs` | 7 | 7 |
| `tests/server_sql_integration.rs` | 40 | 40 |
| `tests/server_tls_integration.rs` | 18 | 18 |
| `tests/server_transaction_integration.rs` | 22 | 22 |
| `doctests` | 3 | 3 |
