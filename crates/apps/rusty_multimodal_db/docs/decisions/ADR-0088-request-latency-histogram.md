# ADR-0088: Request Latency Histogram

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0087` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  the first "still absent" item of the metrics future-growth entry; the
  fork below is held open for the owner at review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-REQUEST-LATENCY-DESIGN.md` (the full
  design), `ADR-0064` (`ServerMetrics`, whose Non-goal this was),
  `ADR-0069` (the HTTP scrape), `ADR-0086` (the plan family, the same
  site and the same amendment), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `LATENCY_BUCKETS_SECONDS`,
  a fixed-width histogram on `ServerMetrics` rendered as
  `dogserver_request_duration_seconds`, one `Instant` per request and
  one `record_latency` at the existing post-dispatch site. No wire,
  protocol, adapter, or client change.

## Context

`ADR-0064` deferred latency to the offline bench: "needs bucketing
machinery and a real decision about bucket boundaries". The machinery
is seventeen atomics and a cumulative render; the decision is made
against this crate's own measurements — sixteen bounds from 100 µs to
10 s in a 1-2.5-5 progression, so an indexed read sits in the first two
buckets, a walk in the sub-millisecond ones, a 100K scan around 100–250
ms, and anything past ten seconds is visible in `+Inf`.

## Decision

Implement: the constant; `latency_buckets`/`latency_count`/
`latency_sum_micros` on `ServerMetrics` with `record_latency(Duration)`
(one bucket add, the count, a saturating sum in microseconds);
`handle_connection` stamps `Instant::now()` once a request's frame is
read and records the elapsed time at the post-dispatch site for every
dispatched request, ok or not — the same population as
`requests_total`; the render gains one `HELP`/`TYPE histogram` pair,
cumulative `_bucket` lines ending at `+Inf`, `_sum` (seconds, six
decimals) and `_count`, before `uptime_seconds` (still the last line).
`MET-FR-002`'s bounded, fixed set is amended by one histogram of fixed
width. Proven in-process and over a socket.

The fork, held for the owner:

- **(a) As implemented.** One histogram, fixed buckets.
- **(b) (a) plus a `kind` label** — per-`RequestKind` latency, 17× the
  lines.
- **(c) Decline and revert.**

## Consequences

- Positive (a): a p50/p99 on a running server from a stock Prometheus
  `histogram_quantile`; the cost is one `Instant::now()` and three
  atomic adds per request (measured: the planner rows within noise,
  two after-runs were taken because the first showed one ~2.5× spike (`eq-range` `fpage-50` 100.6 → 258.4 µs) beside siblings that did not move; the second run put it back (102.6) and spiked a different row instead (`eq-range` `count(*)` 91.6 → 214.1, from 103.9 in the first) — one row per run jumping ~2× is this container's pattern, not a per-request cost, which would move every row by the same few microseconds. Across both runs the rows move in both directions (`index-eq` `query` 1,248.2 → 1,564.2 → 1,313.7; `due-count` 461.4 → 356.3 → 300.3; `range-tight` `count(*)` 45.6 → 48.5 → 48.6; `index-range` `fpage-50` 127.2 → 125.3 → 105.7) and the sub-100 µs rows sit within their usual spread).
- Named, not hidden: one histogram for every request kind; the
  observation ends when the response is ready, not when it is written;
  queue depth and journal size remain absent.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.73.0 /
  `FR-085`; see the design's change history for the proof and the
  measurement. Builder: Claude, under the host-takeover convention;
  independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 643 (up from 642), `server_metrics_integration`
  5 (up from 4), every other target unchanged and green, 933
  tests across 39 targets, 0 failed, every pre-existing test
  unmodified. Measured (`benches/server.rs`'s existing planner rows,
  an idle 4-core Linux container, both binaries back to back): every
  row within noise (two after-runs were taken because the first showed one ~2.5× spike (`eq-range` `fpage-50` 100.6 → 258.4 µs) beside siblings that did not move; the second run put it back (102.6) and spiked a different row instead (`eq-range` `count(*)` 91.6 → 214.1, from 103.9 in the first) — one row per run jumping ~2× is this container's pattern, not a per-request cost, which would move every row by the same few microseconds. Across both runs the rows move in both directions (`index-eq` `query` 1,248.2 → 1,564.2 → 1,313.7; `due-count` 461.4 → 356.3 → 300.3; `range-tight` `count(*)` 45.6 → 48.5 → 48.6; `index-range` `fpage-50` 127.2 → 125.3 → 105.7) and the sub-100 µs rows sit within their usual spread) — `RESULTS.md`. The fork above
  remains the owner's at review; (a) is what merges if the PR merges
  unchanged.
