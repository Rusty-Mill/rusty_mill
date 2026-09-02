# ADR-0010: Pluggable storage via Rust traits

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: storage, traits, memory

## Context

Memory persistence needs a default local-first backend but should leave room for
alternatives (for example, a vector-search-capable store) without rewriting the
memory layer.

## Decision

Express storage as Rust traits: `Stream` and `Store`. SQLite (`rusqlite`) is the
default implementation. DuckDB (`duckdb-rs`) is an optional feature for vector
search.

## Consequences

- Trait objects add a vtable indirection.
- Negligible compared to LLM latency.

> Note: ADR-0024 records the broader trait-object-at-every-seam convention and
> the async-trait mechanism that this decision is an instance of.
