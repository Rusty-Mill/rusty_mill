# Server Request Latency Histogram (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0088`; the `ADR-0059`/`ADR-0076`–`ADR-0087` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as the first
  "still absent" item of `docs/FUTURE-GROWTH.md`'s metrics entry
  ("request latency histograms, queue depth, journal size"); the fork
  below stays open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0064`/`docs/design/SERVER-METRICS-DESIGN.md`
  (`ServerMetrics`; whose Non-goal this was — "latency
  histograms/percentiles: needs bucketing machinery and a real decision
  about bucket boundaries; a genuinely separate, larger round if ever
  wanted"), `ADR-0069` (the HTTP scrape renders the same text),
  `ADR-0086` (the plan family — the same post-dispatch site, the same
  "one fixed family" amendment of `MET-FR-002`), `benches/server.rs`
  (whose numbers chose the buckets).
- Supersedes/Superseded by: none. Additive: `LATENCY_BUCKETS_SECONDS`,
  a histogram on `ServerMetrics` rendered as
  `dogserver_request_duration_seconds`, one `Instant` at each request's
  arrival and one `record_latency` at the existing post-dispatch site.
  No wire, protocol-version, adapter, or client change. The `Metrics`
  text gains one histogram (`RLH-FR-004`).

## Purpose and scope

`ADR-0064` answered "is it up and how loaded", deferring "how fast right
now" to the offline bench. `ADR-0086` put the *path* each read took on
the scrape; this round puts the *time* each request took beside it, as
a Prometheus histogram with fixed buckets — so an operator can see a
p99 drift on a running server, not only a bench on an idle one.

The "real decision about bucket boundaries" is made against this
crate's own measurements: sixteen bounds from 100 µs to 10 s in a
1-2.5-5 progression. An indexed point read on the loopback bench sits
in the first two buckets, the `Ordered` walks in the sub-millisecond
ones, a 100K full scan around 100–250 ms, and nothing this server
answers should take ten seconds; anything that does lands in `+Inf`
and is visible for it.

Scope, exactly: the buckets (`RLH-FR-001`); the counters (`RLH-FR-002`);
the recording (`RLH-FR-003`); the render (`RLH-FR-004`); no other
surface changed (`RLH-FR-005`).

## Non-goals

- **Per-request-kind or per-plan latency.** One histogram; the
  cardinality stays fixed. A `kind` label is 17 histograms.
- **Queue depth, journal size, index stats.** Named in the same
  future-growth line; each its own round.
- **Configurable buckets.** A constant, as `INTERSECT_WALK_BUDGET` is.
- **Measuring the write.** The observation ends when the response is
  ready, not when its bytes have left; the socket write is the client's
  side of the round trip.

## Context and terminology

Read from `main` after PR #287 (`SERVER-001` v0.72.0) this pass:

- **`LATENCY_BUCKETS_SECONDS`** (`metrics.rs`): the sixteen bounds,
  ascending; `+Inf` implicit.
- **The observation**: `Instant::now()` once each request's frame is
  fully read (`arrived`), to the post-dispatch site where
  `record_request` already runs — dispatched requests only, the same
  population as `requests_total` (`Hello`/`Authenticate` and the gates
  that `continue` before dispatch are not observed).
- **Storage**: one non-cumulative count per bucket plus `+Inf`
  (cumulated at render — one atomic add per observation, not sixteen),
  the count, and the sum in whole microseconds (an integer, so it is one
  saturating atomic add; rendered as seconds to six decimals).
- **`MET-FR-002` amended again**: six counters, the plan family, and
  one histogram of fixed width — still bounded at compile time.

## Requirements

- `RLH-FR-001` **The buckets.** As "Context"; a unit test pins them
  ascending.
- `RLH-FR-002` **The counters.** `latency_buckets: [AtomicU64; 17]`,
  `latency_count`, `latency_sum_micros`; `record_latency(Duration)`.
- `RLH-FR-003` **The recording.** `handle_connection` stamps `arrived`
  after the frame is read and records `arrived.elapsed()` at the
  post-dispatch site, for every dispatched request, ok or not.
- `RLH-FR-004` **The render.** After the plan family and before
  `uptime_seconds` (still the last line): one `HELP`/`TYPE histogram`
  pair, cumulative `_bucket{le="…"}` lines in bound order ending at
  `+Inf`, `_sum`, `_count`; every line present at zero on a fresh
  server; the HTTP scrape identical.
- `RLH-FR-005` **Everything else unchanged.** `dispatch`, the planner,
  the wire (`PROTOCOL_VERSION` 27), the clients: untouched.

Proven: the unit test (fixed ascending buckets; observations landing in
the first bucket whose bound they do not exceed, inclusive; the render
cumulating to `+Inf` = count; `_sum` to six decimals; zero on a fresh
instance; uptime last) and, over a real socket, the histogram's count
advancing by exactly `requests_total`'s delta, `+Inf` equal to the
count, buckets monotone, the sum bounded by the client's wall-clock.
Every pre-existing test unmodified. Measured: the planner rows before
and after — the cost is one `Instant::now()` and three atomic adds per
request.

## Considered options

- **(a) One fixed-bucket histogram at the existing site — implemented.**
- **(b) (a) plus a `kind` label** — per-`RequestKind` latency; 17× the
  lines, a real cardinality decision.
- **(c) Decline.** "How fast" stays offline.

The owner's shorthand: **(a)** as implemented; **(b)** (a) per request
kind; **(c)** decline and revert.

## Proposed shape

`src/server/metrics.rs`: `LATENCY_BUCKETS_SECONDS`, the three fields,
`record_latency`, `render_latency`, the render. `src/server/serve.rs`:
`arrived` and one `record_latency` line in `handle_connection`.

## Data/state and invariants

- `bucket[+Inf] (cumulative) == count`; `count == requests_total`
  (both incremented at the same site, once per dispatched request).
- Buckets are non-cumulative in memory and cumulative in the text.

## Errors, failure, recovery, and observability

This round is observability. The sum saturates at `u64::MAX` µs.

## Security, privacy, and compatibility

Twenty more lines on a read already gated at the version gate; no wire
change. As `ADR-0086`: the text's grammar, not its line set, is the
contract.

## Acceptance criteria

1. The unit test above.
2. Integration (`tests/server_metrics_integration.rs`): the socket
   proof above.
3. Measured (`RESULTS.md`): the planner rows within noise.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench before and after, back to back. Independent review
owed.

## Traceability

- Roadmap: `SERVER-REQUEST-LATENCY`.
- Decision: `ADR-0088`.
- Specification: `SERVER-001` v0.73.0 / `FR-085` (amends `FR-064`'s
  `MET-FR-002` again).
- Requirements: `RLH-FR-001`–`005`.

## Open questions

- **A `kind` label** — option (b).
- **Queue depth and journal size** — the rest of the future-growth line.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as the
  first "still absent" item of the metrics entry. Read from `main`
  after PR #287 this pass. The existing planner rows were measured on
  the pre-change code first.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.73.0 /
  `FR-085`, exactly the "Proposed shape". Acceptance criteria 1–2 are
  the tests: `metrics.rs` +1
  (`latency_histogram_cumulates_fixed_buckets_and_keeps_uptime_last`),
  `tests/server_metrics_integration.rs` +1
  (`request_duration_histogram_counts_every_dispatched_request`)
  (lib 643, up from 642; metrics integration 5, up from 4);
  933 tests across 39 targets, 0 failed, every pre-existing test
  unmodified. Criterion 3, `RESULTS.md`: every planner row within noise
  (two after-runs were taken because the first showed one ~2.5× spike (`eq-range` `fpage-50` 100.6 → 258.4 µs) beside siblings that did not move; the second run put it back (102.6) and spiked a different row instead (`eq-range` `count(*)` 91.6 → 214.1, from 103.9 in the first) — one row per run jumping ~2× is this container's pattern, not a per-request cost, which would move every row by the same few microseconds. Across both runs the rows move in both directions (`index-eq` `query` 1,248.2 → 1,564.2 → 1,313.7; `due-count` 461.4 → 356.3 → 300.3; `range-tight` `count(*)` 45.6 → 48.5 → 48.6; `index-range` `fpage-50` 127.2 → 125.3 → 105.7) and the sub-100 µs rows sit within their usual spread). Still no independent review — owed.
