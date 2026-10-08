# ADR-0002: Verification pipeline precedes physics implementation

- Status: Accepted
- Date: 2026-08-28
- Deciders: baileyrd
- Related: RB-VERIFY-001, RB-VERIFY-002, RB-VERIFY-003, ROADMAP.md
- Supersedes/Superseded by: none

## Context

The staged plan is physics core → deterministic simulation → netcode →
polish, roughly 6-9 months part-time for the first three phases. Before any
of that starts, the project needs a way to know whether a candidate
physics implementation is actually accurate — otherwise months of physics
work could be tuned against nothing, or against a wrong intuition about
what "feels right."

Two partial ground-truth sources exist (replay files via `boxcars`;
BakkesMod captures, offline-only since Easy Anti-Cheat now blocks
BakkesMod online), and neither alone is sufficient, nor can either be
skipped in favor of "just building the physics and eyeballing it."

## Decision drivers

- A physics/simulation implementation without a way to measure its
  accuracy against reality is unfalsifiable — any amount of tuning effort
  could be spent without knowing if it's converging or diverging.
- The verification gap (no single source of real-online-input + physics
  ground truth) is a known, documented constraint from the start — better
  to build the pipeline that works around it before physics work begins
  than to discover the gap mid-Phase-1 and have to retrofit measurement.
- Building the pipeline first is cheap relative to the 6-9 month estimate
  for Phases 1-3, and de-risks the rest of the project.

## Considered options

1. **Verification pipeline first (Phase 0)**, before any physics
   implementation. Ingestion adapters + divergence scorer built and
   exercised (even against two recorded trajectories, before any candidate
   engine exists) before Phase 1 starts.
2. **Physics first, verification bolted on later.** Faster to see visible
   progress (a car that moves), but risks tuning against intuition rather
   than measurement, and risks discovering late that the available ground
   truth doesn't actually support the kind of validation assumed.
3. **Skip formal verification, rely on manual feel-testing.** Fastest
   short-term, but defeats the project's own stated method (deconstruct to
   find where flaws actually live, rather than guess) and provides no way
   to know if Phase 1 is actually converging on Rocket League's real
   physics behavior.

## Decision

Adopt option 1. Phase 0 (verification pipeline: replay ingestion +
BakkesMod capture ingestion + divergence scoring) is the actual starting
point, gating Phase 1.

## Consequences

### Positive

- Every later phase has an objective, falsifiable accuracy metric to tune
  against, tracked in `RB-VERIFY-003`.
- The verification-gap constraint (documented in the research backlog) is
  designed around explicitly from day one, not discovered as a surprise.

### Negative / tradeoffs

- Delays visible physics/gameplay progress by however long Phase 0 takes,
  which may read as slow progress even though it's not wasted time.
- The divergence metric itself is currently ball-position-only
  (`RB-VERIFY-003-FR-001`); car-state and alignment extensions
  (`RB-VERIFY-003-FR-002`/`FR-003`) are still open, so Phase 0's initial
  exit criterion is a floor, not a complete metric.

## Validation and revisit triggers

Revisit if Phase 0 proves genuinely unbuildable with the two available
ground-truth sources (not currently expected — both sources are confirmed
accessible). Exit criterion: pipeline runs end-to-end and produces a
divergence score on at least one real replay and one BakkesMod capture
(see ROADMAP.md Phase 0).
