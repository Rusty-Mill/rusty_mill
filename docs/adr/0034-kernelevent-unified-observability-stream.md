# ADR-0034: `KernelEvent` as the unified observability stream

- Status: Accepted
- Date: 2026-05-27
- Tags: observe, observability, otel, standards

## Context

The `on_event` hook and the OTel export seam were specified independently, which
risked two divergent event vocabularies — one for the in-process `Tracer`, one
for telemetry. Round 2 (consolidated §ADAPT.6/7, CloudWeGo Eino's callback
aspects) recommends a single fixed event type pinned to the hook, plus a
pull-based exporter so a future `sandboxed` profile (ADR-0030) does not blind
operators the way a VM can blind an EDR agent.

## Decision

Define a single, fixed **`KernelEvent`** lifecycle enum that the `on_event` hook
emits. It is consumed by **both** the `Tracer` and a **pull-based OTLP
exporter** carrying token/cost/latency attributes. One enum, one stream, two
consumers — there is no second event vocabulary. The exporter is pull-based so
isolation profiles cannot starve telemetry. Detail:
`docs/dev/coding-standards.md`, `docs/ARCHITECTURE.md`.

## Consequences

- `KernelEvent` is a fixed enum (variant set governed like any wire contract);
  adding a lifecycle point means adding a variant, keeping consumers in lockstep.
- The `Tracer` and the OTLP exporter share one source of truth, so traces and
  telemetry cannot drift apart.
- Pull-based export means a `sandboxed` ToolExecutor (ADR-0030) does not blind
  operators — telemetry is collected at the boundary, not pushed from inside.
- The enum carries token/cost/latency attributes, making cost observability a
  first-class property of the event stream rather than a bolt-on.
