# ADR-0028: H0 is a selectable harness level or explicitly evaluation-only

- Status: Proposed
- Date: 2026-05-27
- Tags: faithfulness, eval, maturity

## Context

The maturity model spans H0-H3, where H0 is "task description, repository files"
with no tool registry — the paper's ablation floor. But `RUSTYKEYS_HARNESS_LEVEL`
today accepts only `h1` / `h2` / `h3` (PRD 06), so H0 is unreachable at runtime.
Without it the paper's H0 ablation cannot be measured (consolidated plan §G).

## Decision

Either make H0 a selectable harness level (a runtime mode with no tool registry)
or declare it explicitly evaluation-only. Resolve which in
`docs/dev/eval-plan.md`; if selectable, `RUSTYKEYS_HARNESS_LEVEL` must accept
`h0` and the configuration reference (`docs/reference/configuration.md`) and the
H0-H3 progression gates must reflect it.

## Consequences

- The H0 ablation floor becomes reachable (or its eval-only status is documented),
  so H0-vs-H1+ comparisons are well-defined.
- Status is Proposed: this is an open product decision (consolidated plan "Open
  product decisions") — the owner sets whether H0 is a runtime mode or eval-only.
- If runtime-selectable, the agent loop must run with no tool registry at H0,
  which the kernel and `Session` construction must support.
