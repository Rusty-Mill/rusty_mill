# ADR-0008: Resolve every contact manifold in a step in one combined solve

- Status: Accepted (recorded retroactively 2026-09-30; decided 2026-08-31 to 2026-09-01)
- Date: 2026-08-31
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-030, FR-034, FR-035, FR-041, FR-050, FR-051,
  FR-052; PRs #69, #79, #89, #107, #109, #111; ADR-0004
- Supersedes/Superseded by: none

## Context

Before FR-030, `step` resolved each ball-vs-car and car-vs-car pair with
its own `resolve_contacts_between` call: a full 10-iteration pass, applied
before the next pair was set up. A body touching two others could lose
almost all of the first contact's effect.

In the symmetric-pinch test (a mass-1 ball between two mass-180 cars
closing at 100 units/s; the correct result is about 0), the pairwise solve
left the ball at ~98.9 units/s. More iterations did not change that, so
the error was structural.

FR-051 and FR-052 then found the same order-dependence one level up.
Resolving static shapes one after another, and then running static and
dynamic contacts as two separate calls, both produced mirror-image bias in
a corner.

## Decision drivers

- Bullet's `btSequentialImpulseConstraintSolver`, which ADR-0004 ports,
  iterates all constraints together. Pairwise resolution was a departure
  from it, not a feature of it.
- The pairwise error could not be reduced by spending more compute.
- Results must not depend on contact enumeration order, or determinism
  (`PHASE-2-DETERMINISM`) and symmetric scenarios both suffer.

## Considered options

1. **One combined solve** (chosen): every static and dynamic manifold
   shares one per-body velocity/push accumulator across a single
   iteration loop.
2. **Pairwise resolution**: rejected as structurally lossy (~98.9 units/s
   in the pinch test).
3. **Static first, then dynamic**: rejected in FR-052 as order-dependent.
4. **Raise `SOLVER_ITERATIONS`**: not adopted. It is a real per-step cost
   with no data showing it is needed.
5. **Global SOR over-relaxation (omega > 1)**: rejected in FR-041, because
   it diverged. A parameter-free 1/k relaxation on dynamic rows was
   adopted instead.

## Decision

`solver::resolve_manifolds(bodies, static_manifolds, dynamic_manifolds,
dt, caches)` is called once per `step`. It runs `SOLVER_ITERATIONS = 10`
over all manifolds with a shared accumulator per body index, together
with:
- split impulse as a separate push channel (FR-034);
- warm-starting of dynamic manifolds only (FR-035);
- the 1/k relaxation on dynamic rows (FR-041).

## Consequences

### Positive

- The pinch result went from ~98.9 (pairwise) to ~89.5 (combined) to ~32
  units/s (combined plus 1/k relaxation), at no extra iteration cost.
- The corner-bias cases in FR-051 and FR-052 are symmetric.
- The net-point case (FR-050) dropped from a ~0.25 to a ~0.016 units/s
  lateral bias.

### Negative / tradeoffs

- The sandwiched case still does not fully converge at 10 iterations.
- Static contacts are not warm-started and get no relaxation: extending
  1/k to static rows regressed FR-051's test.
- Nets are resolved after `resolve_manifolds`, outside the shared solve
  (see ADR-0010).
- Validated only on synthetic scenarios; no real recorded multi-car
  contact data yet.

## Validation and revisit triggers

- Revisit iteration count and relaxation once FR-005 calibration or
  `--self-growth` attributes divergence to multi-body contacts.
- Revisit if `PHASE-2-DETERMINISM` finds order-dependence elsewhere in
  the step.
