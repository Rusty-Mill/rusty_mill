# ADR-0001: Rust as implementation language

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: language, runtime, correctness

## Context

The harness is a runtime substrate that mediates every model action, enforces
policy on every tool call, and persists every observation. These responsibilities
are execution-bound (fast, local), not LLM-bound (slow, network I/O). The
implementation language should make resource management, thread safety, and
compile-time correctness the defaults so the constrain layer is provably correct.

## Decision

Implement Rusty Keys in Rust. Rely on its ownership model to enforce correct
resource handling at compile time, tokio for native async I/O on the LLM calls,
and zero-cost abstractions so the substrate adds no overhead in the hot path.

## Consequences

- Higher barrier to entry and longer compile times than Python.
- Justified by the compile-time correctness guarantees and the performance
  headroom Rust leaves for future scale.
