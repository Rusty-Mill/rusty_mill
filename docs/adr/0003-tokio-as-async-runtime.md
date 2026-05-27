# ADR-0003: tokio as async runtime

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: async, runtime, concurrency

## Context

aisdk is tokio-native, and the system is network-I/O-heavy: every LLM call is a
network round trip, while local work (SQLite, file I/O) is fast. The runtime
must make the LLM calls genuinely concurrent and let post-turn work overlap with
the user reading the reply.

## Decision

Use tokio as the async runtime. All LLM calls are `await`; the rest of the
harness (SQLite, file I/O) uses tokio's `spawn_blocking` where needed. Post-turn
work (criteria judge plus consolidation) runs concurrently via `tokio::join!`.

## Consequences

- Async propagates through the call stack: every layer that touches an LLM call
  must be an `async fn`.
- Accepted as the correct default for a network-I/O-heavy system.
