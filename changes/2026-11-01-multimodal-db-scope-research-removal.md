---
category: Removed
changelog: rusty_multimodal_db drops the benchmarked-alternative backends, durability variants, concurrency strategies, spikes and the DuckDB/Postgres/SQLite comparison bench; ADR-0138 records the engine-first scope.
---
## 2026-11-01 - rusty_multimodal_db: engine-first scope, research bucket removed

- **Removed:** the alternative `Dog` backends, durability variants,
  concurrency strategies, spikes, their benches, and the `perf-events` and
  `external-db-bench` features with `redb`, `dashmap`, `rusqlite`,
  `duckdb`, `postgres`, `criterion-perf-events` and `perfcnt`. CI no longer
  builds DuckDB. The code stays readable at the commit ADR-0138 names.
- **Changed:** `research` now gates only the reference domains, the dataset
  builders and the durability harnesses. ADR-0138 records the scope:
  an embedded engine first, with paged storage before new features.
- **Known limitations:** the crate is still excluded from Windows CI, and
  the `research`/`generic_spike` names are kept for now.
