# STORAGE-019 — Schema migration tooling (a documented pattern, one real worked migration)

- Version: 0.1.0
- Status: Accepted, Implemented, Verified
- Owners: baileyrd
- Depends on: `ADR-0019`/`STORAGE-015`/`STORAGE-016` (the `SchemaTag`
  mechanism this unit exercises, unchanged), `ADR-0056` (`Memory`'s
  `@1 → @2` bump, the one real case this unit proves the pattern
  against), `ADR-0052`/`ADR-0065` (`Compact`/`Backup`, the "write a
  fresh directory from records+edges" and "refuse an existing target,
  leave the source untouched" precedents this unit reuses unchanged)
- Supersedes: none. Adds no new on-disk format, changes no existing
  one.

## Purpose and scope

`SchemaTag` (`ADR-0019`) refuses a wrong-record-type companion file by
name rather than mis-decoding it — real, but until this unit, an
operator whose directory was written under an old tag had no tool that
reads it and rewrites it under the current one; `Memory`'s own
`SCHEMA_TAG` doc comment named this gap explicitly as the trigger for
"a layout version with an in-place upgrade" once one is needed.

This unit closes it narrowly: a documented three-step pattern —
(1) a caller-defined struct for the old layout, implementing
`SchemaTag` under the old tag and reusing the current type's own
`IndexedField`/`ScannableField` markers where unchanged; (2) the
existing `open_..._portable`-shaped read path, typed over the old
struct; (3) the existing `create_..._production_stack`-shaped write
path, unmodified, for a fresh new-tagged directory — proven end to end
against `Memory`'s own `@1 → @2` bump, the one bump this crate has
ever shipped. No new `src/` library module, no new dependency, no
wire/protocol change.

## Non-goals

Full list and reasoning: `docs/design/SCHEMA-MIGRATION-DESIGN.md`'s
own Non-goals. In short: no generic Old→New diffing engine (one real
case, `AGENTS.md`'s "no speculative generality"); no wire
`Request::Migrate` (architecturally incoherent — a `SchemaTag`
mismatch is refused before a store can open at all, so migration must
run offline, before a server using the new layout can exist); no
automatic "needs migrating" detection (`open_portable` already answers
that, by refusing); no migration for `Dog`/`Order`/`Employee` (none
implement `SchemaTag`) or for any type that has never bumped its tag.

## Requirements

`MIG-FR-001`–`007`, as designed in
`docs/design/SCHEMA-MIGRATION-DESIGN.md`'s own "Requirements" section
— unchanged by implementation, no deviation:

- `MIG-FR-001` the three-primitive pattern, composed from existing
  `pub` API only.
- `MIG-FR-002` a hand-written, per-migration `Old → New` conversion
  function, every field named explicitly.
- `MIG-FR-003` output to a fresh path only, refused outright if it
  already exists, before any write.
- `MIG-FR-004` reuse of `create_memory_production_stack`, unmodified,
  for the write side.
- `MIG-FR-005` a real, CLI-runnable tool (`examples/
  migrate_memory_v1_to_v2.rs`), not just a library function.
- `MIG-FR-006` a regression test proving the whole pipeline against
  `Memory`'s actual, current, unmodified production code path.
- `MIG-FR-007` zero new dependency, zero wire/protocol change, zero
  new `src/` production module.

## Architecture and interfaces

- `examples/support/migrate_memory_v1_to_v2_lib.rs` (new; not a direct
  child of `examples/`, so Cargo's example autodiscovery does not also
  try to build this `main`-less file as its own example target):
  `MemoryV1` (`Memory`'s current twelve leading fields, `#[derive(...,
  Serialize, Deserialize)]`) with `Record`/`SchemaTag` (tag literal
  `"memory::Memory"`, reconstructed — see the design doc's Context)
  and `IndexedField<CategoryField>`/`ScannableField<AccessCountField>`
  impls reusing `Memory`'s own marker types; `open_memory_v1_stack_
  portable(path) -> Result<MultiSymmetric<GenericMmapStore<MemoryV1,
  CategoryField, AccessCountField>, MemoryV1>, DurabilityError>`
  (`open_memory_production_stack_portable`'s own body, typed over
  `MemoryV1`, no `Ordered` wrapper — migration needs only `AllIds`/
  `GetById`/`MultiNeighbors`); `migrate_memory_v1_to_v2(old: MemoryV1)
  -> Memory` (every field named, `deleted_at_unix_ms: 0`, `node_id:
  String::new()`); `MigrationReport { records, mentions_edges }`;
  `MigrateError { DestinationExists, Durability(DurabilityError) }`;
  `migrate(old_path, new_path) -> Result<MigrationReport,
  MigrateError>` — the whole pattern: refuse if `new_path` exists,
  read every `MemoryV1` record and `mentions` edge from `old_path` via
  `AllIds::all_ids` + `GetById::get` +
  `MultiNeighbors::neighbors_by_relation`, convert each record, call
  `create_memory_production_stack(converted, &edges, new_path)`
  unmodified.
- `examples/migrate_memory_v1_to_v2.rs` (new): `#[path]`-includes the
  file above; `main()` parses two CLI arguments (`old_path`,
  `new_path`), calls `migrate`, prints a one-line summary or an error,
  `ExitCode::SUCCESS`/`FAILURE`.
- `tests/schema_migration.rs` (new, 3 tests): `#[path]`-includes the
  same file, so the test exercises the exact code the CLI runs. Builds
  a synthetic `memory::Memory`-tagged fixture (via the same public
  primitives `create_memory_production_stack` itself uses, typed over
  `MemoryV1`), runs `migrate`, reopens the result through the real
  `open_memory_production_stack_portable`.
- `Cargo.toml`: `[[test]] name = "schema_migration"`, `[[example]]
  name = "migrate_memory_v1_to_v2"` — neither needs
  `required-features` (`crate::generic::memory` is front-door),
  registered explicitly anyway, matching this file's own convention of
  naming every test/example target.

## Data/state and invariants

- The source directory is never opened for writing — only `AllIds`/
  `GetById`/`MultiNeighbors` reads against a store `open_portable`
  already treats as read-then-reconcile, no write path ever called.
- The destination is written exactly once, by
  `create_memory_production_stack`, which already uses
  `STORAGE-014`–`016`'s crash-safe write-to-temp-then-rename per file;
  no new atomicity work needed.
- No locking: an old-tagged directory cannot be opened by a live
  server at all (`ADR-0019`'s own refusal), so there is no concurrent
  writer to protect against — a strictly simpler safety problem than
  `Backup`'s live-snapshot-under-a-write-lock case.

## Errors, failure, recovery, and observability

- `new_path` already exists: `MigrateError::DestinationExists`,
  refused before any write.
- `old_path` unreadable or wrong-tagged: the existing
  `DurabilityError::RecordBlobUnreadable{path, cause}` propagates
  unchanged, naming both the expected tag and both FNV hashes — the
  same message any other `open_portable` mismatch already reports.
- A write failure partway through `create_memory_production_stack`:
  propagates `DurabilityError` unchanged; the source is never touched,
  so the remedy is always "delete the partial destination and rerun."

## Security, privacy, and compatibility

No new attack surface: an offline, operator-run CLI tool, not
network-reachable, gated behind no feature. No wire/protocol change;
`PROTOCOL_VERSION` stays 24, `SERVER-002` untouched.

## Acceptance criteria

Numbered as in `docs/design/SCHEMA-MIGRATION-DESIGN.md`'s own
"Acceptance criteria":

1. `examples/migrate_memory_v1_to_v2.rs` compiles and runs with the
   crate's default build, no feature flags. ✔
2. A synthetic `memory::Memory`-tagged fixture migrates to a directory
   `open_memory_production_stack_portable` opens successfully. ✔
3. Every migrated record's first twelve fields equal the source
   exactly; `deleted_at_unix_ms == 0`, `node_id == ""`. ✔
4. Every `mentions` edge in the source is present, unchanged, in the
   migrated directory. ✔
5. An existing `new_path` refuses before any write, left byte-for-byte
   unchanged. ✔
6. A directory already tagged `memory::Memory@2` is refused with the
   existing schema-tag-mismatch `RecordBlobUnreadable`. ✔

## Verification plan

`cargo test -p rusty_multimodal_db --all-features --no-fail-fast`:
527 lib tests (unchanged — no new library code) + every integration
target green, `tests/schema_migration.rs` included (3/3, new).
`cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p
rusty_multimodal_db --all-features -- -D warnings` clean, including
the new targets explicitly (`--example migrate_memory_v1_to_v2 --test
schema_migration`, since this crate's own `clippy` checkpoint skips
`--all-targets` for a pre-existing, unrelated Linux-only bench).
Verified against a real fixture, not just the automated suite: `cargo
run --example migrate_memory_v1_to_v2` with no args prints the usage
message and exits 1; against a nonexistent `old_path` it reports the
existing `RecordBlobUnreadable` message and exits 1; against a real,
on-disk two-record `MemoryV1` fixture (one with two `mentions` edges),
it prints "migrated 2 record(s), 1 mentions edge(s)" and writes a
complete, real four-file `Memory@2` directory.

## Traceability

Implements: `ADR-0066` / `docs/design/SCHEMA-MIGRATION-DESIGN.md`
(`MIG-FR-001..007`). Resolves: `docs/FUTURE-GROWTH.md`'s "Schema
migration tooling" bullet; `Memory`'s own `SCHEMA_TAG` doc comment's
named L2 trigger (`ADR-0056`).

No deviation from the design as accepted — every requirement landed
exactly as `docs/design/SCHEMA-MIGRATION-DESIGN.md`'s "Proposed shape"
described.

## Open questions

None outstanding — both the design's own open questions (whether to
build option (b)'s generic helper now; whether `examples/` is the
right home) are resolved; see the design doc's own "Open questions"
section.
