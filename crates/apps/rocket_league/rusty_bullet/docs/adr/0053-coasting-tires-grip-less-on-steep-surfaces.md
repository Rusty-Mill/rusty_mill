# ADR-0053: Coasting tires grip less on steep surfaces

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-134, FR-080, FR-090
- Supersedes/Superseded by: —

## Context

Reading RocketSim's `Car::_UpdateWheels` against the port, `isContactSticky`
(`realThrottle != 0`, boost forcing throttle to 1) selects whether a wheel's
friction keeps its value or is scaled by `NON_STICKY_FRICTION_FACTOR_CURVE`
of the contact normal's z. The port had no such scale.

## Decision

- `ground::NON_STICKY_FRICTION_CURVE` = (0, 0.1), (0.7075, 0.5), (1, 1).
  `wheel_impulses` multiplies each wheel's side grip and braking by it when
  the effective throttle is zero.

Alternatives considered: scaling the engine force too. Rejected: engine
force is zero without throttle.

## Consequences

- `hitjump` k = 30 car 10.82 -> 10.52; the other captures unchanged. Level
  ground is unaffected (factor 1 at z = 1).
