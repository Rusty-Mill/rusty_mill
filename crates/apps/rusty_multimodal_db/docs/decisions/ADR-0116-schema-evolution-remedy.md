# ADR-0116: Schema Evolution — What Exists, What the Refusal Now Says

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "1-5", item 2 — schema evolution). No wire change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0056` (the schema-tag bump, option L1, and its named
  follow-up L2), `ADR-0066` / `docs/design/SCHEMA-MIGRATION-DESIGN.md` /
  `STORAGE-019` (the migration tooling that answered L2's trigger),
  `examples/migrate_memory_v1_to_v2.rs` (the worked example),
  `docs/FUTURE-GROWTH.md` (the stale line this corrects).
- Supersedes/Superseded by: none. Additive: `memory_server`'s
  `open_error`.

## Context

The growth document still said a record layout "is unversioned:
adding a field is a schema-tag bump that refuses old directories
distinctly, with an in-place upgrade named as the round for the first
directory that cannot be re-pushed". That sentence predates
`ADR-0066`: the schema tag *is* the layout version (`memory::Memory@2`),
and the migration tooling exists — a three-step pattern (an old-layout
struct under the old tag, the existing portable open typed over it,
the existing create path for a fresh directory) with a real CLI for
`Memory`'s only layout change so far and a regression test that proves
the migrated directory reopens through production code with every
field and edge intact. A copy into a fresh directory is the stronger
guarantee "in-place" was reaching for: the source is never the only
copy.

What was missing was operational: a `memory_server` pointed at an
old-layout directory printed the raw `schema tag mismatch` and
stopped, naming no remedy. An operator had to know `ADR-0066` existed.

## Decision

- `SCE-FR-001` — `memory_server`'s startup error for a table that will
  not open names the table, and when the cause is a schema-tag
  mismatch says what the refusal means (an older layout, refused, never
  mis-read, never recreated) and the remedy: for `memory`, the
  migration command `cargo run -p rusty_multimodal_db --example
  migrate_memory_v1_to_v2 -- <old_path> <new_path>` then
  `SERVER_DATA_DIR` at the new path, or a re-push from the consumer;
  for `entity` and `relation`, which have had no layout change, a
  re-push, with `ADR-0066`'s pattern named for the day one is needed.
  Proven by spawning the real binary against a pre-`ADR-0056` fixture:
  it refuses before listening, names the tool, and leaves the
  directory untouched.
- `SCE-FR-002` — the growth document's schema line is corrected to
  what is built and what is still absent: no runtime `ALTER TABLE`,
  no tolerant decoding (`bincode` writes no field markers), one
  migration tool per layout change, written when the change happens.
- Not built, on evidence: a generic layout-version header or an
  upgrade-on-open. Every layout change so far is one, on one table,
  with one tool; a generic mechanism would be an abstraction with one
  call site.

## Consequences

- Positive: the refusal an operator meets is now a set of
  instructions; the growth document no longer promises a round that
  already happened.
- Negative / tradeoffs: the remedy text names the example by its
  current name; a renamed example must update it (the test pins it).
- Named, not hidden: `Entity` and `Relation` still have no migration
  tool because they have never needed one; `ADR-0066`'s pattern is
  the recipe.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.95.0 / `FR-108`, in one PR with `ADR-0115`, `ADR-0117`–`ADR-0119`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — «TESTS» tests across «TARGETS» targets, 0 failed. Builder: Claude.
