# ADR-0036: Ball manifold points are matched on the ball, not the mesh

- Status: Accepted
- Date: 2026-10-03
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-116, FR-109, FR-115; ADR-0028, ADR-0035
- Supersedes/Superseded by: —

## Context

After FR-115, the worst ball frame is `hitjump.jsonl` 127.633 s, at
988 uu/s. The ball meets the crossbar bevel at a vertex shared by 8
triangles:
- 4 back-wall triangles; FR-109's edge adjustment clamps their normal
  to −y;
- 4 bevel triangles; FR-109 keeps their downward face normal
  (0, −0.71, −0.71).

The recorded bounce has no vertical kick.

FR-115 matched manifold points by the contact point on the mesh. In
Bullet, `btManifoldResult::addContactPoint` matches (`getCacheEntry`) and
sorts (`sortCachedPoints`) by `m_localPointA`, the point on the ball
along the *unadjusted* normal. `btAdjustInternalEdgeContacts` runs
afterwards, through the contact-added callback, on the stored point
only.

Every triangle meeting the ball at one vertex has the same unadjusted
normal, and so the same ball point. They fold into one manifold point:
the last triangle reported, with that triangle's adjusted normal.

## Decision

- `mesh::ManifoldEntry` pairs each contact with its ball-side point.
- `add_manifold_point` matches and sorts on the ball-side point.
- `collision::replacement_slot` takes (point, depth) pairs.
- Triangles are still reported in mesh index order. Bullet reports
  them in the order its quantized BVH visits them, which is not ported.

Alternatives considered:
- Port `btOptimizedBvh` build and traversal to get Bullet's triangle
  order. This is about 300 lines. It only matches if our mesh triangle
  order equals RocketSim's, and RLUtilities mirrors half the meshes
  itself. This is deferred to the owner (see Consequences).
- Prefer the clamped normal at shared vertices. This is not Bullet, so
  it was rejected.

## Consequences

- `hitjump` one-step ball mean, resets excluded: 0.589 (was 0.595). Two
  spikes from FR-115 are gone: 104.408 s (357 to below 230) and
  225.533 s (303 to below 230).
- `hitjump` k = 30 ball: 22.47 (was 22.43, flat).
- `test2`, `front` and `side` are unchanged.
- 127.633 s is still 988 uu/s. In mesh index order the bevel triangle
  is reported last, so its downward normal wins. The fix now hinges on
  triangle order.
