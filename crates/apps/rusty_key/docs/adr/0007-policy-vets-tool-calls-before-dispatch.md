# ADR-0007: Policy vets tool calls before dispatch; errors returned, not panicked

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: constrain, policy, safety

## Context

Once an agent has tools, a wrong inference becomes a destructive action. The
harness needs an enforcement point that runs before any side effect, and a
policy violation must never crash the process — the model should be able to
recover from a rejection.

## Decision

Run `Policy::before_tool()` before the aisdk dispatcher. Violations are returned
as `Err(PolicyError)` and surfaced to the model as a `BLOCKED` string so the
model can recover rather than the process crashing.

## Consequences

- The model sees error text, which adds to the prompt surface.
- Acceptable, given recovery is preferable to a crash.

> Note: ADR-0016 later changes `before_tool` to an `async fn`; ADR-0023 records
> the no-panic rule this decision implies and backs it with clippy lints.
