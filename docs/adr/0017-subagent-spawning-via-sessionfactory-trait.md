# ADR-0017: Subagent spawning via a `SessionFactory` trait

- Status: Accepted
- Date: 2026-05-27
- Tags: architecture, dag, subagents

## Context

The `agent` subagent tool lives in the `feed` crate (it is a registered tool),
but spawning a subagent means constructing a `Session`, which lives in the `app`
crate. Since `app` already imports `feed`, having `feed` construct an `app::Session`
directly would create a `feed -> app -> feed` import cycle and break the acyclic
crate DAG (consolidated plan §A.7). The crate dependency graph must remain a DAG
(PRD 06).

## Decision

Introduce a `SessionFactory` (spawn) trait in a low crate. The `agent` tool
depends on the trait, not on `app`; `app` provides the concrete implementation
that builds a `Session` and injects it at startup. This inverts the dependency so
no high-to-low import edge is created. See `docs/ARCHITECTURE.md` for the crate
DAG and the trait's placement.

## Consequences

- Subagent construction is decoupled from the concrete `Session` type behind a
  trait object, preserving acyclicity.
- One more indirection at the spawn boundary; negligible relative to the cost of
  spawning a subagent turn.
- Subagent system-prompt inheritance and max-depth (`RUSTYKEYS_MAX_AGENT_DEPTH`)
  are handled by the factory implementation.
