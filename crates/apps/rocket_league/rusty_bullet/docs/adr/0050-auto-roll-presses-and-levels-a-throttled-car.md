# ADR-0050: Auto-roll presses and levels a throttled car

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-131, FR-094
- Supersedes/Superseded by: —

## Context

`test2.jsonl` 8.967-9.042 s: a car landing with throttle on, on one to three
wheels. The one-step spin error was 0.25-0.43 rad/s per tick, always toward
lying flat. RocketSim's `Car::_UpdateAutoRoll` does exactly this, and the
port had no equivalent.

## Decision

- `drive::roll::auto_roll`, called last in `apply_driven_forces`: when
  throttle is non-zero and one to three wheels (or the body alone) touch a
  surface, apply 100 uu/s^2 toward the surface and up to 80 rad/s^2 of spin
  toward flat, scaled per axis by `1 - clamp(alignment)`.

Alternatives considered:

- Fit a damping term to the spin error. Rejected: the mechanic exists and is
  known; a fit would hide it.

## Consequences

- `test2` spin error at 8.967-9.042 s falls to <= 0.03 rad/s; k = 30 car
  velocity 5.44 -> 4.73. `front` k = 30 car 0.80 -> 0.49. `hitjump` and
  `side` unchanged.
- Frames near 8.99-9.0 s regress slightly, and the 5.8-5.9 s post-jump z
  bias (~5 uu/s per tick) remains unexplained.
