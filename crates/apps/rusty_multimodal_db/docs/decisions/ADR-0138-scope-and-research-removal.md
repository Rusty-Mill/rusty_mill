# ADR-0138: Scope — an embedded engine first; the research bucket removed

- Status: **Accepted** (2026-11-01, owner's answers to the 2026-11-01 review)
- Date: 2026-11-01
- Deciders: baileyrd
- Related: `ADR-0124` (the engine extracted to libs), `ADR-0008`
  (the production default), `RESULTS.md` (the measurements)

## Context

The 2026-11-01 review found the engine
(`crates/libs/storage/rusty_multimodal_db_engine`) is the reusable asset,
with seven consumers. It also found three problems:

- The engine keeps every record in RAM and rewrites its record blob whole.
  This caps every consumer. agent-cloud built its own segment log for this
  reason; the proposal to fix it in the engine (PR #773) closed unmerged.
- The `research` feature still compiled the alternatives `ProductionStore`
  won against: three `Dog` backends, seven durability variants, three
  concurrency strategies, the spikes, and an external-database benchmark.
  That benchmark's bundled DuckDB took 874 of 1,005 s of the CI test set
  (2026-10-27) and kept the crate off Windows CI.
- The project's stated goal (replace SQLite, Postgres and DuckDB) and its
  docs (`FUTURE-GROWTH.md`: "a different tier of project") disagreed.

## Decision

1. **Scope: both, in order.** Near term, the product is an embedded,
   SQLite-class engine for the owner's own apps. Replacing SQLite,
   Postgres and DuckDB stays the long-term goal; work toward it goes
   through the engine, not around it.
2. **Storage before features.** The next engine work is paged storage, so
   records no longer have to fit in RAM. The replacement for `bincode` 1
   (unmaintained, `RUSTSEC-2025-0141`) ships with it, as one format
   migration rather than two. Vectors and other new features come after.
   *Amended by ADR-0139 (proposed): the codec ships first, on its own;
   it is byte-identical, so it needs no migration.*
3. **The research bucket is removed, not kept behind a flag.** This PR
   deletes the alternative backends, durability variants, concurrency
   strategies, spikes, their benches, `tests/cross_backend.rs`,
   `tests/duckdb_compatibility.rs`, `examples/memory_footprint.rs`, and
   the `perf-events` and `external-db-bench` features with their
   dependencies (`redb`, `dashmap`, `rusqlite`, `duckdb`, `postgres`,
   `criterion-perf-events`, `perfcnt`).

The code stays readable at `e1a4795363c0e760855bea93108c56d745db3d81`
(main before this change). `RESULTS.md` and the earlier ADRs stay as the
record of what was measured.

## What `research` still gates

The feature keeps its name, so scripts and docs that pass it still work.
It now gates only:

- the reference domains: `Order`/`Customer` (from the engine) and
  `Employee`;
- the dataset builders;
- the crash-safety and multi-process harnesses, which the CI crash-safety
  test drives.

`AosStore` and `CanonicalCachedStore` remain as test-only oracles.

## Consequences

- About 9,600 fewer lines in the app crate (+138/−9,782), and 644 fewer
  lines in `Cargo.lock`. CI no longer builds DuckDB.
- Re-running an old comparison means checking out the commit above.
- The Windows exclusion's original reason is gone. Re-enabling Windows is
  a separate change, because this crate has never been tested there.
- Not done here: renaming `research` and `generic_spike`, and the paged
  storage design. The design is next.
