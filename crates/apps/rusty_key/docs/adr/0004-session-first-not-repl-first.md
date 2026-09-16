# ADR-0004: Session-first, not REPL-first

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: architecture, session, transport

## Context

In Keystone the UI loop and the AI loop were interleaved in `main()`, and the
split between them was only aspirational — a comment in `build_kernel()` promised
gateway reuse that the structure never delivered. The AI loop should be a
first-class object so any transport (CLI, web, desktop, API) can reuse it.

## Decision

Make `Session` the first-class object. `Session::send()` owns the full turn cycle
(observe -> orient -> kernel -> compose) and is transport-agnostic. The CLI, web
gateway, and other gateways are thin adapters over the same `Session`.

## Consequences

- Slightly more structure upfront than an interleaved REPL.
- Justified by the gateway reuse Keystone's `build_kernel()` comment already
  promised but never realised.
