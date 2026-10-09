# ADR-0079: The ground boost acceleration applies while any wheel touches

- Status: Accepted
- Date: 2026-10-08
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-157`, FR-056, FR-156, ADR-0078

## Context

`regime_bias.py` groups the one-step velocity error of all fuzz-drive ticks (96 tapes, collisions
and jumps removed) by wheels touching, throttle sign, boost, handbrake and steer, in the car's
frame. After ADR-0078 the largest non-resting biases were the boosted one- and two-wheel regimes,
0.5 to 0.7 uu/s per tick short forward in the game against the port whatever the throttle:
66.7 uu/s^2 is the gap between the airborne (3175/3) and ground (2975/3) boost accelerations.
The port chose the airborne value unless three wheels were down.

## Decision

`apply_boost` takes "any wheel touching" instead of "on the ground (three or more wheels)".

## Consequences

- The boosted 1-2 wheel regimes' forward error drops to -0.12 .. +0.02 uu/s per tick.
- Random-drive means move +2% (113 -> 114, 102 -> 105 uu), nine tapes better and eight worse; the
  sums follow two landings. The mechanism is checked at one-step level, the means are event noise.
- Golden gate unchanged.
