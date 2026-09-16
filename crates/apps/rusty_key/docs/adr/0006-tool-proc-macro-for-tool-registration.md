# ADR-0006: `#[tool]` proc macro for tool registration

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: tools, macro, ergonomics

## Context

Keystone's `Tool` dataclass required manual JSON schema authorship for every
tool, which was error-prone and decoupled from the function signature it
described. Tool signatures should be type-safe at compile time and their schema
should be derived, not hand-written.

## Decision

Register tools as Rust functions annotated with the `#[tool]` proc macro; aisdk
generates the JSON schema from the function signature. This eliminates the manual
schema authorship the Keystone `Tool` dataclass required.

## Consequences

- Proc macros add compile-time complexity.
- aisdk owns that cost, not the harness.
