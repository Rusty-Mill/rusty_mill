# ADR-0102: A `Query` With No `limit` Is Clamped to the Row Cap, Not Refused

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-22, "clamp"). The fork `ADR-0099` held open.
- Date: 2026-09-22
- Deciders: baileyrd
- Related: `ADR-0093`/`docs/design/SERVER-CONNECTION-LIMITS-DESIGN.md`
  (`LIM-FR-003`, which refused a `Query` with no `limit` under a cap),
  `ADR-0099` (which therefore left the row cap off by default),
  `ADR-0034` (`evaluate_query`: `limit` is the first `n` matches in
  scan order, not a top-N), `ADR-0064` (`ServerMetrics`).
- Supersedes/Superseded by: amends `LIM-FR-003` for the no-`limit`
  case. Additive: `serve::clamp_missing_limit`, one step before the
  dispatch match, `dogserver_query_rows_clamped_total`,
  `DEFAULT_MAX_QUERY_ROWS` in `memory_server`. No wire change.

## Context

Under `ADR-0093`'s cap a `Query` with no `limit` was `TooLarge`, and
every `SELECT` without `LIMIT` compiles to one, so `ADR-0099` could
not turn the cap on by default without failing the consumer's plain
reads. A missing `limit` is the one shape the cap exists to bound: it
asks for every matching row.

## Decision

Implement: before the dispatch match, under a cap, a `Query` whose
`limit` is `None` is rewritten to `Some(cap)` and counted in
`dogserver_query_rows_clamped_total`; the answer is exactly what
`limit: Some(cap)` returns — the first `cap` matches in scan order.
An explicit `limit` above the cap, and a page above it, are still
refused `TooLarge`: the client said what it wanted and it is not
allowed. `memory_server` now defaults `SERVER_MAX_QUERY_ROWS` to
10,000 (`0` off). Proven over a socket at a cap of two (no `limit`
answers two rows and moves the counter; `limit` 3 is refused; the SQL
client sees both) and through the binary's banner.

## Consequences

- Positive: the cap can be on by default; a plain `SELECT` still
  answers; the truncation is visible in metrics.
- Negative / tradeoffs: a clamped answer is silently short from the
  client's side — the reply carries no "truncated" mark (a wire
  change, named as the fork). An operator who sees the counter move
  knows which deployments need a higher cap or a `LIMIT`.
- Named, not hidden: the SQL client compiles `SELECT … LIMIT n` to
  `Some(n)` and everything else to `None`, so a `LIMIT` above the cap
  is the one way a client is refused.

## Acceptance and implementation

- 2026-09-22: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.83.0 / `FR-095`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 966 tests across 44 targets, 0 failed. Builder: Claude; independent Codex inspection owed.
