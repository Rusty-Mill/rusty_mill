# ADR-0109: `dogserver_mvcc_history_entries`, One Gauge Per MVCC Table

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "2 and 3" — the metric `ADR-0096`/`ADR-0105` held
  open). No wire change: the `Metrics` text gains a family, which is
  content, not shape.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0064` (`ServerMetrics`, the Prometheus text),
  `ADR-0069` (the HTTP scrape), `ADR-0096` (`history_len`), `ADR-0105`
  (the automatic reclaim this gauge lets an operator watch).
- Supersedes/Superseded by: none. Additive:
  `ConnectionStore::mvcc_history_entries` (defaulted `None`),
  `ServeOptions::render_metrics`, `ServeOptions::metric_tables`.

## Context

`ServerMetrics` holds process-wide atomic counters the request loop
increments; the adapters never see it, so a per-table size read from
the version index had no way in. `ADR-0105` bounds the history but
an operator could not watch it.

## Decision

Implement: the trait gains `mvcc_history_entries(&self) ->
Option<u64>` — `None` for a table with no MVCC state (no line),
`Some(index.history_len())` for one with. `serve_tables` hands the
table list to `ServeOptions::metric_tables`; `render_metrics` renders
`ServerMetrics::render` and then, when any table has state, one
`# HELP`/`# TYPE gauge` header and one
`dogserver_mvcc_history_entries{table="<name>"} <n>` sample per such
table, read live at render time. Both the `Metrics` request and the
HTTP scrape call it. Proven over the wire (`0` before the first
`BeginWith`, the seeded baseline after) and at the adapter.

## Consequences

- Positive: the size `ADR-0105` bounds is visible; a deployment can
  alert on a long-held snapshot pinning history.
- Negative / tradeoffs: a render takes each MVCC table's index lock
  briefly; `ServeOptions` now knows its tables, one more field set by
  `serve_tables`.
- Named, not hidden: a table without MVCC state has no sample rather
  than a `0` — absence means "not an MVCC table", `0` means "MVCC
  state, nothing recorded yet".

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.88.0 / `FR-101`, in one PR with `ADR-0108`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 973 tests across 44 targets, 0 failed. Builder: Claude; independent Codex inspection owed.
