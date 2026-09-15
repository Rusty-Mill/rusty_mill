# Server Restore Tooling: a Real `restore_backup` CLI Closing `ADR-0065`'s Own "Restore Needs No New Code" Claim (Accepted)

- Status: **Accepted, option (a)** (2026-09-15, `ADR-0070`) — the owner
  picked option (a): the `restore_backup` CLI, as recommended.
  Implementation delegated to Codex via `codex-build`, independently
  inspected by Claude before merge.
- Related: `docs/FUTURE-GROWTH.md`'s "Operational maturity" section,
  Backup/restore bullet ("Still absent: any `RESTORE` request or
  documented restore procedure — a backup is a portable, copy-safe
  directory ..., so 'restore' today means manually pointing a server's
  data directory at the backup and restarting it, not a request or
  tool that does that for you"), `ADR-0065`
  (`SERVER-BACKUP-DESIGN.md`, `Request::Backup`/`Response::BackedUp`,
  protocol 24 — its own Decision text already asserts *"Restore needs
  no new code: the produced directory is exactly what
  `open`/`open_portable` already read"* — this round tests that claim
  directly and builds the tool the gap actually names), `ADR-0066`
  (`SCHEMA-MIGRATION-DESIGN.md` — the exact "documented pattern plus
  one real, tested CLI, zero new `src/` library code, zero wire
  change" precedent tier this round follows), `ADR-0067`
  (`Request::FetchSnapshot` — the "no continuous/automated tooling
  shipped, an operator's own script does the rest" precedent this
  round's own Non-goals cites directly).

## Purpose and scope

`docs/FUTURE-GROWTH.md`'s Backup/restore bullet names the gap
directly: *"a backup is a portable, copy-safe directory ..., so
'restore' today means manually pointing a server's data directory at
the backup and restarting it, not a request or tool that does that for
you."* `ADR-0065` built `Request::Backup` and asserted, in its own
Decision text, that restore needs no new *server* code because the
produced directory already round-trips through `open`/`open_portable`
unmodified — but never built the operator-facing tool that actually
performs the copy, checks its own preconditions (an existing/non-empty
target refused, matching `handle_backup`'s own precedent), and reports
what happened. This round closes exactly that: a real CLI, not a new
wire operation, that copies a `SERVER_BACKUP_ROOT`-produced directory
into a fresh target directory an operator then points a server binary
at.

## Non-goals

- **A `Request::Restore` wire operation, or any change to
  `PROTOCOL_VERSION`/`SERVER-002`.** See Context for why a live,
  in-process restore is architecturally unsound for this crate's own
  mmap-backed, thread-per-connection server model — restore is
  fundamentally a directory-level, offline operation here, the same
  way `ADR-0066`'s own schema migration is. `clients/python/` needs no
  change either.
- **Automatic, unattended, or scheduled restore.** This is an
  operator-invoked CLI, run by hand (or by an operator's own external
  automation) exactly once per restore — matching `ADR-0067`'s own
  named non-goal for `FetchSnapshot` ("no replica-refresh daemon
  shipped by this crate — an operator's own script fetches, writes to
  local disk, and restarts").
- **Verifying or repairing a corrupt backup.** The tool copies files
  and then, per domain, attempts a real portable open as its own
  verification step — it does not attempt to recover a backup that
  itself fails to open; that failure is reported plainly and the
  restore is left in place for the operator to inspect, not silently
  discarded.
- **A generic, record-type-agnostic restore engine.** Verification
  needs to call the right domain's own `open_..._production_stack_
  portable` constructor — a `--domain` argument names which, the same
  per-migration specificity `ADR-0066` already chose over a reusable
  generic helper (its own declined option (b)).
- **Restoring `Dog`.** No shipped binary gives `Dog` a durable data
  directory today (`ADR-0065`'s own implementation note #2) — nothing
  to restore into until a future round changes that.

## Context

Read directly from `src/server/serve.rs` (`handle_backup`,
`copy_table_files`, `ConnectionStore::backup`), `src/bin/
memory_server.rs` (the only binary wiring `SERVER_DATA_DIR`/
`SERVER_BACKUP_ROOT`), `src/generic/{memory,entity,relation}.rs`
(`open_..._production_stack_portable`/`open_or_create_..._production_
stack`), `examples/migrate_memory_v1_to_v2.rs` plus `examples/support/
migrate_memory_v1_to_v2_lib.rs`, and `ADR-0065`/`ADR-0066`/`ADR-0067`
and their design docs, as they stand at `SERVER-001` v0.57.0:

- **`ADR-0065`'s own Decision text already states restore needs no new
  server-side code**, verbatim: *"the produced directory is exactly
  what `open`/`open_portable` already read."* Confirmed directly:
  `open_memory_production_stack_portable(path)`/
  `open_entity_production_stack_portable(path)`/
  `open_relation_production_stack_portable(path)` each take a plain
  `&Path` stem and reconstruct the full stack from whatever files sit
  beside it — the identical shape `handle_backup`'s own output
  directory already has. Nothing about a backup's own file format
  differs from a live data directory's; restore is copying files, not
  transforming them.
- **`memory_server.rs` is the one binary with real durability
  (`SERVER_DATA_DIR`) and the one binary `SERVER_BACKUP_ROOT` is wired
  into**, for all three of its tables. `<dir>/memories.mmap`,
  `<dir>/entities.mmap`, `<dir>/relations.mmap` are each opened via
  `open_or_create_{memory,entity,relation}_production_stack`, and each
  adapter's `with_backup_source(<dir>/<table>.mmap)` names the exact
  stem `copy_table_files` matches every companion file against. A
  `Request::Backup { name }` on any of the three produces
  `SERVER_BACKUP_ROOT/<name>/` holding exactly that table's own files,
  under their original names (`copy_table_files` preserves each
  matched file's own file name unchanged when copying it into the
  target directory) — a restore of that directory back into a fresh
  data directory needs no renaming, no per-file logic, just a whole-
  directory copy.
- **`handle_backup`'s own crash-safety shape is the template to adapt,
  not reinvent, with one real difference named up front**: it copies
  into a fresh, process-unique temporary directory first, then one
  atomic `std::fs::rename` swaps the *whole temporary directory* onto
  the real target name — legal because a backup's target is a brand
  new directory name nobody else uses. A restore's target is
  `target_stem`'s parent directory, which already exists (or is
  created once) and may already hold *other* tables' own files
  side by side — there is no single fresh name to swap onto as a
  group. `RST-FR-003` adapts the same "stage everything first, touch
  real names only after every file is ready" principle to that
  constraint: stage every file, then move each one into place
  individually (per-file atomic, not one whole-group swap) — see
  `RST-FR-003`'s own named limitation.
- **A live, in-process `Request::Restore` is architecturally unsound
  for this crate's own server model, not just unbuilt.** Every
  `ConnectionStore` this crate ships is `Arc`-shared across every
  connection thread `serve_tables` spawns, and every durable adapter
  (`GenericMmapStore`/`MmapAgeStore`) holds an open `mmap` over its own
  slot file for the lifetime of the process. Swapping the underlying
  files out from under a live `mmap` while other threads may be
  reading or writing through it has no safe answer inside this
  process — the OS permits overwriting a mapped file's bytes, but
  nothing coordinates that with concurrent readers/writers, and
  Windows in particular restricts overwriting a file that is actively
  mapped at all. A "restore" request that only stages files for the
  *next* start (not a hot swap) would still need the process restarted
  afterward to take effect — the exact two-step "copy, then restart"
  sequence this round's CLI already performs, plus a new authenticated
  wire-write surface whose visible effect is nothing until an operator
  restarts the process out-of-band. Restore is fundamentally an
  offline, directory-level operation for this architecture, the same
  conclusion `ADR-0066`'s own schema migration tooling already reached
  for an adjacent operation (`open_..._portable`'s own read path,
  never a live in-process rewrite).
- **`ADR-0066`'s own precedent is the exact tier and shape to match**:
  a real, runnable CLI (`examples/migrate_memory_v1_to_v2.rs`) built
  entirely from primitives that already exist and are already `pub`,
  zero new `src/` library code, zero new dependency, zero wire/
  protocol change — accepted by the owner over a generic in-library
  helper specifically to avoid speculative generality
  (`AGENTS.md`'s own rule) ahead of a second real case.

## Requirements

- `RST-FR-001` **A new `examples/support/restore_backup_lib.rs` (no
  `main`, matching `migrate_memory_v1_to_v2_lib.rs`'s own precedent
  exactly — the logic a test file can call directly without spawning
  a subprocess) plus a thin `examples/restore_backup.rs` CLI wrapper**
  that parses argv and calls it, invoked `cargo run --example
  restore_backup -- <backup_dir> <target_stem> <domain>` —
  `backup_dir` a `SERVER_BACKUP_ROOT/<name>` directory a prior
  `Request::Backup` produced, `target_stem` the `<dir>/<table>.mmap`-
  shaped path a server binary will later be pointed at (matching
  `SERVER_DATA_DIR`'s own per-table stem convention), `domain` one of
  `memory`/`entity`/`relation` (naming which portable-open constructor
  verifies the result). No new `src/` library module — every
  primitive this tool calls already exists and is already `pub`; the
  support file is included via `#[path]` by both the CLI and
  `tests/restore_backup.rs`, exactly as `tests/schema_migration.rs`
  already does for `migrate_memory_v1_to_v2_lib.rs`.
- `RST-FR-002` **Copies every file in `backup_dir` into `target_stem`'s
  parent directory, preserving each file's own name unchanged** — no
  per-file logic, no format transformation; a backup directory already
  holds exactly one table's own companion files, matching
  `RST-FR-001`'s domain argument.
- `RST-FR-003` **Crash-safe per file, staged before any real name is
  touched**: every file in `backup_dir` is first copied into one
  fresh, process-unique temporary directory beside `target_stem`'s
  parent (mirroring `handle_backup`'s own "stage everything, verify
  the whole set copied, only then touch real names" shape). Only once
  *every* file has staged successfully does the tool move each staged
  file into `target_stem`'s parent directory via `std::fs::rename`
  (one call per file — same-filesystem rename is atomic per file,
  matching `STORAGE-014`'s own companion-file write discipline).
  **Named limitation, not hidden**: unlike `handle_backup`'s own
  single-directory swap (the whole backup lands under one name in one
  rename), a restore's target is an *existing* directory potentially
  already holding sibling tables' own files, so there is no single
  name this tool can rename onto atomically as a group — the staging
  phase guarantees every file is fully readable and written before
  any real name changes, but a hard kill between two of the final
  per-file renames can still leave a *partial* set of real files
  behind. `RST-FR-004`'s refusal on any existing file makes that safe
  to detect and retry: a partially-restored target is visibly
  incomplete (some but not all of the domain's own files present),
  the operator removes what is there and reruns. A pre-copy failure
  (anything before the staging phase completes) leaves the real
  target names completely untouched, no partial set ever appears.
- `RST-FR-004` **Refuses to touch a target that already has any file
  matching `target_stem`'s own file-name prefix** — the identical
  "an existing target is refused outright, never silently overwritten"
  posture `handle_backup` already established for the backup
  direction, applied here to the restore direction: an operator who
  wants to retry removes the old files first, exactly as
  `handle_backup`'s own doc comment already documents for a repeated
  backup name.
- `RST-FR-005` **Verifies the restored directory by actually opening
  it**, via the domain argument's matching
  `open_{memory,entity,relation}_production_stack_portable(target_stem)`
  call — a real reopen through this crate's own production code path,
  not a byte-count or checksum proxy — and reports what it found (a
  record count, at minimum) on success, or the exact
  `DurabilityError` on failure, leaving the copied files in place
  either way for the operator to inspect.
- `RST-FR-006` **Prints the exact next step** on a successful restore:
  the `SERVER_DATA_DIR`-shaped directory and stem an operator now
  points a real server binary at, and that the binary must be
  (re)started for the restored data to take effect — restore is a
  two-step "copy, then restart" operation, not a live one, and the
  tool's own output says so rather than leaving that implicit.

## Considered options

- **(a) A real CLI, `examples/restore_backup.rs` plus
  `examples/support/restore_backup_lib.rs`, as scoped above —
  recommended.** Directly proves `ADR-0065`'s own "restore needs no
  new code" claim by actually exercising it, with the missing
  operator-facing piece (safe copy, existing-target refusal, real
  verification via reopen) built once rather than left as a manual,
  undocumented sequence of shell commands. Zero new `src/` library
  code (the support file lives under `examples/`, not `src/`, the
  same non-published-library placement `ADR-0066`'s own migration
  tooling already uses), zero new dependency, zero wire/protocol
  change — the identical tier `ADR-0066` already established and the
  owner already accepted for the adjacent "another offline,
  directory-level maintenance operation" question.
- **(b) A `Request::Restore { name }` wire operation**, gated like
  `Backup`/`Compact`, copying a `SERVER_BACKUP_ROOT`-relative backup
  into the live table's own `backup_source` directory under its write
  lock — but, per Context, this cannot actually take effect until the
  process restarts (a live `mmap` cannot be safely swapped underneath
  itself), so its real behavior is identical to the CLI's own "copy,
  then an operator restarts the process" sequence, plus a new
  authenticated write surface whose effect is invisible until that
  restart happens out-of-band, and a `PROTOCOL_VERSION`/`SERVER-002`
  bump for a capability that gains nothing over (a) except reachability
  over the wire instead of shell access to the server host. Real,
  named cost: an authenticated write client could stage a restore that
  silently corrupts a live directory the moment an unrelated future
  restart happens, with no way for that future restart to know a
  restore was ever staged unless this round also invents a "pending
  restore" marker file and start-time check — real new complexity for
  a shape (a) already covers.
- **(c) Decline** — leave the manual "copy the backup directory over
  the data directory yourself, restart" procedure exactly as it is
  today, documented in `docs/FUTURE-GROWTH.md`'s own bullet and
  nowhere else. Zero cost, zero risk. `ADR-0065`'s own "restore needs
  no new code" claim stays an assertion an operator has to trust
  rather than a tool that proves and performs it; a caller who wants
  the existing-target/crash-safety/verification properties (a) gives
  has to reimplement them by hand, every time, exactly the situation
  `ADR-0066` already declined to leave in place for schema migration.

The owner's shorthand: **(a)** the real CLI, as proposed; **(b)** a
wire `Request::Restore` instead; **(c)** decline, the manual procedure
stays the whole story.

## Proposed shape

`examples/support/restore_backup_lib.rs` (new, no `main` — mirroring
`migrate_memory_v1_to_v2_lib.rs`'s own shape exactly) exposes the
testable orchestration, called by both the thin CLI and the test file:

```rust
pub enum Domain { Memory, Entity, Relation }

pub struct RestoreReport { pub files: u64 /* , … */ }

#[derive(Debug)]
pub enum RestoreError { /* existing target, staging I/O failure,
                           reopen failure (carries the DurabilityError) */ }

// RST-FR-002/003: stage every backup_dir file into one temp
// directory first; only once every file has staged successfully,
// rename each staged file individually into target_stem's parent
// (per-file atomic, not one whole-group atomic swap — see
// RST-FR-003's own named limitation).
// RST-FR-004: refuse outright if any file already exists at
// target_stem's own prefix, before any copy begins.
// RST-FR-005: open_{memory,entity,relation}_production_stack_portable
//   (target_stem) — a real reopen, reported to the operator.
pub fn restore(
    backup_dir: &std::path::Path,
    target_stem: &std::path::Path,
    domain: Domain,
) -> Result<RestoreReport, RestoreError> {
    // …
}
```

`examples/restore_backup.rs` (new; thin — parses argv, maps `domain`'s
string to `Domain`, calls `restore`, and — `RST-FR-006` — prints the
exact next step on `Ok`, or the error on `Err`, then
`std::process::exit(1)`):

```rust
#[path = "support/restore_backup_lib.rs"]
mod restore_backup;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, backup_dir, target_stem, domain] = args.as_slice() else {
        eprintln!("usage: restore_backup <backup_dir> <target_stem> <memory|entity|relation>");
        std::process::exit(1);
    };
    // parse `domain`, call `restore_backup::restore(..)`, report the
    // result (RST-FR-006 on success; the error, verbatim, on failure).
}
```

## Data/state and invariants

- The tool reads `backup_dir` and writes only under `target_stem`'s
  parent directory (plus one process-unique temporary directory beside
  it, per `RST-FR-003`) — it never touches `SERVER_BACKUP_ROOT` itself,
  which stays exactly what a prior `Request::Backup` produced,
  restorable again if this run is aborted or its result discarded.
- No new on-disk format, no new file, no new state persisted anywhere
  beyond what a restored table's own files already are — the identical
  `memories.mmap`/`.records`/`.mentions.edges`/`.relations`-shaped set
  every live `Memory` table already has.

## Errors, failure, recovery, and observability

- A missing or unreadable `backup_dir`: reported plainly, process
  exits non-zero, no partial write attempted.
- An existing file already at `target_stem`'s own prefix: refused
  before any copy, `RST-FR-004`, process exits non-zero naming the
  conflicting path.
- A pre-staging failure (anything before every file has copied into
  the temporary directory): the temporary directory is removed, the
  real target names are left completely untouched.
- A hard kill between two of the final per-file renames: a partial set
  of real files at the target — a named, not hidden, limitation
  (`RST-FR-003`); `RST-FR-004`'s refusal on any existing file makes
  this safe to detect (some but not all expected files present) and
  retry after the operator removes what is there.
- A restored directory that fails to reopen (`RST-FR-005`): the exact
  `DurabilityError` is printed, the copied files are left in place
  (not deleted) for the operator to inspect directly, process exits
  non-zero.

## Security, privacy, and compatibility

- **No new network-reachable surface at all** — this is a local CLI an
  operator with filesystem access to the backup/target directories
  already runs by hand; it adds nothing an operator with that access
  could not already do with `cp`/`robocopy` plus manually restarting
  the server, it only makes the safe, verified version of that
  sequence the easy one to reach for.
- **No wire, protocol, or `SERVER-002` change of any kind** — option
  (a) touches nothing a client ever sees.
- Wire, hard-to-reverse-once-shipped in the sense of "a real, relied-
  upon operational tool now exists," but not a `PROTOCOL_VERSION`
  change at all — sized like `ADR-0066` itself (a bounded, additive
  operator tool), not `ADR-0010`'s original protocol-defining tier.

## Acceptance criteria

1. `cargo run --example restore_backup -- <backup_dir> <target_stem>
   memory` against a real `Request::Backup`-produced directory
   restores a `Memory` table whose contents, reopened via
   `open_memory_production_stack_portable`, match the original table's
   records field-for-field, `mentions` edges included.
2. The same for `entity`/`relation` domains, each against a real
   backup of that domain's own table.
3. Running the tool a second time against the same `target_stem`
   refuses outright (`RST-FR-004`) — no file at the target is
   overwritten, silently or otherwise.
4. A pre-staging failure (simulated: an unreadable file inside
   `backup_dir`, or an injected I/O error during the staging copy)
   leaves the real target names completely untouched — no file ever
   becomes visible under a real name unless every file staged
   successfully first.
5. A `backup_dir` whose contents do not actually form a valid
   `{memory,entity,relation}` stack (e.g., an empty directory, or a
   different domain's files) is copied, then the verification reopen
   fails with a real `DurabilityError`, reported plainly — files are
   not silently deleted.
6. `PROTOCOL_VERSION`/`SERVER-002`/`clients/python/` are byte-for-byte
   unaffected — every existing test passes unmodified, and no wire
   fixture needs regeneration.

## Verification plan

`cargo test -p rusty_multimodal_db --all-features --no-fail-fast` (a
new `tests/restore_backup.rs` proving acceptance criteria 1–5 against
real, on-disk fixtures — a real `Request::Backup` over a live socket
first, then the tool's own logic invoked directly as a library call
the test can assert against, mirroring `ADR-0066`'s own
`tests/schema_migration.rs` shape); a real, by-hand `cargo run
--example restore_backup` invocation against a genuine backup
directory, output inspected directly (not just the automated suite),
matching `ADR-0066`'s own "verified against a real fixture" bar;
`cargo fmt -p rusty_multimodal_db -- --check`/`cargo clippy -p
rusty_multimodal_db --all-features --tests --bins --lib --example
restore_backup -- -D warnings` clean — `--example restore_backup`
added explicitly (`--tests --bins --lib` alone does not lint example
targets), the identical extension `ADR-0066`'s own implementation
already used for `migrate_memory_v1_to_v2`.

## Traceability

- Roadmap: `SERVER-RESTORE-DESIGN` (this document, `Proposed`),
  `SERVER-RESTORE` (implementation, not started).
- `docs/FUTURE-GROWTH.md`'s Backup/restore bullet updated once
  implemented — "any `RESTORE` request or documented restore
  procedure" moves from "still absent" to named, bounded, and built,
  the identical treatment `ADR-0064`/`ADR-0065`/`ADR-0069` each already
  received on this same page.

## Open questions

- **Should the tool also accept `Dog`, for the future binary
  `ADR-0065`'s own implementation note #2 names as not-yet-shipped?**
  Recommendation: no — building restore support for a domain with no
  durable binary today is speculative generality ahead of a real call
  site, the same reasoning `ADR-0065` itself already used to leave
  `Dog`'s own `with_backup_source` uncalled by any shipped binary.
  Trivial to add the fourth match arm once a durable `Dog` binary
  exists.
- **Should a successful restore also print a diff against the backup's
  own record count if one was already visible** (e.g., from a prior
  `Request::Metrics` or `Backup`'s own reported file/byte counts)?
  Recommendation: no — the verification reopen's own record count is
  the real proof; a diff against a number the operator would have to
  have separately recorded is a nice-to-have with no clear consumer,
  left for a future round if a real operator asks for one.
- **Should `restore_backup` be a real installed binary
  (`src/bin/restore_backup.rs`) rather than a `cargo run --example`
  target?** Recommendation: an example, matching
  `migrate_memory_v1_to_v2`'s own precedent exactly — this crate's own
  established convention for an operator tool that is not one of the
  four production server binaries. Left for the implementation to
  resolve if a real objection surfaces — a mechanical detail, not a
  design fork the owner needs to weigh.
