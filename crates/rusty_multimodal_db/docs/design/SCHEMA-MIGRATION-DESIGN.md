# Schema Migration Tooling: Reading an Old-Tagged Directory Forward, Proven on `Memory@1 → Memory@2` (Accepted, Implemented)

- Status: **Accepted as designed, implemented on the same branch**
  (2026-09-14, `ADR-0066`, option (a)). See `ADR-0066`'s own
  "Acceptance and implementation" section for the full implementation
  record.
- Related: `ADR-0019`/`docs/design/BLOB-SCHEMA-TAG-DESIGN.md` (the
  `SchemaTag` mechanism this round exercises, not changes), `ADR-0056`
  (the only real schema-tag bump this crate has ever shipped, and the
  source of the "L2: a layout version with an in-place upgrade — the
  principled follow-up" language this round closes), `ADR-0052`
  (`Compact`, the crate's established "full-table-rewrite under a fresh
  path" precedent), `ADR-0065` (`Backup`, the crate's established
  "write to a fresh location, leave the original untouched, operator
  swaps the deployment path" operational contract this round reuses
  unchanged), `docs/FUTURE-GROWTH.md`'s "Schema migration tooling"
  bullet (the gap this round closes).

## Purpose and scope

`SchemaTag` (`ADR-0019`) makes a wrong-record-type companion file a
named, refused-by-construction error rather than a silent `bincode`
mis-decode: `src/generic/record_blob.rs`'s `parse_tagged_header` checks
an 8-byte FNV-1a64 hash of `SchemaTag::SCHEMA_TAG` at a fixed offset in
every companion file (the record blob, the edge blob, the insert log,
and the label manifest — all four share the same 28-byte tagged-header
layout and helper functions), and refuses distinctly, leaving every
file untouched, on any mismatch reached through `read_portable_records`/
`open_portable`/`read_portable_edges`. `Memory` is the one record type
that has ever bumped its tag across a real layout change
(`memory::Memory` → `memory::Memory@2`, `ADR-0056`, when
`deleted_at_unix_ms`/`node_id` were appended) — and its own `SCHEMA_TAG`
doc comment names the gap this round closes verbatim: *"There is no
upgrade; the remedy is a re-push from the consumer, and the day a
directory cannot be re-pushed is the trigger for a layout version with
an in-place upgrade (the design's option L2)."*

`docs/FUTURE-GROWTH.md`'s own accounting is the same finding, named
independently: *"no migration* runner*: no command that reads an
old-tag directory and rewrites it under the new one. Today a layout
change is a manual, per-deployment operation."*

This round's scope is narrow and concrete: **prove a real, runnable
migration exists for the one bump this crate has actually shipped, and
document the pattern so the next bump does not have to invent it from
scratch.** It does not propose a generic schema-diffing engine, a
`RESTORE`/`MIGRATE` wire request, or automatic detection of "this
directory needs migrating" — see Non-goals.

## Non-goals

- **A generic Old→New reflection/diffing engine.** Every past bump in
  this crate (there has been exactly one) changed field *sets*, not
  just types — `bincode` writes no field markers, so there is no
  mechanical way to infer what a byte range used to mean without a
  human-supplied mapping. `AGENTS.md`'s own "no speculative generality"
  rule and this crate's own "no abstraction before two real call sites"
  precedent (`GENERIC-SCHEMA-DESIGN`'s acceptance) both argue against
  building one now, with one real case to validate a shape against.
- **A wire `Request::Migrate`.** Architecturally incoherent given how
  this crate is built: a `SchemaTag` mismatch is refused *before* a
  store can open at all, and a live server process is compiled against
  exactly one record type per table — it cannot simultaneously hold a
  connection open under `Memory`'s old and new layouts to serve a
  network request. Migration has to run *before* a server using the new
  layout can open the directory at all, i.e. offline, against a path —
  the same reason `Compact`/`Backup` are in-process requests but this
  is not.
- **Automatic detection or a "does this directory need migrating"
  probe.** `open_portable` already answers that question today, exactly
  as designed (`ADR-0019`): a mismatched tag is a named, distinct
  refusal. A migration tool's caller already knows they are migrating
  — they are running it because a directory refused to open.
- **Restoring `Dog`/`Order`/`Employee`'s bespoke, non-generic stores.**
  None of them implement `SchemaTag` at all (only types persisted
  through the generic companion-blob machinery do) and none has ever
  changed its on-disk layout. Out of scope until one does.
- **A migration for every current type "just in case."** `entity`,
  `order_customer::Order`, `relation::Relation`, and `reminder::Reminder`
  have never bumped their tags. This round ships exactly one worked
  migration, for the one type and the one bump that actually happened.

## Context and terminology

Read from `src/generic/{traits,record_blob,edge_blob,insert_log,store,
mmap_store,memory}.rs` as they stand at `SERVER-001` v0.54.0:

- `SchemaTag` (`src/generic/traits.rs:43-45`): `pub trait SchemaTag {
  const SCHEMA_TAG: &'static str; }` — an opt-in, standalone trait, not
  a supertrait of `Record`.
- Four companion-file kinds share the tagged-header mechanism
  (`record_blob::{encode_tagged_image, parse_tagged_header, tag_hash}`,
  a 28-byte header: the shared 20-byte magic+version+fingerprint header
  followed by an 8-byte FNV-1a64 hash of the tag string): the record
  blob (`GENBLOB\0`, `<path>.records`), the edge blob (`GENEDGE\0`,
  `<path>.edges` or `<path>.<label>.edges` for a `MultiSymmetric`
  label), the insert/tombstone log (`GENINSL\0`, `<path>.inserts`, and
  one per edge file), and the label manifest (`GENLABL\0`,
  `<path>.relations`, a private module inside `store.rs`).
- Mismatch behavior bifurcates by call path. When the caller already
  holds correctly-typed records (`GenericMmapStore::open`,
  `MultiSymmetric::open`), a mismatch is silently **healed** — the
  companion is rewritten from the caller's truth, no error. When the
  caller has nothing but the path (`read_portable_records`,
  `open_portable`, `read_portable_edges`), a mismatch is a **hard,
  propagated `DurabilityError::RecordBlobUnreadable{path, cause}`**
  naming the expected tag string and both FNV hashes, and — confirmed
  by dedicated tests — the directory's files are left completely
  untouched, never deleted or recreated. Migration only ever needs the
  second path: the whole reason a migration is needed is that the
  caller has *only* an old-tagged directory, not a caller-supplied
  in-memory record set the new type could heal onto.
- The mmap slot file itself (`<path>` with no suffix) carries only a
  numeric `SCHEMA_VERSION` (format, not per-type tag) and stores only
  `[id, scannable value]` pairs — no other field data. Its check is
  unaffected by a schema-tag bump that adds/removes non-indexed,
  non-scanned fields, which is exactly `ADR-0056`'s case: `Memory`'s
  `CategoryField`/`AccessCountField` markers are unchanged across the
  `@1`→`@2` bump, so the slot file for an old-tagged directory opens
  identically under either record type.
- `Memory`'s current 14-field layout (`src/generic/memory.rs:76-114`):
  `id`, `content`, `category`, `tags`, `source`, `metadata_json`,
  `created_at_unix_ms`, `updated_at_unix_ms`, `memory_type`, `status`,
  `sensitive`, `access_count`, then the two fields `ADR-0056` appended —
  `deleted_at_unix_ms: i64` (sentinel `0` = live) and `node_id: String`
  (sentinel `""` = unattributed). `impl SchemaTag for Memory { const
  SCHEMA_TAG = "memory::Memory@2"; }` (`memory.rs:131-133`).
- Since `ADR-0050`, `Memory`'s durable stack is not a bare
  `GenericMmapStore` — it is `Ordered<MultiSymmetric<GenericMmapStore<
  Memory, CategoryField, AccessCountField>, Memory>, Memory,
  UpdatedAtOrder>`: one foreign relation, `mentions` (memory → entity),
  stored via the edge blob/label manifest machinery above. `Ordered`
  adds no on-disk companion of its own — its sort index is rebuilt from
  the records at every open (`memory.rs:187-189`). This means the
  worked example below exercises the record blob *and* the edge blob
  and label manifest, not just the trivial no-relation case.
- **The pre-`ADR-0056` tag literal is reconstructed, not recovered.**
  This crate was `git subtree`-imported into the `rusty_mill` monorepo
  (`b64034738`), which squashed the standalone repo's history — `git
  log -S` for any string that ever appeared in `memory.rs` returns only
  that one import commit, so the exact pre-bump literal cannot be
  independently verified from this repository. Every other record type
  that has never bumped its tag follows one unbroken convention —
  `module::Type`, no suffix (`entity::Entity`, `order_customer::Order`,
  `relation::Relation`, `reminder::Reminder`) — and `ADR-0056`'s own
  text ("bump `Memory`'s schema tag *to* `memory::Memory@2`") is
  phrased as a bump *from* something, making `"memory::Memory"` the
  only candidate consistent with both facts. The worked example below
  is stated honestly as a synthetic, realistic reconstruction of
  `ADR-0056`'s exact field diff (not a recovered historical artifact —
  `ADR-0056` itself records that no deployment holds a layout-1
  directory today, so there is nothing to recover) — the mechanism it
  proves does not depend on the literal being byte-for-byte what some
  past process actually wrote.
- **`Compact` precedent** (`ADR-0052`): a real, proven "rewrite a whole
  table" primitive already exists and needs no locking discipline this
  round can't reuse conceptually — but `Compact` rewrites records *in
  place, under the same tag*, so it is a precedent for "rewrite from a
  record set," not "read under one type, write under another."
- **`Backup` precedent** (`ADR-0065`): the operational contract this
  round copies unchanged — write to a **fresh** target location, refuse
  outright if it already exists, leave the source completely untouched
  on any failure, and let the operator swap the deployment path
  manually afterward. Migration needs no locking at all beyond that: an
  old-tagged directory *cannot* be opened by a live server today (that
  is the refusal `ADR-0019` already guarantees), so there is no
  concurrent writer to protect against — a strictly simpler safety
  problem than `Backup`'s live-snapshot-under-a-write-lock case.

## Requirements

- `MIG-FR-001` **The pattern is three existing, public primitives
  composed, not new generic machinery.** (1) A caller-defined struct
  for the *old* layout, implementing `Record`/`SchemaTag` (with the old
  tag literal) and whatever `IndexedField`/`ScannableField` markers the
  type needs — reusing the current type's own marker structs where the
  indexed/scanned fields are unchanged, which is the common case for a
  field-*addition* bump. (2) The existing `open_..._portable`-shaped
  read path (mirroring `open_memory_production_stack_portable`'s own
  three-line body, typed over the old struct) to load every old record
  and every relation's edges, already merged with any pending insert
  log — `read_portable_records`/`open_portable` fold the insert log
  automatically (`INS-FR-004`), so a migration never misses or
  resurrects a record a live process wrote before its last compaction.
  (3) The existing `create_..._production_stack`-shaped write path
  (`memory.rs`'s own `create_memory_production_stack`, already `pub`,
  already the exact function `memory_server.rs` calls on first boot) to
  write a fresh, current-tagged directory from the converted records
  and unchanged edges.
- `MIG-FR-002` **A hand-written, per-migration conversion function.**
  `Old → New`, not a derived/generic mapping — every field that
  changed is named explicitly in one reviewable function, matching this
  crate's `AGENTS.md` "no speculative generality" rule and the
  precedent every other record-shape change in this crate has followed
  (each is its own ADR, never an automated schema diff).
- `MIG-FR-003` **Output goes to a fresh path, never the source.** The
  tool refuses outright if the destination path already exists (the
  same refusal `Backup`'s `ADR-0065` already established, reused
  unchanged) before writing anything. The source directory is never
  opened for writing, only for reading — a botched or interrupted run
  can corrupt at most the (fresh, disposable) destination, never the
  only copy of the data. This is a stronger guarantee than "in-place":
  `ADR-0056`'s own L2 language ("an in-place upgrade") describes the
  *operational* outcome (a stuck directory becomes usable again), not a
  requirement to mutate the original bytes.
- `MIG-FR-004` **Reuse `Compact`/`Backup`'s already-proven"write a
  fresh, complete directory from records+edges" code path exactly, not
  a new writer.** `create_memory_production_stack` already builds a
  full portable directory (mmap slot file, record blob, edge blob,
  label manifest, all correctly tagged for the type parameter it is
  instantiated with) from a `Vec<Memory>` and an edge list — nothing
  about that function needs to change for a migration to call it.
- `MIG-FR-005` **A real, runnable command, not a library function only
  reachable from a test.** `docs/FUTURE-GROWTH.md`'s own gap names "no
  command" specifically. Ships as `examples/migrate_memory_v1_to_v2.rs`
  (this crate's existing `cargo run --example` convention, already used
  by `examples/memory_footprint.rs`), taking the old and new paths as
  CLI arguments — a real operator can run it against a real directory,
  not just a hardcoded demo.
- `MIG-FR-006` **A regression test proving the whole pipeline against
  the crate's real, current production code path.** Not just "the
  conversion function is correct" in isolation — the test builds a
  synthetic `memory::Memory`-tagged (v1) directory, runs the migration,
  and reopens the result through the *actual*
  `open_memory_production_stack_portable` (the same function
  `memory_server.rs` calls on every restart) to prove the output is
  genuinely indistinguishable from a directory `Memory` code has always
  written — records, sentinel defaults, and `mentions` edges all
  checked.
- `MIG-FR-007` **No new dependency, no protocol/wire change, no new
  `src/` production module.** Every primitive this round calls already
  exists and is already `pub`. The new code is one `examples/` file
  (not compiled into the library or any consumer's dependency graph)
  plus one test.

## Considered options

- **(a) Documented pattern + one real, tested worked example —
  recommended.** As scoped above:
  `examples/migrate_memory_v1_to_v2.rs` plus this design doc as the
  durable "how to write the next one" reference. Zero new `src/`
  library code. Pro: matches "no abstraction before two real call
  sites" exactly — there has been exactly one bump, ever; the pattern
  is proven against that one real case rather than guessed at for a
  hypothetical second one. Con: the *next* migration still has to
  rewrite the old-struct-plus-glue boilerplate by hand; nothing is
  saved mechanically.
- **(b) A reusable generic library primitive**
  (`src/generic/migrate.rs`, e.g. `pub fn migrate_portable<Old, New>
  (old_path, new_path, convert: impl Fn(Old) -> New) -> Result<...>`)
  wrapping the same three-step pattern into one call. Pro: less
  ceremony for a second migration, if one ever looks like this one.
  Con: committing to a function signature/abstraction shape before a
  second real case exists to validate it against — the exact failure
  mode `GENERIC-SCHEMA-DESIGN`'s own "no abstraction before two real
  call sites" rule was written to prevent, and there is no guarantee
  the next bump is even shaped like "same `Id`, same index/scan
  fields, only new trailing fields" (this bump's happen to be); a field
  *removal*, an `Id` type change, or a relation-label rename would each
  need different glue than a `Fn(Old) -> New` closure over whole
  records can express uniformly.
- **(c) Decline / defer again.** Matches `ADR-0056`'s own original
  stance — still literally true today (no deployment holds a
  layout-1 `Memory` directory; nothing forces a migration to exist
  right now). Con: this is the second time this round would name the
  same gap and build nothing against it; `FUTURE-GROWTH.md`'s own
  accounting already says this plainly, so declining again adds no new
  information — and the *next* bump (of any type, not necessarily
  `Memory` again) would face the exact same "invent it under pressure,
  for real deployed data, with no proven precedent" position `ADR-0056`
  itself flagged as the risk.

The owner's shorthand: **(a)** pattern + worked example as proposed;
**(b)** (a) plus a generic `migrate_portable` helper now; **(c)**
decline, no code this round.

## Proposed shape

`examples/migrate_memory_v1_to_v2.rs` (new; no `required-features` —
`crate::generic::memory` is front-door, compiled unconditionally, the
same reason `memory_server.rs` needs only `server`, not `research`, to
build):

- `struct MemoryV1 { id, content, category, tags, source,
  metadata_json, created_at_unix_ms, updated_at_unix_ms, memory_type,
  status, sensitive, access_count }` — `Memory`'s current 12 leading
  fields verbatim, `#[derive(Serialize, Deserialize)]`.
- `impl Record for MemoryV1 { type Id = Uuid; fn id(&self) -> Uuid {
  self.id } }`.
- `impl SchemaTag for MemoryV1 { const SCHEMA_TAG: &'static str =
  "memory::Memory"; }` — the reconstructed pre-`ADR-0056` literal (see
  Context above).
- `impl IndexedField<memory::CategoryField> for MemoryV1` /
  `impl ScannableField<memory::AccessCountField> for MemoryV1` —
  reusing `Memory`'s own marker types directly (a marker is a
  zero-sized tag, not tied to one record type; nothing prevents a
  second type implementing the field trait against it), since the
  indexed/scanned fields are byte-for-byte unchanged across this bump.
- `fn open_memory_v1_stack_portable(path: &Path) -> Result<...,
  DurabilityError>` — `open_memory_production_stack_portable`'s own
  three-line body, typed over `MemoryV1` instead of `Memory`.
- `fn migrate_memory_v1_to_v2(old: MemoryV1) -> Memory` — every field
  copied by name, `deleted_at_unix_ms: 0` and `node_id: String::new()`
  appended (`ADR-0056`'s own documented sentinels for "live" and
  "unattributed").
- `fn main()`: reads `old_path`/`new_path` from `std::env::args()`;
  refuses (clear message, nonzero exit) if `new_path` already exists
  (`MIG-FR-003`); opens the old stack via
  `open_memory_v1_stack_portable`; collects every record via
  `AllIds::all_ids()` + `GetById::get` (both already implemented,
  forwarded through `MultiSymmetric`/`Ordered`); collects every
  `mentions` edge via `AllIds::all_ids()` +
  `MultiNeighbors::neighbors_by_relation("mentions", id)` per memory id
  (memory ids only ever appear on the "near" side of this foreign
  relation, so no dedup is needed — the far side is `Entity` ids, never
  enumerated by `MemoryV1::all_ids()`); converts every record via
  `migrate_memory_v1_to_v2`; calls the existing, unmodified
  `create_memory_production_stack(converted, &edges, new_path)`;
  prints a one-line summary (record/edge counts) on success.

No change to `src/generic/memory.rs`, `record_blob.rs`, `edge_blob.rs`,
`insert_log.rs`, or `store.rs`'s label manifest — every primitive the
example calls already exists and is already `pub`.

## Data/state and invariants

- The source directory is opened read-only in effect (only
  `read`-shaped calls: `open_portable`'s own read path, `all_ids`,
  `get`, `neighbors_by_relation` — nothing in the example ever calls
  `insert`/`replace`/`delete`/`compact` against the old stack). No
  write, temp file, or rename ever touches `old_path`.
- The destination is written exactly once, by the existing
  `create_memory_production_stack`, which itself already uses
  `GenericRecordBlob`/`EdgeBlob`/`label_manifest`'s crash-safe
  write-to-temp-then-rename per file (`STORAGE-014`–`016`) — no new
  atomicity work needed; a crash mid-write leaves an incomplete
  destination directory the operator discards and reruns against
  (source untouched, per `MIG-FR-003`), matching `Compact`'s own
  documented crash-safety property.
- Record count and `mentions` edge count in equal the source's, exactly
  — the acceptance test asserts this directly, not just "some records
  survived."

## Errors, failure, recovery, and observability

- Destination already exists: refused before any file is written,
  clear message naming the path (mirrors `Backup`'s
  `ErrorCode`-equivalent refusal, though this is a CLI exit code, not a
  wire error — there is no protocol involved).
- Source directory unreadable, or its tag does not match `MemoryV1`'s
  (e.g. it is already a `@2` directory, or a foreign type entirely):
  the existing `DurabilityError::RecordBlobUnreadable{path, cause}`
  propagates unchanged, naming both hashes and the expected tag — the
  same message an operator would already see running any other
  `open_portable` call against the wrong directory, so no new error
  vocabulary is introduced.
- A write failure partway through `create_memory_production_stack`
  (disk full, permissions): propagates `DurabilityError` unchanged; the
  destination directory may be left partially populated (matching
  `create`'s own existing behavior — it is not itself an atomic
  whole-directory operation, unlike `Backup`'s deliberate
  whole-directory-rename design), but the source is never touched, so
  the remedy is always "delete the partial destination and rerun."
- No logging/audit trail beyond the CLI's own stdout summary — this is
  an offline, operator-run tool against a directory no live process has
  open, not a request `ADR-0029`/`0031`'s audit/access log would ever
  see.

## Security, privacy, and compatibility

- No new attack surface: this is not network-reachable code, gated
  behind no feature an unauthenticated peer can reach, and requires
  filesystem access to both paths the same way any other CLI tool in
  `examples/`/`src/bin/` does.
- No wire/protocol change — `PROTOCOL_VERSION` stays 24, `SERVER-002`
  is untouched. This round is entirely below the network layer.
- Forward-compatible by construction with the next bump of a
  *different* type: nothing here couples the pattern to `Memory`
  specifically except the one worked example: the next migration for
  `entity`/`relation`/`reminder`/`order_customer` (should one ever bump)
  writes its own `examples/migrate_<type>_v<n>_to_v<n+1>.rs` following
  the same three-step recipe, cited by this design doc as the reference
  shape.

## Acceptance criteria

1. `examples/migrate_memory_v1_to_v2.rs` compiles and runs with no
   feature flags beyond the crate's default build.
2. Given a synthetic `memory::Memory`-tagged (pre-`ADR-0056`) directory
   with N records (including at least one with `mentions` edges), the
   tool produces a fresh directory that `Memory`'s real, unmodified
   `open_memory_production_stack_portable` opens successfully.
3. Every migrated record's first 12 fields equal the source record's
   exactly; `deleted_at_unix_ms == 0` and `node_id == ""` on every
   migrated record (`ADR-0056`'s own documented sentinels).
4. Every `mentions` edge in the source directory is present, unchanged,
   in the migrated directory — proven by `MultiNeighbors::
   neighbors_by_relation` against the reopened result, not just a
   record count match.
5. Running the tool against a `new_path` that already exists refuses
   before writing anything — the (pre-populated, to make the check
   observable) destination is byte-for-byte unchanged afterward.
6. Running the tool against a directory already tagged `memory::
   Memory@2` (the current, non-`V1` type) is refused with the existing
   `DurabilityError::RecordBlobUnreadable` schema-tag-mismatch message
   — no silent no-op, no partial write.

## Verification plan

`cargo test --all-features` (a new integration test, or a `#[cfg(test)]`
module inside the example crate target, covering acceptance criteria
2–6 against real temp directories via `crate::test_support::
fresh_temp_dir`, the same helper every other portable-file test in this
crate already uses); `cargo run --example migrate_memory_v1_to_v2 --
<old> <new>` run once by hand against a hand-built fixture directory as
a real, not just automated, smoke test; `cargo fmt`/`cargo clippy
--all-features -- -D warnings` clean, per every prior round's own
checkpoint.

## Traceability

- Roadmap: `SCHEMA-MIGRATION-DESIGN` (this document, `Proposed`),
  `SCHEMA-MIGRATION` (implementation, not started).
- Spec registry: `STORAGE-019` (new row, this round).
- `docs/FUTURE-GROWTH.md`'s "Schema migration tooling" bullet updated
  once implemented, matching the "Partly built since this was written"
  treatment its `Backup`/`Metrics` siblings already received.
- `ADR-0056`'s own `Memory::SCHEMA_TAG` doc comment ("the trigger for a
  layout version with an in-place upgrade — the design's option L2")
  gets a dated addendum once implemented, naming this ADR as the
  answer, matching `ADR-0043`'s own addendum precedent from the
  `python3` round.

## Open questions

- **Resolved: the owner picked (a)**, not (b)'s generic helper — see
  `ADR-0066`'s own Decision/Acceptance record.
- **Resolved: `examples/` is the right home**, confirmed by
  implementation — with one refinement the original text didn't
  anticipate: the shared migration logic (`MemoryV1` and the `migrate`
  function) lives in `examples/support/migrate_memory_v1_to_v2_lib.rs`
  (a subdirectory, not a direct child of `examples/`) and is pulled
  into both the real CLI (`examples/migrate_memory_v1_to_v2.rs`) and
  the regression test (`tests/schema_migration.rs`) via `#[path]` —
  not published, not a new library module, but shared between the two
  Cargo targets that need it rather than duplicated. A direct child of
  `examples/` with no `fn main()` would have been auto-discovered by
  Cargo as its own (failing) example target; the subdirectory avoids
  that without needing `autoexamples = false` or any other Cargo.toml
  change to this crate's existing discovery behavior.

## Change history

- 2026-09-14: initial proposal, design only.
- 2026-09-14: the owner picked option (a); implemented on the same
  branch as `STORAGE-019` v0.1.0. No deviation from the design as
  proposed. See `ADR-0066`'s own "Acceptance and implementation"
  section for the full record.
