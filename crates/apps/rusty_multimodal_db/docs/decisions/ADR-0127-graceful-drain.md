# ADR-0127: Graceful Drain on `Shutdown`, and SIGTERM in `memory_server`

- Status: **Proposed and implemented on one branch; the owner chose it**
  (2026-09-30, "scan budget + drain ... and the multi-year items").
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0010` (drain a non-goal of the first design), `ADR-0093`
  (connection cap, idle timeout), `ADR-0072` (an open MVCC session pins a
  snapshot), `ADR-0126` (the round before).
- Supersedes/Superseded by: none. Additive: `Shutdown`,
  `ServeOptions::{with_shutdown, with_drain_timeout}`,
  `DEFAULT_DRAIN_TIMEOUT`, `SERVER_DRAIN_TIMEOUT_SECS`, a `rusty_libc`
  dependency behind `server` (Linux). No wire change.

## Context

`serve_tables` ran until the listener errored. A deployment could only stop
it by killing the process, which drops every connection mid-request, rolls
back sessions by accident of exit, and leaves clients guessing.

## Decision

A `Shutdown` handle, given to `ServeOptions::with_shutdown`. `request()`:

1. The accept loop stops (the listener is polled every 20 ms so the flag is
   seen while idle; with no handle configured the original blocking loop is
   untouched).
2. The *read* side of every open connection is shut. A request being
   answered completes and its response is written; the next read is an end
   of stream, which ends the connection through the path a client
   disconnect takes: session rolled back, MVCC snapshot released, gauges
   decremented, all by the existing `Drop` guards. Nothing is cut
   mid-write.
3. `serve_tables` waits for the in-flight count to reach zero, at most
   `with_drain_timeout` (default 30 s), then returns.

`memory_server` (Linux): SIGTERM and SIGINT call `request()` through
`rusty_libc::signal`. The handler stores one atomic (async-signal-safe); a
watcher thread does the rest. `SERVER_DRAIN_TIMEOUT_SECS` sets the deadline.
Elsewhere, or if a handler cannot be installed, the binary runs as it did.

## Consequences

- Positive: a supervisor's SIGTERM ends the server without cutting a
  response, and the process exits 0.
- Negative / tradeoffs: a request whose frame is only partly read when the
  read side closes is dropped with its connection (the client was
  mid-send); a request that runs past the drain timeout is left to the
  process exit. The accept poll costs up to 20 ms of accept latency, only
  when a `Shutdown` is configured and the socket is idle.
- New dependency: `rusty_libc`, first-party, already in the workspace,
  behind the `server` feature and Linux only; one `unsafe` call site with
  its safety comment. `docs/WORKSPACE-MAP.md` regenerated.
- Not done: draining the `/metrics` listener (it holds no state), and a
  wire "going away" frame (a protocol round).

## Acceptance and implementation

- 2026-09-30: `SERVER-001` v0.103.0 / `FR-116`.
  `tests/server_drain_integration.rs` (idle connection closed and the
  listener gone; an in-flight request answered in full first; a request
  made before `serve` starts returns at once) and
  `tests/memory_server_drain.rs` (SIGTERM and SIGINT exit 0). fmt, clippy
  (`--features server,research -D warnings`) and the crate's tests clean;
  workspace dependency and layer checks pass. Builder: Claude; independent
  inspection owed.
