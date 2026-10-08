# ADR-0024: A car meets planes through Bullet's one-corner-a-tick persistent manifold

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-105, FR-047, FR-100; ADR-0008, ADR-0021
- Supersedes/Superseded by: supersedes FR-047's decision not to adopt
  Bullet's single-vertex contact for box versus plane

## Context

`front.jsonl` lands nose first twice (19.083 s, 25.967 s). Both times the
recorded car leaves the impact with large yaw and roll (25.967 s: spin
(-4.32, 0.12, -3.40) rad/s from (-0.29, 3.80, 0)). The candidate's car
bounced off flat, because `box_vs_plane` contacts every penetrating
corner at once and the impulses balance.

RocketSim's floor, ceiling and side walls are `btStaticPlaneShape`s, and
its car is a box in a `btCompoundShape`. Bullet's
`btConvexPlaneCollisionAlgorithm` adds one contact per tick: the box's
support vertex along the plane normal, picked by sign on each axis. Its
multi-point perturbation is off by default. The `btPersistentManifold`
keeps earlier points until they separate or slide past the breaking
threshold (0.02 times the shape's angular motion disc, about 2 uu for the
car), up to 4. FR-047 found this and chose all corners at once as "more
rigorous".

## Decision

- `collision::PlaneManifold` holds a box's contact with one plane: each
  tick it adds the support corner (or refreshes it), drops stored corners
  that lift or slide more than the breaking threshold, and returns the
  stored corners that touch. When full it replaces the oldest, a simpler
  stand-in for Bullet's area-maximizing replacement.
- `PhysicsWorld` keeps one per car for the ground and each wall. The ball
  (a sphere, one support point anyway) and the windowed and bounded goal
  walls keep their stateless queries.

## Consequences

- `front.jsonl` one-step error: 19.083 s 30 uu/s (was 55), spin 1.1
  (was 3.8); 25.967 s 156 (was 252), spin 2.0 (was 5.8), with the yaw
  sign right; mean 0.407 (was 0.457). `test2`/`side` unchanged.
- A bare box sliding flat at high speed rides on one corner and can trip;
  a 1 kg slab at 2000 uu/s launched in a test, which now uses a real car
  on its wheels. A real car sliding on its roof at up to 2000 uu/s stays
  level (spin under 0.5 rad/s).
- Persistence did not change the impact ticks in the captures, but it is
  what lets a box lying flat gather its corners over a few ticks.
- Still open: the remaining error on the second impact tick (25.975 s,
  96 uu/s), and curves and corner walls, which RocketSim meshes as
  triangles (`btConvexTriangleCallback`), not planes.
