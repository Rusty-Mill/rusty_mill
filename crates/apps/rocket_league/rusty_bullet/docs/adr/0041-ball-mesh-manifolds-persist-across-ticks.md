# ADR-0041: The ball's mesh manifolds persist across ticks

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-121, FR-105, FR-115, FR-116, FR-118; ADR-0024, ADR-0035, ADR-0036, ADR-0038
- Supersedes/Superseded by: —

## Context

`hitjump.jsonl` 97.900 s: one tick after bouncing out of a goal corner,
the simulator bounced the ball again (207 uu/s off in one-step) while the
game waited one more tick. The ball's mesh contacts were rebuilt from
nothing every tick (FR-115), so the second tick saw only the back-slope
facets the ball was still inside, all approaching.

Bullet keeps a `btPersistentManifold` per shape pair across ticks. Each
tick its points are matched and sorted (by their ball-frame position,
`m_localPointA`) as new triangles report, then `refreshContactPoints`
moves each point's ball side with the ball's transform, drops it once it
is past the breaking threshold along its normal or has slid past the
threshold sideways, and updates its depth. The car's plane and mesh
contacts already work this way (FR-105, ADR-0024); the ball's did not.

Two RocketSim details settled on the way:
- `btCollisionShape::getBoundingSphere` pads a sphere's radius by 0.08 BT
  (4 uu), so the ball's breaking threshold is 0.02 x 95.25 = 1.905 uu:
  the 1.9 uu band FR-118 measured.
- RocketSim's `getCacheEntry` returns -1, never folding a new point into
  a cached one ("makes ball-arena collisions far less accurate" with it
  on). Tried here: one-step ball 0.549 (was 0.500), the crossbar frame
  back at 711 uu/s. The recording prefers FR-115's folding, so it stays.

## Decision

- `mesh::BallManifold` keeps `ManifoldEntry` points, each with its
  ball-frame position and its point on the mesh. `StaticMesh::sphere_manifold`
  adds this tick's triangle contacts to it (matching and sorting by the
  ball-frame position) and refreshes it as Bullet does; `sphere_contacts`
  is the same call on an empty manifold.
- `PhysicsWorld` carries one `BallManifold` per mesh, sized in `step`,
  cleared by `snap_to_frame`: a snapped ball has no contact history.
- The folding of near points (FR-115) and the 1.86 uu threshold
  (`CONTACT_BREAKING_FACTOR` x radius) are unchanged.

## Consequences

- `hitjump` k = 30: the 97.908 s window 206 uu/s to 4, 97.917 s 76 to
  11, the 39.9–40.1 s corner sequence 274/261/233/204 to 2/7/2/4. Mean
  20.35 (was 20.37); median, p90 and p99 unchanged; 139 windows improved
  by more than 5 uu/s and 160 worsened, the largest losses one-tick
  bounce-timing flips (20.783 s 119 to 438, 40.050 s 70 to 267).
- `test2` k = 30 ball 0.33, unchanged. One-step errors are unchanged by
  construction: each one-step tick starts from a snapped, history-free
  ball. The 97.908 s one-step frame therefore still reads 207.
- A spinning ball's kept points turn with it and leave the threshold
  within a tick at a few rad/s, so persistence mostly matters to a ball
  sliding or settling against a surface.
