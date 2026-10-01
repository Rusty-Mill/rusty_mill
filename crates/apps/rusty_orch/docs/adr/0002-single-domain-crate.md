# ADR-0002: One domain crate with three aggregates

- **Status:** Accepted
- **Date:** 2026-10-01

## Context
Goals, task cards, and the blackboard share primitives (`Text`, `Ref`, id newtypes). An earlier standalone `goal-core` crate would have forced either duplicating those or a third crate for primitives only.

## Decision
Keep `goal`, `task`, and `board` as modules of one zero-dependency crate, `orch-core`. `Plan` and `Board` are separate aggregates and do not reference each other; cross-checks (e.g. a board entry's task id exists in the plan) belong to the application layer.

## Consequences
- One place for the domain vocabulary; no speculative crate boundaries.
- Split later only for a concrete forcing function, per the repo's architecture defaults.
