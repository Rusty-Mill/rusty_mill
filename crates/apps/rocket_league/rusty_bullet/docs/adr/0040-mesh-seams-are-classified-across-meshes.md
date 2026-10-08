# ADR-0040: Edges on a seam between meshes are classified across meshes

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-120, FR-109, FR-117; ADR-0028, ADR-0037
- Supersedes/Superseded by: amends ADR-0037's seam note

## Context

`hitjump.jsonl` 119.900 s: the ball meets the goal roof 16 uu from the
`x = 0` seam between RocketSim's two goal-half meshes (ADR-0037), moving
up at 1,808 uu/s. The game's bounce has no sideways component. The
simulator gave 84 uu/s sideways (110 uu/s off).

The ball reaches the `x <= 0` half only at its seam edge. FR-109's edge
classification (`btGenerateInternalEdgeInfo`) runs per mesh, so that
edge has no neighbour and is `Open`: the contact keeps the edge-to-centre
normal, tilted 0.18 in x. The `x >= 0` half reports a face contact with
no x. Bullet keeps each `btTriangleInfoMap` to its own mesh, so RocketSim
has the same seam; the game, whose arena is not split there, does not.

## Decision

- `StaticMesh::classify_seams(&mut [StaticMesh])` re-classifies every
  edge a mesh left `Open` against the other meshes' nearby triangles,
  with the same `edge_kind` rule as within a mesh.
- `arena::standard_meshes` calls it after loading the 16 files.
- Per-mesh classification in `StaticMesh::new` is unchanged; a mesh used
  alone behaves as before.

Alternatives considered:
- Merge the goal halves into one mesh. Changes the BVH order FR-117
  reproduces and the per-mesh manifolds.
- Leave it, as RocketSim does. The recording decides: no sideways kick.

## Consequences

- `hitjump` 119.908 s: 75 uu/s (was 110); the sideways kick is gone, the
  rest is the bounce's vertical component. One-step ball 0.500 (was
  0.501); k = 30 ball 20.37 (was 20.41).
- `test2`, `front` and `side` are unchanged.
- The mesh test `a_seam_between_meshes_is_smooth_once_classified_across_them`
  shows the tilted normal before and the face normal after.
- Seams between the corner, ramp and goal meshes are classified the same
  way wherever their vertices coincide within `EQUAL_VERTEX_DISTANCE`.
