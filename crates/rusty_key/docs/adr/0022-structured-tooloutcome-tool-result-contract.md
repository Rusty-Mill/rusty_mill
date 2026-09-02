# ADR-0022: Structured `ToolOutcome` tool-result contract

- Status: Accepted
- Date: 2026-05-27
- Tags: tools, error-model, observe

## Context

PRD 04 infers `ToolStatus` from the tool result string by matching magic
prefixes: `BLOCKED ...` -> `Blocked`, `ERROR ...` / `TIMEOUT ...` -> `Error`,
everything else -> `Ok`. This prefix-sniffing is fragile: any tool whose
legitimate output begins with one of those words is misclassified, and status is
not carried structurally (consolidated plan §D). The `ToolResultClassifier` was a
deferred seam.

## Decision

Introduce one structured `ToolOutcome` type that carries the status structurally,
plus a single formatter/parser pair that renders it to and from the model-facing
string. This replaces the magic-prefix inference and pulls the
`ToolResultClassifier` seam forward into v1. The `ToolOutcome` type and its serde
encoding live in `docs/architecture/data-model.md`; the contract is detailed in
`docs/dev/error-handling.md`.

## Consequences

- Tool status is authoritative and structural, not inferred from text.
- Tracer / observe consumes `ToolOutcome.status` directly instead of re-parsing
  the result string.
- One formatter/parser is the single place that defines the model-facing
  rendering, removing the duplicated prefix conventions.
