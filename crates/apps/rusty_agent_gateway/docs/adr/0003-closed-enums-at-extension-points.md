# ADR-0003: Closed enums at extension points, not trait objects

Status: Accepted
Date: 2026-08-08

## Context

This gateway has several places where one of N things happens: a route's backend
is a `host`, `service`, `mcp`, `ai` or `dynamic`; an LLM route speaks OpenAI,
Anthropic or Gemini; a listener terminates TLS, forwards it, or does neither.

The conventional Rust answer — and the ports-and-adapters default this
organisation starts from — is a trait per extension point with one implementation
per variant. This repo has no such traits. That is a deliberate choice rather
than an oversight, and it was being inferred as an oversight by anyone reading
the crate layout, which is why it is written down.

## Decision

Every extension point is a closed enum, matched exhaustively at the seam.
`BackendTarget` (configuration) becomes `BackendState` (runtime); `Provider`
selects the LLM translation; `Protocol` plus `Listener::passes_through()`
decides what a listener does with a connection.

Critically, **no catch-all match arm**. `gateway.rs` handles every variant by
name. When the fifth and final backend kind was implemented, the `Some(other) =>
Unsupported(...)` arm became unreachable and was deleted rather than left as
defensive padding.

## Alternatives considered

**A trait per extension point (`trait Backend`, `trait Provider`).** Lets a
variant live outside the crate that dispatches it, and reads as more idiomatic.
Rejected on the failure mode: adding a variant would compile fine while silently
falling through to whatever the default arm did. For a gateway, the default arm's
behaviour is *routing traffic somewhere nobody asked for* — a 501 to a client, or
worse, a request quietly sent to the wrong upstream. A compile error is strictly
better feedback than a runtime fallback here.

**Enums with a catch-all arm.** The middle ground, and the status quo until the
backend work finished. It has the same defect in slower motion: the arm exists to
be hit, so a missing variant becomes a runtime surprise instead of a build
failure.

## Consequences

- Adding a backend kind, provider, or protocol means touching every match site.
  That is the point — the compiler produces the checklist.
- Nothing outside this workspace can add a variant. Acceptable: this is an
  application, not a plugin host, and no such extension has been asked for.
- The crate graph carries the modularity a trait would otherwise provide.
  `agentgateway-config` depends on nothing; `agentgateway-core` holds what
  several crates share; protocol crates depend on those two and not on each
  other; `agentgateway` is the only crate that knows about all of them.
- Shared code moves *down* into `-core`, never sideways between protocol crates.
  `Retry` and `Endpoints` both made that move when a second caller appeared.
- If a genuine out-of-tree extension point is ever needed, this is reversible for
  that one seam without disturbing the others.
