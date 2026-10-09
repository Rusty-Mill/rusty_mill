# ADR-0003: Target Bullet-derived physics fidelity, defer engine choice

- Status: Accepted
- Date: 2026-08-28
- Deciders: baileyrd
- Related: RB-PHYSICS-001, CHARTER.md, docs/research/RESEARCH-BACKLOG.md
- Supersedes/Superseded by: none

## Context

Rocket League runs on a modified Bullet Physics integration inside Unreal
Engine 3, entirely client-side (confirmed by offline play requiring no
network connection). This is Psyonix's actual, confirmed physics
foundation — not a guess.

Separately, there's an open question this ADR does **not** resolve: whether
this project's Phase 1 physics core should be hand-rolled from scratch or
built on an existing engine (e.g. Rapier). That choice is deliberately left
open until the Phase 0 verification pipeline exists and can score
candidates — see `docs/research/RESEARCH-BACKLOG.md`. This ADR only settles
what the target *behavior* is, not how it gets implemented.

## Decision drivers

- Confirmed research (Psyonix's own architecture, corroborated by
  offline-play behavior) makes the fidelity target a settled fact, distinct
  from the unsettled implementation question.
- Recording this now prevents Phase 1 design from silently drifting toward
  "whatever feels right" instead of "matches Rocket League's actual car/
  ball dynamics," regardless of which engine ends up implementing it.
- No public Psyonix fork of Bullet exists to copy or reference directly
  (checked fork network and issue tracker, nothing found) — the fidelity
  target has to be validated behaviorally (via the divergence metric),
  not by comparing source code.

## Considered options

1. **Target Bullet-derived fidelity as the reference, defer the
   build-vs-integrate implementation choice.** Settle what "correct"
   means (matching Bullet-based car/ball dynamics as measured by
   divergence scoring); leave how to achieve it open.
2. **Target fidelity to Bullet *and* commit now to reimplementing/forking
   Bullet itself.** Rejected: forecloses the build-vs-integrate question
   before the verification pipeline exists to actually inform it, and
   Bullet upstream is effectively unmaintained since v3.2.4 (Apr 2022),
   which is itself a relevant input to that later decision.
3. **Don't commit to any specific fidelity target; let Phase 1 discover
   what "feels right" empirically.** Rejected: unfalsifiable, and
   contradicts the verification-first ordering in ADR-0002 — there has to
   be a defined target for the divergence metric to measure distance from.

## Decision

Adopt option 1. Phase 1's physics core targets reproducing the physical
behavior of Rocket League's Bullet-based car/ball dynamics (aerodynamics,
suspension, ball bounce/spin, boost, etc.), measured via the `RB-VERIFY-003`
divergence metric against recorded ground truth. Whether that target is met
by a from-scratch implementation, an integrated engine tuned to match, or
something else is explicitly not decided by this ADR.

## Consequences

### Positive

- Gives Phase 1 (and `RB-PHYSICS-001`) a concrete, falsifiable target
  instead of a vague "make it feel like Rocket League."
- Keeps the build-vs-integrate decision honestly open until it can be made
  with real evidence (divergence scores from actual candidates), rather
  than argued from priors.

### Negative / tradeoffs

- Some Phase 1 design work (e.g. what physical phenomena RB-PHYSICS-001
  needs to model) has to proceed without knowing the final engine choice,
  which may mean some rework once that choice is made.
- "Matches Bullet-derived behavior" is itself only measurable indirectly
  (via replay/capture divergence, not by inspecting Psyonix's actual Bullet
  fork, which isn't available) — see the verification-gap discussion in
  `docs/research/RESEARCH-BACKLOG.md`.

## Validation and revisit triggers

Revisit the build-vs-integrate question (not this ADR's core decision) once
the Phase 0 pipeline exists and can score at least one candidate of each
approach — tracked as an open item in
`docs/research/RESEARCH-BACKLOG.md`. This ADR's fidelity-target decision
itself is revisited only if evidence emerges that Rocket League's actual
physics diverged meaningfully from stock Bullet behavior in ways that
change what "the target" should be.
