# ADR-0073: A wheel-ray instrument for the wall work (`--wheel-trace`)

- Status: Accepted
- Date: 2026-10-07
- Deciders: baileyrd
- Related: `RB-RESEARCH-O012`, `RB-RESEARCH-O019`, `docs/roadmap/PARITY-PLAN.md` (workstream A, step 1), `RB-VERIFY-003`
- Supersedes/Superseded by: none

## Context

Wall and ramp climbing is the largest error left, and every fix tried so far was a
constant tuned against recordings (O012, O019). The game gives only whole-car state
per tick, so the question "which part of the wheel model is wrong at a facet
crossing" had no tool: the port's wheel hits were invisible outside `step`.

## Decision drivers

- Do not change any simulation behaviour or any existing score.
- Use the one-step prediction already trusted for the contact work (`snap_to_frame`
  per tick), so a row's error is that tick's model error alone.
- No new dependency.

## Considered options

1. Debug prints inside `drive::cast_wheels` (not reusable, noisy in tests).
2. A read-only `PhysicsWorld::wheel_contacts` plus a trace over a recording.
3. Record the triangle index in `RayHit` (changes the ray type for every caller;
   hit normals already tell neighbouring facets apart on the ramp).

## Decision

Option 2. `PhysicsWorld::wheel_contacts(car, dt)` casts the four wheel rays as `step`
does and changes nothing; `world::wheel_contacts_along` does it along a recording;
`WheelContact` gains `suspension_length()` and `pushback()` getters.
`rb-verify --wheel-trace <capture> <from> <to>` prints, per tick, the recorded input,
the one-step velocity and spin error and each wheel's hit normal and suspension
length, with `FACET` where any wheel's normal changed. `wheel_facets.py` summarises
a batch.

Addendum (same day): `rb-verify --wheel-kstep <capture> [k]` prints the k-step
(default 30) position and velocity error of each frame with the front and rear
wheel facet changes inside its window, and `wheel_kstep.py` buckets a batch by that
count. A steady force bias is absorbed by the suspension inside a window, which the
one-step view cannot show, so this is the metric for facet hypotheses.

## Consequences

### Positive

- A velocity error can be lined up with the facet each wheel stood on.
- First result (O012): the per-tick error on a climb is small and one-sided.

### Negative / tradeoffs

- The facet is identified by its normal, not a triangle id; two facets with the
  same normal look alike (harmless for a smooth ramp).
- The trace reads the car in the recorded state, so it cannot show what a bias does
  over a free run; use `--scenario ... --against` for that.

## Validation and revisit triggers

- `world` tests: four floor hits with the floor normal, none in the air, one set of hits per recorded step.
- Revisit if a triangle id is needed to separate facets with equal normals.
