# ADR-0093: Connection Limits — Idle Timeout, Connection Cap, Row Cap

- Status: **Proposed and implemented on one branch; the owner asked for
  the hardening** (2026-09-21). The second "do now" item of the
  release-readiness review, after `ADR-0092`.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-CONNECTION-LIMITS-DESIGN.md` (the full
  design), `ADR-0010` (thread-per-connection, "drain is a non-goal"),
  `ADR-0030` (the one bounded per-peer table before this), `ADR-0064`
  (`ServerMetrics`), `ADR-0067` (`ErrorCode::TooLarge`, reused),
  `ADR-0072` (an idle MVCC session pins `min_open_snapshot`),
  `ADR-0034` (`evaluate_query`'s truncate-after-collect).
- Supersedes/Superseded by: none. Additive: three `ServeOptions`
  fields and builders (`with_idle_timeout`, `with_max_connections`,
  `with_max_query_rows`), `request_exceeds_row_cap`, one guard arm,
  one check at accept, `dogserver_connections_refused_total`, three
  `memory_server` environment variables. No wire change (protocol 29;
  `TooLarge` existed since 25).

## Context

The server had no limit of any kind: a connection that never spoke
held its thread, its session, and its MVCC snapshot forever; every
accept spawned a thread without bound; a `Query` with no `limit`
materialized every matching row before truncating, and a reply past
`MAX_FRAME_BYTES` dropped the connection with no error code. The
release-readiness review rated the three together High.

## Decision

Implement, each opt-in and unset by default so no existing deployment
changes: `with_idle_timeout` sets the socket's read and write
timeouts before the TLS handshake, so a stalled peer is closed as a
disconnect is; `with_max_connections` is checked at accept against an
in-flight count — a refused socket is closed with nothing written and
counted; `with_max_query_rows` refuses a `Query` with no `limit` or a
`limit` above the cap, and a page whose `limit` is above it, with
`TooLarge` before any read (`Malformed` below 25). `evaluate_query`
stops at `limit` while filtering, so at most `limit` rows are ever
materialized, cap or no cap. `memory_server` reads
`SERVER_IDLE_TIMEOUT_SECS`, `SERVER_MAX_CONNECTIONS`,
`SERVER_MAX_QUERY_ROWS`.

## Consequences

- Positive: an operator can bound threads, idle sessions, and reply
  size without a wire change; the loudest failure (a silent drop past
  16 MiB) becomes a named refusal under the cap.
- Negative / tradeoffs: opt-in means a deployment that sets nothing is
  exactly as unbounded as before — the fork for the owner is defaults
  on in `memory_server`. A refused accept is EOF to the client, not an
  error frame (an error frame would be a protocol round). `Aggregate`
  and `Join` are not capped: their cost is the scan, not the reply.
- Named, not hidden: the idle timeout is a socket timeout, so it also
  bounds a slow *write* of a large reply; set it above the largest
  reply's transfer time.

## Acceptance and implementation

- 2026-09-21: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.78.0 / `FR-090`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 651 (up from 650), `server_limits_integration` 3 (new), 948 tests across 41 targets, 0 failed. Builder: Claude; independent Codex inspection owed.
