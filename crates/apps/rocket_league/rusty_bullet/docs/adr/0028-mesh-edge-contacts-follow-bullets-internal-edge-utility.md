# ADR-0028: Mesh edge contacts follow Bullet's internal-edge adjustment

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-109, FR-106, FR-108; ADR-0025, ADR-0027
- Supersedes/Superseded by: —

## Context

After ADR-0027 the worst one-step ball errors in `test2.jsonl` were
bounces off the corner mesh: 13.967 s (252 uu/s) and 7.95 s (201). At
7.95 s the recorded impulse leans 6 degrees up from horizontal, which is
the face the ball sits on (normal z 0.10). Our contacts also included the
ball touching three neighbouring triangles' edges, with normals pointing
from each edge to the ball's centre (z 0.21, -0.13, 0.43), so the folded
contact leaned 9 degrees. RocketSim calls Bullet's
`btAdjustInternalEdgeContacts` on every contact. On a concave or flat
shared edge it replaces the normal with the triangle's face normal. On a
convex edge it clamps the normal so it turns no further than the
neighbouring face. Unshared edges are left alone.

## Decision

- `mesh::StaticMesh::new` classifies each triangle edge against the
  triangle sharing it (vertices within 0.5 uu, Bullet's
  `m_equalVertexThreshold`):
  - convex, when the neighbour bends away by more than `m_planarEpsilon`;
  - smooth, when it is concave or flat;
  - open, when no triangle shares it.
- `sphere_contacts` adjusts a contact within 5 uu of its nearest non-open
  edge (`m_edgeDistanceThreshold`):
  - smooth edge: the face normal, if it faces the same side;
  - convex edge: clamped to the wedge.

  The contact point moves to keep the sphere's own contact point, as
  Bullet does.
- `combined_ball_world_contact` measures the distance to the ball's own
  contact point (`point - normal * depth`), as Bullet's `rel_pos1` does.
  For a sphere this is its radius.
- Car-mesh contacts are unchanged: they already use face normals.

## Consequences

- `test2.jsonl` one-step ball error: mean 0.17 uu/s (was 0.41). 13.967 s
  drops out of the top steps; 7.95 s is 32 (was 201). Car errors are
  unchanged. The worst ball steps are now car hits: 5.758 s (129) and
  12.267 s (90).
- The soccar corner meshes have 2 convex edges each; the side ramps have
  none.
- The goal-post fillet test lifts its ball off the floor, because the
  floor contact folded in with the fillet's.
