# ADR-0065: Server-Side Backup — `Request::Backup`, Path-Confined to an Opt-In Server-Local Root

- Status: **Accepted as designed and implemented** (2026-09-14 — the
  owner picked option (a): in-process, lock-consistent, path-confined
  to an opt-in server-local root). Proposed and implemented in the
  same session.
- Date: 2026-09-14
- Deciders: baileyrd
- Related: `docs/design/SERVER-BACKUP-DESIGN.md` (the full design),
  `docs/FUTURE-GROWTH.md`'s "Operational maturity" section (this
  round's source), `ADR-0052` (`Request::Compact` — the "operator's
  request, under the table's write lock" precedent reused directly),
  `STORAGE-014`/`015`/`016` (the write-to-temp-then-rename discipline
  every file this round writes reuses), `ADR-0053`
  (`SERVER_DATA_DIR` — the opt-in, env-var-named-directory precedent
  `SERVER_BACKUP_ROOT` mirrors).
- Supersedes/Superseded by: none. Additive: `Request::Backup` (new
  variant), `Response::BackedUp` (new variant), `PROTOCOL_VERSION`
  bump, an opt-in `SERVER_BACKUP_ROOT`-shaped configuration, an
  additive `backup_source` field on each production-capable
  `ConnectionStore` adapter. No existing file format, request, or
  client behavior changes.

## Context

`docs/FUTURE-GROWTH.md`'s new "Operational maturity" section names the
gap directly: the portable store files (`STORAGE-014`–`016`) are
copy-safe when the server is stopped — a cold copy is exactly what
`open_portable` already reconstructs from — but there is no live-backup
story. The files are mutated in place under `with_exclusive` while a
server runs; an external copy tool started by a different process has
no way to observe or wait for that lock, so a copy racing a write can
capture a torn file. Nothing in this crate today closes that.

Two things make this bounded rather than a from-scratch build:

1. **`Request::Compact` (`ADR-0052`) already establishes the exact
   shape of "operator's request that touches a table's files under its
   write lock."** Reusing that lock for a backup copy — not a new lock,
   not a new coordination primitive — gives the live-consistency
   guarantee for free.
2. **`STORAGE-014`'s write-to-temp-then-rename discipline already gives
   per-file crash safety.** A backup copy is, file by file, the
   identical operation `create`/`open` already perform when writing a
   companion blob — reused, not reinvented.

What does **not** already exist, and is the real new work this round
does: no adapter today retains the directory it was opened from
(`GenericProductionStore<S>` is `RwLock<S>` alone; the path is known
only by the binary that called `open`/`create`), and no request today
lets a caller's input choose a server-local filesystem write target —
every existing write (`Insert` through `WriteBatch`) writes to paths
the adapter already knew, never caller-supplied. Both gaps need a real
design, not just a request.

## Decision

Propose a new `Request::Backup { name: String }` answered
`Response::BackedUp { files: u64, bytes: u64 }`, gated as a write
(`Unauthorized` for `ReadOnly`, `SessionOpen` while a session is open,
matching `Compact` exactly). Handled inside the adapter's existing
`with_exclusive` critical section: every file the table's stack owns is
copied, write-to-temp-then-rename per file, into
`backup_root.join(name)` — `backup_root` a new, opt-in, server-
configured directory (`SERVER_BACKUP_ROOT`, mirroring `SERVER_DATA_DIR`'s
own shape); `name` restricted to a single path component (no `/`, `\`,
`..`), checked before any I/O, so a client's input can never name a
path outside the configured root. Unset `backup_root`: every `Backup`
request answers `Unsupported`, server-wide — no new filesystem-write
surface exists at all unless an operator opts in. Restore needs no new
code: the produced directory is exactly what `open`/`open_portable`
already read.

The fork, held for the owner:

- **(a) As scoped above — in-process, lock-consistent, path-confined
  to an opt-in server-local root (recommended).** The only option
  giving a genuinely live, consistent backup with no external
  coordination. Real, named cost: a brief stop-the-world pause for the
  whole table (`Compact`'s own already-accepted cost), and a new class
  of attack surface — a network client choosing where the server
  writes files — closed by the path-confinement design, not left
  implicit.
- **(b) Decline; document a manual cold-backup procedure only** (stop
  the server, copy the directory, restart). Zero code, zero new attack
  surface, but answers only "backup with downtime." Worth checking
  against real consumer need before committing to (a)'s implementation
  cost — `rusty_remind_me` has never asked for zero-downtime backup as
  of this round.
- **(c) A wire-level streaming backup** (bytes returned directly to the
  client, no server-local write target). Removes the path-confinement
  problem but replaces it with a strictly worse one — any authenticated
  write client could pull an entire table's bytes on demand, a new
  bulk-exfiltration primitive — and needs new framing for a
  potentially large multi-file payload. Declined in the design
  document; listed here for completeness.

## Consequences

- Positive (a): the only option that actually closes the "live backup"
  half of the named gap, not just the "backup exists as a concept"
  half. Reuses every crash-safety and locking primitive this crate
  already has — no new coordination mechanism, no new file format.
- Named, not hidden: this is the **first request whose input chooses a
  server-local filesystem write target.** Every prior write this crate
  has shipped writes to paths the adapter already knew. This is a real
  category of new risk (path traversal, writing outside the intended
  directory) and the design's `BAK-FR-004` closes it by construction
  (a single-component name joined to a server-configured root, never a
  caller-supplied absolute path or `..`) rather than by a runtime
  canonicalize-and-compare check that could itself have edge cases
  (symlinks). If the owner is not comfortable with this class of
  surface existing at all regardless of mitigation, option (b) or (c)
  in this ADR are the alternatives to weigh, not a reason to skip the
  mitigation in (a).
- Named, not hidden: a table-wide stop-the-world pause for the
  duration of the copy — the same cost `ADR-0052` already accepted for
  compaction, "milliseconds at this crate's target scale," but real
  and worth restating since backup, unlike compaction, may be called
  on a schedule by an operator rather than only reactively.
- This is a **wire, append-only, hard-to-reverse-once-shipped** change
  — `PROTOCOL_VERSION` bump, new variants, a new opt-in server
  configuration surface. Exactly the class of decision `WORKFLOW.md`
  requires design-first, owner-accepted before implementation.

## Acceptance and implementation

- 2026-09-14: proposed, design only.
- 2026-09-14: the owner picked option (a). Implemented on the same
  branch: `src/server/protocol.rs` (`Request::Backup { name: String }`/
  `Response::BackedUp { files, bytes }`, `PROTOCOL_VERSION` 23 → 24,
  the round after `ADR-0064`'s `Metrics` in the same PR); `src/server/
  serve.rs` (`ConnectionStore::backup` trait method, default
  `Unsupported`; `BackupReport`; `copy_table_files`; `handle_backup`;
  `ServeOptions::{with_backup_root, backup_root}`); `src/server/
  {dog,memory,entity,relation}.rs` (`backup_source: Option<PathBuf>`,
  `with_backup_source`, `backup()`); `src/bin/memory_server.rs`
  (`SERVER_BACKUP_ROOT`, wired to all three tables when
  `SERVER_DATA_DIR` is also set); `src/server/client.rs`
  (`SchemaDrivenClient::backup(name)`). `SERVER-001` v0.54.0 / FR-064.
- **Two real corrections, found during implementation, before writing
  the code the Decision above sketched — both narrowing scope, neither
  changing the security posture:**
  1. **`BAK-FR-002`'s premise was wrong.** The Decision assumed each
     adapter would need a new mechanism to *learn* its own data
     directory, since `GenericProductionStore<S>` retains no path.
     True at that layer — but every *leaf* store already does:
     `SlotFile::path()` (`src/generic/slot_file.rs`) and
     `MmapAgeStore`'s own `path` field are both already there, just
     with no public accessor and (for `MmapAgeStore`) marked dead code.
     A per-layer `Backup` trait walking the stack to reach them (the
     `Compact` precedent's own shape) was considered and rejected: it
     would need a correct, hand-enumerated file list at *every* layer
     (`GenericMmapStore`'s slot file, `.records`, `.inserts`;
     `MultiSymmetric`'s per-label `.edges` and `.relations` manifest;
     each edge file's own `.inserts`) — real duplication of knowledge,
     and silently incomplete the day a future round adds one more
     companion file. Implemented instead as a single, adapter-level
     `backup_source: Option<PathBuf>` (still real, additive, threaded
     from the binary's already-known directory — `BAK-FR-002` as
     originally scoped) paired with one shared, prefix-glob copy
     (`copy_table_files`): every file in the base path's directory
     whose name starts with the base path's own file name is copied,
     unconditionally — correct by construction against every companion
     file this crate has today or adds later, with no per-layer
     enumeration to keep in sync.
  2. **`Dog` is mechanically backup-capable but no current binary
     exercises it.** `DogConnectionStore<S>::backup` is implemented
     (`S: TransactionalStore` already gives the exclusive-lock access
     needed) and covered by the design's own reasoning — backup only
     ever copies existing files, unlike `Insert`/`Replace`/`Delete`/
     `Compact`, which `Dog` has never supported at all. But
     `dog_server.rs` has no `SERVER_DATA_DIR`-equivalent: it always
     seeds a fresh per-process scratch directory, matching this
     round's own Non-goal ("backing up a non-durable server"). So
     `SERVER_BACKUP_ROOT` is wired into `memory_server.rs` only (the
     one binary with real `SERVER_DATA_DIR` durability, for all three
     of `Memory`/`Entity`/`Relation`) — the library capability exists
     for a future durable `Dog` binary; no shipped binary calls
     `DogConnectionStore::with_backup_source` today.
  3. **The write-to-temp-then-rename discipline named in the Decision
     was simplified to a whole-directory atomic rename.** Per-file
     temp-then-rename (the Decision's own sketch, mirroring
     `STORAGE-014`) still leaves a *partially populated* target
     directory visible mid-copy under a hard process kill. Implemented
     instead: every file is copied into a fresh, process-unique
     temporary directory under the backup root, and only a fully
     successful copy is followed by one `std::fs::rename` onto the
     real name — simpler than per-file temp-then-rename, and a
     strictly stronger guarantee (the target name never exists in a
     partial state, not even between two files of the same backup).
     An *existing* target — empty or not — is refused outright rather
     than risked against `std::fs::rename`'s platform-specific
     "destination already exists" behavior (POSIX and Windows disagree
     on replacing a directory), narrowing `BAK-FR-005`'s "non-empty"
     wording to "exists at all."
- Proven: `tests/server_backup_integration.rs` (5 tests, real socket,
  `Memory` domain): a full backup reopened via
  `open_memory_production_stack_portable` with the identical record
  set (the flagship correctness proof); every path-traversal shape
  (`../escape`, `a/b`, `a\b`, `..`, `.`) refused `Malformed` with the
  backup root left untouched; a repeat name refused `Storage`; no
  configured root answering every `Backup` `Unsupported`; a `ReadOnly`
  token refused `Unauthorized`. `cargo fmt -p rusty_multimodal_db --
  --check` clean; `cargo clippy -p rusty_multimodal_db --all-features
  -- -D warnings` clean; `cargo test -p rusty_multimodal_db
  --all-features --no-fail-fast` 527 lib tests + every integration
  target green except the pre-existing, unrelated
  `server_python_client` failure (`python3` missing from this
  session's `PATH`).
