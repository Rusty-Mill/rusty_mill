# ADR-0014: Intervention Logger + M-HIR in observe layer

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: observe, metrics, mhir

## Context

The harness needs a signal for whether humans are compensating for capabilities
it lacks. Human interventions — task overrides, manual consolidations,
unverified followups — are observable evidence of harness gaps and should be
recorded and trended.

## Decision

Record human interventions to `.rustykeys/interventions.jsonl` and compute M-HIR
(Missing-Harness Human Intervention Rate) as `interventions / total_turns`. A
rising rate signals harness gaps; a falling rate signals improvement.

## Consequences

- Provides a quantitative, trendable signal of harness maturity over time.
- The metric's meaning depends on which actions count as interventions; see
  ADR-0019, which adds `avoidability` / `harness_gap` / `burden` so M-HIR counts
  missing-harness interventions specifically.
