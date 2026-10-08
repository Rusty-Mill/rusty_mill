# ADR-0022: Arena curves take the real mesh's radii; wheel rays hit every surface

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-102, FR-020 to FR-027, FR-036, FR-040,
  FR-088; RB-VERIFY-003-FR-006; ADR-0006, ADR-0007, ADR-0017
- Supersedes/Superseded by: supersedes FR-025's corner-arch radius and
  FR-023's corner spheres; amends ADR-0017 (wheel rays cast against the
  floor only)

## Context

`rb-verify --self-onestep` ranked `test2.jsonl` 8.86-8.95 s as the
model's worst steps by far (about 1,390 uu/s). The recorded car flips past
the +X/+Y corner at z 186, 110-150 uu from the corner wall. The
candidate's corner arch used FR-025's uncalibrated 750 uu radius, which
put the surface 256 uu from the wall at that height, inside the car. The
car then drives the corner and the side wall, where the wheel rays
(ADR-0017) only saw the floor.

FR-040 found no reliable published radius. FR-036 had already used
RLUtilities' extracted soccar collision mesh (`samuelpmish/RLUtilities`,
`assets/soccar/*.bin`, GPL-3.0; only measured dimensions are used, no data is copied) for the flat walls. That mesh also fixes the
curves: its ramp vertices lie on circles of 256 uu (side and corner floor),
160 uu (back-wall floor) and 512 uu (every ceiling) to within 0.5 uu. The
vertical corner edges are 864 uu bends, not the 292 uu placeholder. At the
side end of an edge the floor ramp wraps it as a torus of tube 256 uu
(within 2.4 uu); at the back end the tube blends from 256 to 160 uu.

Options considered:

1. Import the triangle mesh (RocketSim's own approach): exact, but needs
   box-vs-triangle narrow phase and a BVH, a large change across
   collision, the solver and the arena.
2. Keep ADR-0006's analytic primitives with radii fitted to the mesh, and
   add one primitive for the swept corner seams.

## Decision

Option 2.

- `arena` radii come from the mesh: `SIDE_FLOOR_RADIUS` 256,
  `BACK_FLOOR_RADIUS` 160, `CORNER_FLOOR_RADIUS` 256, `CEILING_RADIUS` 512,
  `CORNER_EDGE_RADIUS` 864. The goal fillets keep the old 292 uu
  placeholder as `GOAL_FILLET_RADIUS`.
- `body::StaticSweptFillet`: a floor or ceiling seam swept around a curved
  vertical edge, with a tube radius that blends linearly across the
  sector. It replaces the 16 arena `StaticCornerFillet` spheres.
- `collision::raycast` bisects a ray against any point-contact function
  and finishes with one Newton step. A wheel ray uses the scene's deepest
  zero-radius contact, so it hits the floor, walls, curves and sweeps
  alike, still front face only.

## Consequences

- `test2.jsonl` one-step error: mean 3.1 uu/s (was 15.2), 99th percentile
  28.7 (was 222), worst tick 432 (was 1,393). The corner and wall segment
  averages 13.4 uu/s (was 117.7). Captures without wall contact are
  unchanged.
- A car can drive on walls and ceilings with wheel contact and the sticky
  force, and a wall jump is a ground jump.
- A box against a sweep is tested by its corners. Distance from a torus
  center circle is only locally convex, so this is an approximation, close
  while the box is small against the tube.
- Open: the goal mesh's own fillets, and, at 15.625 s in `test2.jsonl`,
  a double jump 0.9 s after a wall jump that the real car does not get.
