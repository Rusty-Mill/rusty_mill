# ADR-0024: Trait objects at all plugin seams + async-trait mechanism + MSRV pin

- Status: Accepted
- Date: 2026-05-27
- Tags: standards, traits, async

## Context

ADR-0010 chose trait objects for storage and accepted the vtable cost as
negligible against LLM latency. The same question recurs at every plugin seam
(`Policy`, `Check`, `SecurityCheck`, `ToolFn`, `SessionFactory`, `Stream`,
`Store`): generics-with-monomorphization versus `dyn` trait objects. The spec
needs one convention, plus a decision on how async appears in those traits and a
pinned minimum supported Rust version (consolidated plan §D).

## Decision

Use trait objects (`Box<dyn Trait>` / `&dyn Trait`) at all plugin seams in v1;
no generic-parameterised seams. Use the chosen async-trait mechanism for async
methods on those traits, and pin an MSRV. This records ADR-0010 concretely and
extends it to every seam. The async-trait mechanism, MSRV value, and
trait-object-vs-generics rationale live in `docs/dev/coding-standards.md`.

## Consequences

- One uniform plugin convention; seams are object-safe by construction.
- A vtable indirection per seam call, accepted as negligible relative to LLM and
  I/O latency (consistent with ADR-0010).
- Generics remain available for internal hot-path code that is not a plugin seam.
