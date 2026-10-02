# ADR-0021: The car body uses RocketSim's car-vs-world friction and bounce

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-100, FR-081, FR-063, FR-090; ADR-0012,
  ADR-0018, ADR-0020
- Supersedes/Superseded by: supersedes FR-081's frictionless car-floor
  contact

## Context

FR-081 made a car body's floor contact frictionless because the box then
rested on the floor and its Coulomb friction (0.5 combined) fought the tire
model. Since FR-090 the car rides on its suspension with the box clear of
the floor, so the body only touches the world in impacts. Every car-world
contact combined the car's placeholder 0.5 with the surface's coefficients,
except the floor, which had no friction.

The owner's front-flip capture (`front.jsonl`) lands nose first and its
body hits the floor at 19.083 s: the recorded car loses 28 uu/s of forward
speed in that tick; the candidate lost 2 (its brake). RocketSim overrides
every car-vs-world manifold with `CARWORLD_COLLISION_FRICTION = 0.3` and
`CARWORLD_COLLISION_RESTITUTION = 0.3` (`Arena.cpp`), whatever the surfaces.

## Decision

- Static manifolds carry a `solver::StaticMaterial`: `Surface` (the
  shape's coefficients, combined with the body's, as before) or `Pair`
  (coefficients fixed for the pair, used as they are).
- Every car-vs-static manifold, floor included, uses
  `Pair { restitution: 0.3, friction: 0.3 }` (`CAR_WORLD_MATERIAL`).
- The ball keeps its surface combine; car-car and car-ball overrides stay
  open (FR-063).

## Consequences

- A car body sliding or landing on the floor, walls or curves slows under
  0.3 friction and bounces at 0.3, as in RocketSim.
- Grip on four wheels is unchanged: the body is clear of the floor.
- The roll and yaw the recorded car picks up at 19.083 s may also need the
  real arena's triangle-mesh floor; this decision does not model that.

Real capture, 2026-10-02: confirmed as an improvement. Velocity error after
the 19.083 s impact 13-27 uu/s (was 35-78); `--self-growth` windows 19/20 s
9.9/17.3 uu (were 11.3/26.8). The impact tick itself now overshoots
(forward -45 vs -28.5 uu/s, vertical +175 vs +120): the candidate's car
hits flat on two corners while the recorded one rolls and yaws.
