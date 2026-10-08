# ADR-0006: Model the standard arena from analytic primitives, not a triangle mesh

- Status: Accepted (recorded retroactively 2026-09-30; decided 2026-08-30 to 2026-08-31)
- Date: 2026-08-30
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-013, FR-019 to FR-026, FR-029, FR-031, FR-036,
  FR-040, FR-055; PRs #47 to #61, #67, #87; ADR-0003, ADR-0004
- Supersedes/Superseded by: none

## Context

Rocket League's real arena collision is a triangulated mesh; the community
has dumped it (`ZealanL/RLArenaCollisionDumper`), and RocketSim loads it.
When `PhysicsWorld::standard_arena` was built (FR-019), the port only had
FR-013's generic flat `StaticPlane`. Each later increment added one more
analytic shape: `StaticQuarterPipe` edge fillets (FR-020 to FR-022),
`StaticCornerFillet` compound corners (FR-023), `StaticGoalWall` with a
goal-mouth window (FR-024), and `StaticBoundedWall` goal interiors
(FR-029).

The mesh was not weighed and rejected when these shapes were chosen. It
shows up afterwards, in FR-031, FR-036 and FR-040, as what would be needed
to calibrate the fillet radii. FR-040 found that ingesting it needs the
owner's own Windows/Rocket League environment, the same blocker as
`RB-VERIFY-002-FR-001`. This ADR records that fork and the reasons the
primitives stay.

## Decision drivers

- Every primitive reuses the existing sphere/box-vs-plane and
  sphere-vs-shape narrow phase; each increment added no new collision
  framework (FR-019: "no new collision code").
- Every boundary constant the community has published (half-width 4096,
  half-length 5120, ceiling 2048, corner length 1152, goal
  892.755 x 642.775, goal depth 880) maps directly onto a primitive's
  parameter and is independently checkable.
- Mesh ingestion is blocked on the owner's machine (FR-040) and would add
  a BVH triangle-mesh narrow phase this port does not have.
- ADR-0003's target is "Bullet-derived fidelity", scored by divergence. A
  geometry error only matters once it is shown to drive divergence.

## Considered options

1. **Analytic primitives** (chosen): planes, single-radius quarter-pipe
   fillets, single-radius corner fillets, a windowed goal wall, and
   bounded goal-interior walls.
2. **The real dumped triangle mesh** with a BVH narrow phase (Bullet's
   `btBvhTriangleMeshShape`, as RocketSim uses). This is the highest
   fidelity, but it is blocked on mesh extraction and needs a new
   narrow-phase family.
3. **A hybrid**: primitives for the flat regions and a mesh only for the
   curved corners and ramps. This inherits option 2's blocker without
   removing it.

## Decision

Build the arena from analytic primitives (option 1), each shape's
parameters taken from a cited community or RocketSim value where one
exists. The non-goal is explicit: no geometry finer than a flat plane, a
single-radius edge fillet or a single-radius corner fillet per boundary
segment. Fillets are independent, additive contact sources and are not
blended.

## Consequences

### Positive

- The arena is 9 planes, 30 edge fillets, 20 corner fillets, 2 windowed
  goal walls and 2 bounded goal interiors, all tested analytically.
- Each dimension is one named constant (`arena.rs`), so calibrating one
  is a one-line change.
- The shapes made FR-032's exactness proof for corner testing possible
  (see ADR-0007).

### Negative / tradeoffs

- `FILLET_RADIUS` (292) and `CORNER_ARCH_RADIUS` (750) are uncalibrated
  placeholders. FR-040 considered the RLBot wiki's "approx. 256, not
  circular" and did not adopt it.
- Real corners are curved and blend into ramps; the port's corner walls
  are flat cuts with fillets. Contacts near seams can differ from the
  mesh.
- Moving to a mesh later means a new narrow phase for both ball and car,
  and re-validating every arena test.

## Validation and revisit triggers

- Revisit when `--self-growth` or FR-005 calibration attributes
  divergence to contacts near fillets, corners or goal edges.
- Revisit once a dumped arena mesh is available locally (the FR-040
  blocker is cleared). At that point, measure the primitive-vs-mesh
  surface distance before deciding.
