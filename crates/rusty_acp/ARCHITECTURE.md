# Architecture

## Overview

A Rust implementation of the [Agent Communication Protocol][acp] v0.2.0 — protocol types, an
HTTP client, and an [`axum`] router that hosts agents behind the standard endpoints. Each of the
three is usable without the others.

The design target is ACP's [high-availability guide][ha]: identical replicas behind a load
balancer with **no session affinity**. Any replica must be able to serve any request for any run,
including runs it is not executing.

**Non-goals** are listed at the bottom, and the first one is the one that shapes the rest: this
is not published to crates.io.

## Boundaries

One port carries essentially all of the I/O. That is deliberate — the replica-agnostic property
above is exactly the statement that everything durable goes through a single interface.

| Port | Adapter(s) | Notes |
| --- | --- | --- |
| `server::store::Store` | `InMemoryStore`, `RedisStore`, `PostgresStore`, `MeteredStore` | Runs, event logs, sessions, ownership leases and per-run pub/sub. Every endpoint reads and writes through it, so swapping the adapter is the whole of the single-node → HA change. |
| `server::Agent` | `FnAgent` (via `agent_fn`), user impls | The inbound port. An agent sees `RunContext` and never the store, the router, or HTTP. |
| `telemetry` (server + client) | `metrics` facade, `tracing` | Emits names and identifiers; chooses no exporter. Both are feature-gated so a build that wants neither carries neither. |
| `trace::TraceContext` | W3C `traceparent` header | Correlation across replicas, behind the `trace` feature. |

`Store` is a published contract, not an internal convenience: `server::store::testkit` ships a
16-check conformance suite behind the `store-testkit` feature so a third-party backend can be
held to the same invariants the built-in three are. `tests/store_conformance.rs` runs it against
all three.

## Structure

A single crate with feature-gated layers rather than a workspace of small crates. The layers
(`types` / `client` / `server` / per-backend stores) are separable at compile time through
Cargo features, which gives the isolation a split would give without the version-skew that
comes with publishing several crates that must agree.

Domain logic stays out of the adapters. `types` has no I/O at all — it is the wire format and
its validation, and it builds with `--no-default-features`. `server::run` holds the run state
machine and talks to `Store`; `server::routes` holds HTTP and talks to `server`.

## Data flow

A run, in the multi-replica case:

1. `POST /runs` arrives at **replica A**. It resolves the session, checks admission and capacity,
   writes the run, subscribes to its channel, and spawns the agent.
2. The agent emits events. Each one is appended to the durable log *and* published on the run's
   channel — append first, then publish, because a subscriber may resolve the notification by
   reading the log.
3. **Replica B** serving `GET /runs/{id}/events` reads the same log and splices a live
   subscription onto the replay, so a client attached to B sees A's work.
4. `POST /runs/{id}/cancel` at **replica C** publishes a `Cancel`; A's control listener applies it
   locally and writes the outcome. Nobody but A writes A's run.
5. Terminal transition: session history is written **before** the terminal event, because that
   event is what releases a `sync` caller and a caller told its run is done must not then read a
   session missing that run's output.

If A dies, its lease lapses; whichever replica next reads the run reaps it, or — for an agent
that declares itself recoverable — claims it and starts a replacement.

## Invariants

Four rules the code depends on. Breaking any of them is a correctness bug, not a style question.

1. **The replica executing a run is its sole writer.** This is what lets `put_run` be a plain
   overwrite with no distributed locking.
2. **Terminal transitions apply exactly once**, so a cancellation racing a completion cannot
   rewrite the outcome.
3. **The terminal event releases `sync` callers**, so anything a caller could reasonably read
   next must be written before it goes out.
4. **Storage failures fail the run.** Emitting is `async` and returns `Result` precisely so a
   storage outage produces a failed run rather than a silently truncated one.

A non-terminal run with no live lease has lost its writer, and is reaped by whichever replica
next reads it.

## Key decisions

See [docs/adr/](./docs/adr/) for individual decisions and their tradeoffs. Until this repo has
real ADRs, the reasoning lives in the merged PR bodies and in the module docs, which are written
to carry it — `src/server/store/mod.rs` and `src/trace.rs` in particular state what was rejected
and what it would have cost.

## Non-goals

- **Publishing to crates.io.** Depend on it from git. No release workflow, no `docs.rs` link, no
  version-based install snippet.
- **Choosing an observability backend.** `metrics` is a facade and `trace` emits identifiers;
  neither picks an exporter, because taking `tracing-opentelemetry` would pick the ecosystem for
  every dependent.
- **Raising the MSRV for optional dependencies.** `rust-version` is 1.86. `redis-store` and
  `postgres-store` need 1.88 because their dependencies do; an optional dependency does not
  raise the floor for everyone else.
- **Extending the wire format.** Where this crate serves more than ACP defines — `/ready`, the
  resumable SSE stream, `Acp-Events-From` — it is an extension a client can ignore, and
  `tests/spec_coverage.rs` fails if one is added without being declared as such.

[acp]: https://agentcommunicationprotocol.dev
[ha]: https://agentcommunicationprotocol.dev/how-to/high-availability
[`axum`]: https://github.com/tokio-rs/axum
