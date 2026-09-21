# Server Connection Limits: Idle Timeout, Connection Cap, Row Cap (Proposed and implemented)

- Status: **Proposed and implemented on one branch; the owner asked
  for it** (2026-09-21, `ADR-0093`). The second "do now" item of the
  release-readiness review.
- Date: 2026-09-21
- Related: `ADR-0010`/`docs/design/SERVER-QUERY-LAYER-DESIGN.md`
  (thread-per-connection; drain a non-goal), `ADR-0030` (rate
  limiting — the one bounded table before this), `ADR-0064`/`0069`
  (metrics and the HTTP scrape), `ADR-0067` (`TooLarge`), `ADR-0072`
  (an open MVCC session pins the oldest snapshot), `ADR-0034`
  (`evaluate_query`), `ADR-0092` (the round before).
- Supersedes/Superseded by: none. Additive: `ServeOptions` fields,
  `request_exceeds_row_cap`, `InFlightGuard`, the guard arm, the
  accept check, one metric, three environment variables.

## Purpose and scope

Bound the three unbounded things a peer could cost the server: a
thread held forever (no timeout), threads without limit (no cap), and
a reply of every row (no row cap, and every row materialized before
the truncate). Each bound is the operator's choice; nothing changes
for a deployment that sets none.

Scope, exactly: the idle timeout (`LIM-FR-001`); the connection cap
(`LIM-FR-002`); the row cap (`LIM-FR-003`); bounded materialization
(`LIM-FR-004`); the binary's settings (`LIM-FR-005`); everything else
unchanged (`LIM-FR-006`).

## Non-goals

- **An error frame for a refused accept.** Nothing is written; the
  client sees EOF. A `Busy` code would be a protocol round.
- **Capping `Aggregate`/`Join`/`Compact`.** Their cost is the scan or
  the rewrite, not the reply; a scan budget is a planner question.
- **Defaults on.** The fork for the owner; here every limit is opt-in.
- **Graceful drain / shutdown.** Still `ADR-0010`'s non-goal.
- **A per-peer cap.** One process-wide count.

## Context and terminology

Read from `main` after PR #292 this pass:

- **Idle timeout**: `TcpStream::set_read_timeout`/`set_write_timeout`
  on the accepted socket, before the TLS handshake and the split into
  reader/writer halves (both clones share it). A timed-out read or
  write is an `io::Error` to `framing`, which ends the connection as a
  disconnect does — session rolled back, MVCC snapshot released,
  metrics gauge decremented, all by the existing `Drop` guards.
- **Connection cap**: `ServeOptions::in_flight: AtomicUsize`, claimed
  by `try_admit` at accept (`fetch_add`, undone if past the cap) and
  released by an `InFlightGuard` the connection thread holds. A refused
  socket is dropped unread and `record_connection_refused` is called —
  `dogserver_connections_refused_total`, rendered after
  `connections_active`. Never in `connections_total`.
- **Row cap**: `request_exceeds_row_cap(req, cap)`, pure: a `Query`
  whose `limit` is `None` or above `cap`; a `Page`/`FilteredPage`/
  `PageDesc`/`FilteredPageDesc` whose `limit` is above `cap`; every
  other variant `false`. One guard arm in `handle_connection` after
  the contradiction arm: `TooLarge` at 25 or above, `Malformed` below
  (rule 3; the cap is the operator's, so no version is bumped).
- **Bounded materialization**: `evaluate_query` filters, projects, and
  `take(limit)`s in one pass — the same rows the truncate-after-collect
  it replaces produced (input order, first `limit` matches).
- **`memory_server`**: `SERVER_IDLE_TIMEOUT_SECS`,
  `SERVER_MAX_CONNECTIONS`, `SERVER_MAX_QUERY_ROWS` — each a positive
  integer or a startup error (`positive_env`).

## Requirements

- `LIM-FR-001` **Idle timeout.** As "Context"; a connection that sends
  nothing for longer than it is closed by the server; one that keeps
  talking under it is not.
- `LIM-FR-002` **Connection cap.** As "Context"; the `n+1`th
  concurrent accept is closed before any byte is read and counted;
  once a connection ends, a new one is admitted.
- `LIM-FR-003` **Row cap.** As "Context"; a refused read is an error
  the metrics see; `Metrics` itself is never capped; the schema-driven
  client surfaces `TooLarge` unchanged.
- `LIM-FR-004` **Bounded materialization.** At most `limit` projected
  rows are ever collected by `evaluate_query`; identical results.
- `LIM-FR-005` **The binary.** The three variables, parsed at startup.
- `LIM-FR-006` **Everything else unchanged.** No wire, protocol,
  client, adapter, or planner change; `ServeOptions::default()` and
  `new` reproduce the unbounded server exactly.

## Considered options

- **(a) Opt-in limits through `ServeOptions` — implemented.**
- **(b) (a) with defaults on in `memory_server`** (say 300 s, 1024,
  10 000) and `0` to disable. Changes a running deployment's
  behaviour at the next restart; the owner's call.
- **(c) A `Busy` error frame on a refused accept.** A protocol round.
- **(d) Decline.**

The owner's shorthand: **(a)** as implemented; **(b)** defaults on;
**(d)** decline and revert.

## Proposed shape

`src/server/serve.rs`: the fields, builders, getters, `try_admit`,
`InFlightGuard`, `request_exceeds_row_cap`, the guard arm, the accept
check, the timeout, `evaluate_query`. `src/server/metrics.rs`: the
counter. `src/server/protocol.rs`: `TooLarge`'s doc.
`src/bin/memory_server.rs`: `positive_env`, the three settings, the
banner. `tests/server_limits_integration.rs` (new).

## Data/state and invariants

- `in_flight` equals the number of live connection threads plus
  accepts between `try_admit` and the thread's guard — never below
  zero: every successful claim has exactly one guard.
- Under a cap `c`, `in_flight <= c` at every instant after `try_admit`
  returns.
- `evaluate_query(rows, s, f, Some(n)).len() <= n`, and the rows are
  the first `n` matches in input order.

## Errors, failure, recovery, and observability

A refused accept: EOF, `connections_refused_total`. A capped read:
`TooLarge`/`Malformed`, `requests_err_total`. A timed-out connection:
closed; the audit sink sees the disconnect as any other.

## Security, privacy, and compatibility

No wire change. Each limit narrows what an unauthenticated peer can
cost the process; none widens anything.

## Acceptance criteria

1. Unit: the refused counter (`metrics.rs`).
2. Integration (`tests/server_limits_integration.rs`): `LIM-FR-001`
   through `LIM-FR-003`.
3. Not measured: no request's path changes except `evaluate_query`'s
   early stop, which does strictly less work.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`.
Independent review owed.

## Traceability

- Roadmap: `SERVER-CONNECTION-LIMITS`.
- Decision: `ADR-0093`.
- Specification: `SERVER-001` v0.78.0 / `FR-090`.
- Requirements: `LIM-FR-001`–`006`.

## Open questions

- **Defaults on** — option (b). *Taken: `ADR-0099` (2026-09-21) — idle
  timeout 300 s and connection cap 1,024 on by default in
  `memory_server`, `0` turning either off; the row cap stays opt-in,
  since under it a `Query` with no `limit` is refused.*
- **A `Busy` error frame** — option (c), if a client should tell a
  full server from a dead one.
- **A scan budget** for `Aggregate`/`Join` — the planner's question,
  not this round's.

## Change history

- 2026-09-21: proposed and implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` on the owner's word
  ("harden"), the second "do now" item of the release-readiness review.
- 2026-09-21: implemented as `SERVER-001` v0.78.0 / `FR-090`. Acceptance
  criteria 1–2 are the tests: `metrics.rs` +1,
  `tests/server_limits_integration.rs` +3. `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 651 (up from 650), `server_limits_integration` 3 (new), 948 tests across 41 targets, 0 failed. Still no
  independent review — owed.
