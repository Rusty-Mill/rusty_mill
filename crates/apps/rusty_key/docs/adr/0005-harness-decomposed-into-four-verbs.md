# ADR-0005: Harness decomposed into constrain / feed / observe / compose

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: architecture, decomposition, modularity

## Context

The harness wraps the kernel with several cross-cutting concerns. Each concern
needs one obvious home so it has a stable place to grow, mirroring the
decomposition that worked in Keystone.

## Decision

Decompose the harness into the same four verbs as Keystone — constrain, feed,
observe, compose. Each verb maps to one module, and every cross-cutting concern
has a stable home in exactly one of them.

## Consequences

- The modules are thin at phase 1.
- Accepted as intentional placeholders with documented seams that earn their
  place as the system grows.
