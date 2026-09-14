# Server-Side Backup: `Request::Backup`, a Consistent Snapshot Copy Under the Table's Write Lock (Accepted)

- Status: **Accepted as designed, implemented with three corrections**
  (2026-09-14, `ADR-0065`, option (a)). See `ADR-0065`'s own
  "Acceptance and implementation" for the full account: a single
  prefix-glob copy replaces this document's per-layer file
  enumeration, `Dog`'s capability ships unused by any current binary,
  and the target directory is produced via one whole-directory atomic
  rename rather than this document's per-file temp-then-rename sketch.
- Date: 2026-09-14
- Related: `docs/FUTURE-GROWTH.md`'s "Operational maturity" section
  (this round's own source: "Backup/restore as a real, named
  operation... the closest thing to a backup story today [is] copy-
  safe [portable files]... but there is no `BACKUP`/`RESTORE` request,
  no documented procedure, and no tooling around 'copy while the
  server is live'"), `ADR-0052` (`Request::Compact` — the "operator's
  request, under the table's write lock" precedent this round reuses
  directly), `STORAGE-014`/`015`/`016` (the write-to-temp-then-rename
  crash-safety discipline every file this round writes reuses),
  `ADR-0053` (`SERVER_DATA_DIR` — the env-var-named-durable-directory
  precedent `SERVER_BACKUP_ROOT` below mirrors).
- Supersedes/Superseded by: none. Additive: one new `Request`/`Response`
  variant pair, one new opt-in `ServeOptions`/binary-level directory
  configuration, one new method on each production-capable
  `ConnectionStore` adapter. No file-format change to any existing
  file; a backup directory is exactly what `open`/`open_portable`
  already read.

## Purpose and scope

The portable-store file formats (`STORAGE-014`–`016`) already make a
*cold* copy (server stopped) trivially safe — the files are exactly
what `open_portable` reads. What is missing is a *live* one: the files
are mutated in place under `with_exclusive` while the server runs, so
an external `cp`/`rsync` started by another process has no way to
coordinate with an in-flight write and can observe a torn file — a real
gap `FUTURE-GROWTH.md` now names explicitly rather than leaving
implicit.

Scope, exactly: one new gated write request that performs the copy
*inside* the server process, under the same critical section `Compact`
already uses, so no external coordination is needed
(`BAK-FR-001`/`006`); a real path-confinement design, because this is
the first request that lets an authenticated client choose where the
server writes files (`BAK-FR-003`/`004`); the per-adapter plumbing
needed to know each table's own directory, which does not exist today
(`BAK-FR-002`); explicitly, restore needs no new code at all
(`BAK-FR-007`).

## Non-goals

- **Restore as a request.** A backup directory is a complete, portable
  store directory the moment this round produces it — pointing
  `SERVER_DATA_DIR` (or the domain's equivalent) at it and starting a
  server *is* restore, using machinery that has existed since
  `STORAGE-014`. No `Request::Restore` is proposed.
- **Scheduled/automatic backup.** Operator-triggered only, the
  identical stance `ADR-0052` already took for compaction ("no
  threshold, no timer, no background thread — an operator's request").
- **Incremental/differential backup.** Every call is a full copy. A
  incremental scheme would need a real design of its own (what "changed
  since" means against an mmap file rewritten in place) and is not
  named as needed by any current consumer.
- **Encrypting the backup at rest.** Whatever filesystem-level
  protection an operator's `SERVER_BACKUP_ROOT` already has applies;
  this round adds no new encryption layer.
- **`Order`/`Employee`.** `research`-gated reference material with no
  production binary or durable-directory story; they answer
  `Unsupported`, matching `Compact`'s own precedent for domains without
  the capability.
- **Backing up a non-durable (scratch-mode) server.** A binary run
  without its data-directory environment variable set (e.g.
  `memory_server` with no `SERVER_DATA_DIR`) has no directory to copy
  *from*; `Unsupported`.

## Context and terminology

Read from `src/server/{memory,journal,mod}.rs`, `src/generic/{mmap_store,production}.rs`,
`src/bin/memory_server.rs` as they stand at `SERVER-001` v0.52.0:

- `GenericProductionStore<S>` (`src/generic/production.rs`) is
  `RwLock<S>` and nothing else — it does not retain the path(s) it was
  opened from. Paths are known only at the call site that invoked
  `create`/`open`/`open_or_create_*` — today, that is each binary
  (`memory_server.rs`'s `open_stores`, reading `SERVER_DATA_DIR`), not
  the adapter (`MemoryConnectionStore` et al.) or the store type
  itself. **This is the one real structural gap this round has to
  close**: an adapter cannot back itself up without being told, at
  least once, where its own files live.
- `Compact` (`ADR-0052`) is the closest existing precedent for "an
  operator's request that touches files under the table's write lock,"
  but it takes **no path argument at all** — it rewrites files it
  already knows the paths of (via each store layer's own internal
  path-deriving helpers, e.g. `blob_path(path)`), in place. `Backup` is
  a different shape of request: the **target** is caller-supplied. That
  is new attack surface a compaction request never had to consider —
  see "Security, privacy, and compatibility" below.
- `SERVER_DATA_DIR` (`ADR-0053`) is the existing precedent for "one
  environment variable naming a durable directory, opened or created."
  `SERVER_BACKUP_ROOT` below is the same shape of variable, for a
  different purpose (a confinement boundary, not a store to open).
- `record_blob.rs`/`edge_blob.rs`'s write-to-temp-then-rename discipline
  (`STORAGE-014`, reused by `STORAGE-015`/`016`) is exactly the
  per-file crash-safety primitive a backup copy needs: a target file is
  either absent or the complete new bytes, never partial, regardless of
  when a crash or I/O error interrupts it.

## Requirements

- `BAK-FR-001` **`Request::Backup { name: String }` /
  `Response::BackedUp { files: u64, bytes: u64 }`.** A new request at
  the next protocol version after `ADR-0064`'s `Metrics` (24,
  provisional — see "Open questions" on sequencing). Gated exactly as
  `Compact`: `Unauthorized` for a `ReadOnly` token, `SessionOpen`
  while a session is open, `Malformed` below the new protocol version.
- `BAK-FR-002` **Each production-capable adapter learns its own
  directory.** `MemoryConnectionStore`/`EntityConnectionStore`/
  `RelationConnectionStore`/`DogConnectionStore` gain an additive
  `backup_source: Option<PathBuf>` (or equivalent), set at
  construction from the same directory the binary already passed to
  `open_or_create_*`/`create`/`open` — a real, bounded structural
  addition (a threaded parameter, not a redesign), not a behavior
  change to any existing method. An adapter built without it (scratch
  mode, or a future adapter that forgets to wire it) answers
  `Unsupported`, never a panic or a wrong path guessed.
- `BAK-FR-003` **`SERVER_BACKUP_ROOT`, opt-in, mirroring
  `SERVER_DATA_DIR`'s shape.** `ServeOptions` (or the binary directly,
  matching `SERVER_DATA_DIR`'s own binary-level precedent rather than
  `ServeOptions`, since this is per-binary deployment topology, not a
  security policy) gains an optional backup-root directory. Unset:
  every `Request::Backup` answers `Unsupported`, server-wide — the
  identical "opt-in capability, closed by default" shape
  `SERVER_TXN_JOURNAL_PATH` already established for journaling.
- `BAK-FR-004` **Path confinement, checked before any I/O.** `name`
  must be a single path component: no `/`, `\`, `..`, and non-empty —
  refused `Malformed` otherwise, before the target path is even
  constructed. The real target directory is always
  `backup_root.join(name)`, so a client can never name a path outside
  the configured root, and never an absolute path overriding it. **This
  is the one genuinely new security consideration this round
  introduces** — see "Security, privacy, and compatibility."
- `BAK-FR-005` **Refuse to overwrite.** If `backup_root.join(name)`
  already exists and is non-empty, refused with `ErrorCode::Storage`
  (matching `Compact`'s own "Storage if a file could not be rewritten"
  precedent for a files-layer refusal) before any copy begins — a
  backup call never silently clobbers a prior one.
- `BAK-FR-006` **The copy itself, under the table's write lock.**
  Inside the same `with_exclusive` critical section `Compact` already
  uses (so a concurrent writer can never observe, or race, a partial
  backup): create the target directory; copy every file the adapter's
  stack owns (the slot file, the companion record blob, the insert
  log, and — for a relation-bearing table — the edge log/blob) into
  it, each via write-to-temp-then-rename (`STORAGE-014`'s exact
  discipline, reused, not reinvented); on any I/O failure, remove the
  partial target directory entirely and answer `Storage` — the target
  is always either wholly absent or a complete, `open_portable`-
  reopenable directory, never a partial one an operator could
  mistake for a good backup.
- `BAK-FR-007` **Restore is documentation, not code.** The produced
  directory is byte-for-byte what `create`/`open` already write and
  `open`/`open_portable` already read — no new request, no new store
  method. Recorded as an operational procedure in this crate's docs
  (point a fresh process's `SERVER_DATA_DIR`, or the domain-specific
  equivalent, at the backup directory).

## Considered options

- **(a) As scoped above — an in-process `Request::Backup` under the
  table's write lock, path-confined to an opt-in server-local root
  (recommended).** The only option that gives a *live*, consistent
  backup with no external coordination needed, reusing every
  crash-safety primitive this crate already has (`STORAGE-014`'s
  write-then-rename, `Compact`'s locking precedent). Its cost is real
  and named: a brief stop-the-world pause for the whole table (the
  same cost `Compact` already accepted, `ADR-0052`'s own "milliseconds
  at this crate's target scale" finding), and one new class of
  attack surface (a network client choosing a server-local write
  target) that needs the path-confinement design above to close.
- **(b) Decline; document a manual cold-backup procedure only** (stop
  the server, copy the directory, restart) — zero code, real, but
  answers only "backup with downtime," not "backup while live," which
  is the actual gap named. A legitimate answer if no consumer
  (`rusty_remind_me` or otherwise) has ever asked for zero-downtime
  backup — worth checking before committing to (a)'s real
  implementation cost.
- **(c) A wire-level streaming backup** — the client receives the raw
  bytes directly over the connection rather than the server writing
  them to a server-local directory. Removes the path-confinement
  problem entirely (no server-local write target to protect) but
  trades it for a strictly larger one: *every* authenticated write
  client could pull the *entire* table's bytes on demand, a new bulk-
  exfiltration primitive this crate has never had, and needs new
  framing to stream a potentially large multi-file payload over the
  existing length-prefixed, whole-frame protocol. Declined — worse on
  the security axis this round exists to get right, not better.

## Proposed shape

`src/server/protocol.rs`: `Request::Backup { name: String }`,
`Response::BackedUp { files: u64, bytes: u64 }`, `PROTOCOL_VERSION`
bump, golden vectors. `src/server/{memory,entity,relation,dog}.rs`:
`backup_source: Option<PathBuf>` field, `backup(&self, target: &Path)
-> Result<BackupReport, BackupError>` per adapter (mirrors `compact`'s
own per-adapter method shape). `src/server/serve.rs`: `handle_connection`
gains one arm calling the adapter's `backup` after `BAK-FR-004`/`005`'s
checks; `ServeOptions`/binary gains `backup_root`. Both clients gain
`backup(name) -> Result<(u64, u64), ClientError>`.

## Data/state and invariants

- A target directory this round produces, reopened via
  `open_portable`, reconstructs a store with the exact record/edge set
  the source table held at the instant the critical section ran — no
  weaker and no stronger a guarantee than what `Compact`'s own
  in-place rewrite already gives a reopen.
- `backup_root.join(name)` is always a descendant of `backup_root` —
  proved by construction (`BAK-FR-004`'s single-component check), not
  by runtime canonicalization-and-compare (simpler, and avoids a
  symlink-resolution edge case a canonicalize-based check would need
  to get right).
- A target directory is never observed by any external reader in a
  partial state — either it does not exist yet, or every file inside
  it is already the complete, renamed-into-place image.

## Errors, failure, recovery, and observability

`ErrorCode::Malformed` for an invalid `name` (empty, containing `/`,
`\`, or `..`) or a request below the negotiated protocol version.
`ErrorCode::Storage` for an existing non-empty target, or any I/O
failure during the copy (with the partial target removed first).
`ErrorCode::Unsupported` when `backup_root` is unset server-wide, or
for a domain/adapter with no durable directory. No new audit/access
log event kind is proposed — `Backup` is recorded by the existing
`AccessEvent`/`RequestKind` machinery exactly as every other request
already is (`ADR-0031`); a distinct audit line naming the target
directory could be added later if operators want it, not proposed
here.

## Security, privacy, and compatibility

This is the first request where an authenticated client's input
chooses a **filesystem path the server writes to**. Every other
write this crate has ever added (`Insert`/`Replace`/`Delete`/`Link`/
`Compact`/`WriteBatch`) writes to files the adapter already knows the
paths of, none of them caller-influenced. `BAK-FR-003`/`004` exist
specifically to close this: the server never honors an absolute path
or a `..` component from the wire, ever — the real target is always
computed server-side as `configured_root.join(single_path_component)`.
An operator who never sets `SERVER_BACKUP_ROOT` gets zero new
filesystem-write surface at all (`Unsupported`, unconditionally) — the
identical "opt-in, closed by default" posture `SERVER_TXN_JOURNAL_PATH`
already established. A pre-this-round client is unaffected; a
version-negotiated-below-24 client never sees `Request::Backup` exist.

## Acceptance criteria

1. `name` containing `/`, `\`, `..`, or empty is refused `Malformed`
   before any directory is created or touched — proven by a test that
   also asserts the filesystem is untouched (no stray directory).
2. A `Request::Backup` against a table with real data, under
   concurrent readers, produces a target directory that a fresh
   `open_portable` reconstructs with the identical record/edge set —
   the flagship correctness proof.
3. A `Request::Backup` whose I/O fails partway (simulated: an
   unwritable target subpath) leaves no partial target directory
   behind — removed entirely, `Storage` returned.
4. A `Request::Backup` against an existing, non-empty target directory
   is refused `Storage` without touching its contents.
5. A server started with no `SERVER_BACKUP_ROOT` answers every
   `Request::Backup` `Unsupported`, proven by an integration test.
6. `cargo test --all-features` green; `cargo clippy --all-features --
   -D warnings` clean; `cargo fmt --all --check` clean.

## Verification plan

`cargo test --all-features` (per-adapter unit tests for `backup`;
`tests/server_backup_integration.rs`, a real socket test: seed data,
call `Backup`, reopen the target via `open_portable`, compare against
the source); a real I/O-failure injection test (a read-only subpath, or
an already-existing non-empty target) for the "no partial directory
left behind" property; `cargo clippy --all-features -- -D warnings`;
`cargo fmt --all --check`.

## Traceability

- Roadmap: `SERVER-BACKUP-DESIGN` (this document), `SERVER-BACKUP`
  (implementation, `Implemented`).
- Decision: `ADR-0065`.
- Specification: `SERVER-001` v0.54.0 / FR-064.
- Requirements: `BAK-FR-001`–`007`.

## Open questions

- **Sequencing against `ADR-0064` (`Request::Metrics`).** Both rounds
  are proposed together; whichever implements first claims protocol
  version 23, the other 24. Not decided here.
- **Is live (zero-downtime) backup actually wanted** by any real
  consumer, or does option (b)'s documented cold-backup procedure
  already satisfy every deployment this crate has today? Worth
  checking before committing to (a)'s real implementation cost — named
  here rather than assumed.
- **Should a distinct audit event name the backup target directory?**
  Left to a follow-on if an operator asks; today's `AccessEvent`
  already records that a `Backup` request happened and its outcome,
  just not the `name` argument (matching the audit log's existing
  "never a record id or value" privacy posture — whether a
  server-local directory *name* should be an exception is a real,
  separate question).

## Change history

- 2026-09-14: initial proposal, design only.
- 2026-09-14: the owner picked option (a). Implemented on the same
  branch — protocol 24, `SERVER-001` v0.54.0 / FR-064. See `ADR-0065`'s
  own "Acceptance and implementation" for the three real mechanism
  corrections (prefix-glob copy instead of per-layer enumeration, no
  binary wiring for `Dog` since no shipped binary gives it a durable
  directory, whole-directory atomic rename instead of per-file
  temp-then-rename) and why each changed from this document's own
  sketch.
