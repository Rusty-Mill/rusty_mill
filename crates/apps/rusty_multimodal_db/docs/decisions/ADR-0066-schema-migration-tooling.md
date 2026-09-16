# ADR-0066: Schema Migration Tooling — a Documented Pattern Plus One Real, Tested Migration

- Status: **Accepted as designed, implemented on the same branch**
  (2026-09-14 — the owner's pick, option (a): pattern plus one real,
  tested worked example; (b) a generic helper and (c) decline both
  declined). See `docs/design/SCHEMA-MIGRATION-DESIGN.md` for the full
  design.
- Date: 2026-09-14
- Deciders: baileyrd
- Related: `ADR-0019` (`SchemaTag`, the mechanism this round exercises
  unchanged), `ADR-0056` (`Memory@1 → Memory@2`, the only real bump
  this crate has shipped, and the source of the "L2… the day a
  directory cannot be re-pushed is the trigger" language this round
  answers), `ADR-0052` (`Compact`), `ADR-0065` (`Backup`, whose
  "write to a fresh path, refuse if it exists, leave the source
  untouched, operator swaps the deployment path" operational contract
  this round reuses unchanged), `docs/FUTURE-GROWTH.md`'s "Schema
  migration tooling" bullet (the gap this round closes).
- Supersedes/Superseded by: none proposed.

## Context

`docs/FUTURE-GROWTH.md`'s "Operational maturity" section (added
alongside `ADR-0064`/`ADR-0065`, updated once both shipped) names the
gap directly: *"`ADR-0056`'s schema-tag versioning refuses a directory
built under an older layout by name, distinctly — a real safety
property — but there is no migration **runner**: no command that reads
an old-tag directory and rewrites it under the new one."* `Memory`'s
own `SCHEMA_TAG` doc comment names the same gap from the inside,
verbatim: *"There is no upgrade; the remedy is a re-push from the
consumer, and the day a directory cannot be re-pushed is the trigger
for a layout version with an in-place upgrade (the design's option
L2)."* No deployment holds a layout-1 `Memory` directory today — this
round is not answering an active incident, it is building the proven
playbook before the next bump (of any type, not necessarily `Memory`
again) needs one under real pressure.

## Decision

**Recommended: option (a)** — a documented pattern plus one real,
tested migration, `examples/migrate_memory_v1_to_v2.rs`, built entirely
from primitives that already exist and are already `pub`
(`open_..._production_stack_portable`'s own shape, typed over a
caller-defined old-layout struct; `create_..._production_stack`,
unmodified, for the write side). Zero new `src/` library code, zero new
dependency, zero wire/protocol change. A hand-written `Old → New`
conversion function per migration, matching every other record-shape
change in this crate's own precedent (each is its own ADR, never an
automated schema diff) and `AGENTS.md`'s "no speculative generality"
rule.

Full reasoning, the two alternatives (a reusable generic
`migrate_portable<Old, New>` helper; declining again), and the
requirements/acceptance criteria are in
`docs/design/SCHEMA-MIGRATION-DESIGN.md`.

## Consequences

- Positive: closes `ADR-0056`'s own named-but-deferred gap with a real,
  runnable tool and a passing regression test proving it works against
  the crate's actual, current production code path — not just a
  promise for later.
- Positive: no new `src/` surface, no new dependency, no wire/protocol
  version bump — this round is entirely additive and entirely below
  the network layer.
- Named, not hidden: the pre-`ADR-0056` tag literal used in the worked
  example is a reconstruction (this crate's `git subtree` import
  squashed the standalone repo's history, so it cannot be independently
  verified) — grounded in `ADR-0056`'s own text and every other
  unbumped type's convention, not fabricated, and the mechanism proven
  does not depend on it being byte-for-byte a recovered historical
  artifact.
- Named, not hidden: the *next* migration (a different type, or a
  differently-shaped change to `Memory` again) still requires writing
  its own old-layout struct and conversion function by hand — nothing
  here is mechanically reusable across migrations. This is a deliberate
  scope choice (see Considered options), not an oversight.

## Considered options

**(a) Documented pattern + one real, tested worked example** —
recommended; **(b)** (a) plus a reusable generic `migrate_portable`
library helper now; **(c)** decline/defer again, no code this round.

The owner's shorthand: **(a)** as proposed; **(b)** (a) plus the
generic helper; **(c)** decline.

## Acceptance and implementation

- 2026-09-14: proposed, design only.
- 2026-09-14: the owner picked option (a).
- 2026-09-14: implemented as `STORAGE-019` v0.1.0 — `examples/support/
  migrate_memory_v1_to_v2_lib.rs` (new: `MemoryV1` and its `Record`/
  `SchemaTag`/`IndexedField`/`ScannableField` impls, reusing `Memory`'s
  own `CategoryField`/`AccessCountField` markers; `open_memory_v1_
  stack_portable`, `open_memory_production_stack_portable`'s own body
  typed over `MemoryV1`; `migrate_memory_v1_to_v2`, every field named
  explicitly; `migrate`, the whole three-step pattern), included via
  `#[path]` — not published, not a new library module — by both
  `examples/migrate_memory_v1_to_v2.rs` (the real CLI, `cargo run
  --example migrate_memory_v1_to_v2 -- <old_path> <new_path>`) and
  `tests/schema_migration.rs` (3 tests: the flagship round trip through
  `Memory`'s real, unmodified `open_memory_production_stack_portable`
  with every field and `mentions` edge checked; the existing-destination
  refusal; the already-`@2`-tagged-source refusal, the existing
  `RecordBlobUnreadable` schema-tag-mismatch message unchanged) — so
  both exercise the exact same code, not two hand-kept-in-sync copies.
  `examples/support/` (not a direct child of `examples/`) specifically
  so Cargo's example autodiscovery does not also try to build the
  `main`-less lib file as its own example target. Zero new `src/` file,
  zero new dependency, zero wire/protocol change, exactly as designed —
  no deviation from the accepted text. **Verified against a real
  fixture, not just the automated suite**: `cargo run --example
  migrate_memory_v1_to_v2` with no args prints the usage message and
  exits 1; with a nonexistent `old_path` it reports the existing
  `RecordBlobUnreadable` "cannot read file" message and exits 1; run
  against a real, on-disk two-record `MemoryV1` fixture (one with two
  `mentions` edges), it prints "migrated 2 record(s), 1 mentions
  edge(s)" and writes a complete, real four-file `Memory@2` directory
  (`memories.mmap`, `.records`, `.mentions.edges`, `.relations`).
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --all-features -- -D warnings` clean (including
  the new example/test targets explicitly, `--example
  migrate_memory_v1_to_v2 --test schema_migration`, since this crate's
  own `clippy` checkpoint skips `--all-targets` for a pre-existing,
  unrelated Linux-only bench); `cargo test -p rusty_multimodal_db
  --all-features --no-fail-fast`: 527 lib tests (unchanged — no new
  library code) + every integration target green, `schema_migration`
  included (3/3, new).
