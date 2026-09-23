# ADR-0111: The Review's Mediums 4–8, Fixed

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23 — the review's second round, after "merge" of
  `ADR-0110`). No wire change; `SERVER-002` 0.19.2 records one
  semantics narrowing on `RowsClamped`.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `docs/reports/2026-09-23-hardening-line-review.md` (Mediums
  4–8), `ADR-0102`/`ADR-0103` (the clamp, its counter and its mark),
  `ADR-0094` (the exposure check), `ADR-0104` (the refusal pool),
  `ADR-0069` (the metrics HTTP listener), `ADR-0110` (the round before).
- Supersedes/Superseded by: amends `CLP-FR-002` (where the counter
  counts), `WCB-FR-001` (when the mark is sent), `WCB-FR-004`
  (`last_clamp`), `EXP-FR-002` (the metrics listener), `BTL-FR-002`
  (the drain). Additive: `exposure::check_metrics_exposure`,
  `Exposure::MetricsListener`.

## Context

Five Mediums from the review, each a place where a mechanism was
right and its accounting, wording, or bound was not:

4. `dogserver_query_rows_clamped_total` counted at the clamp step,
   before authentication and validation — an unauthenticated peer
   could drive it, and a refused `Query` counted as clamped.
5. `RowsClamped` marked every `Query` the clamp rewrote, complete or
   not; four docs said it meant "truncated". A five-row table under
   the default cap answered `RowsClamped{cap:10000}`.
6. `last_clamp` went stale across the point-read and `ORDER BY` paths
   of `query`, which return rows without touching it.
7. The metrics HTTP listener was bound after `check_exposure` ran and
   was never checked itself; it has no auth or TLS to configure, so
   `SERVER_METRICS_HTTP_ADDR=0.0.0.0:9100` served counters and table
   names to the network with no warning.
8. The refusal thread's drain loop restarted the 2 s socket timeout on
   every read, so sixteen peers trickling a byte each could pin the
   whole refusal pool indefinitely.

## Decision

Implement all five:

- `RVM-FR-001` — the counter increments where the answer is marked,
  after dispatch, only when it is `RowsClamped`; the HELP text says
  "whose answer was cut at the row cap".
- `RVM-FR-002` — `mark_clamped` marks only when `rows.len() >= cap`:
  the answer holds exactly `cap` rows and more may have matched. An
  answer shorter than the cap is complete and goes as `Rows`. The
  four docs now say that. Proven: an unmatched filter under the cap
  answers an empty, unmarked `Rows`, uncounted.
- `RVM-FR-003` — `last_clamp` is reset at the start of every `query`;
  only a `RowsClamped` answer sets it.
- `RVM-FR-004` — `exposure::check_metrics_exposure(addr)`: only a
  loopback bind is `Ok`; `memory_server` refuses a non-loopback
  metrics bind naming `SERVER_METRICS_HTTP_ADDR`, or warns under
  `SERVER_ALLOW_INSECURE=1`. Proven in the exposure test with a
  loopback wire bind and an exposed metrics bind.
- `RVM-FR-005` — the drain runs under an `Instant` deadline of one
  `BUSY_REFUSAL_TIMEOUT` in addition to the per-read socket timeout.

## Consequences

- Positive: the counter means what its name says and cannot be
  driven before authentication; a client reading `last_clamp()` /
  `RowsClamped` as "possibly incomplete" gets no false positive on
  the common case; the metrics listener obeys the same rule as the
  wire listener; a refusal thread lives at most ~two timeouts.
- Negative / tradeoffs: a clamped answer of exactly `cap` rows that
  happens to be complete is still marked — the server cannot tell
  without reading one more row; named as the remaining imprecision.
  A deployment that bound its metrics listener to a non-loopback
  address without `SERVER_ALLOW_INSECURE` now fails at startup.
- Named, not hidden: `history_len` per scrape (Medium 9), the
  automatic reclaim's counter including replayed entries (10), the
  two unsynced creations and the backup rename (11), the journaled
  update's `Ok(false)` after fsync (12), the stale `TooLarge` doc
  (13), and the `Cargo.toml` comment (14) are the next round.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.90.0 / `FR-103`, `SERVER-002` 0.19.2.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 976 tests across 44 targets, 0 failed. Builder: Claude.
