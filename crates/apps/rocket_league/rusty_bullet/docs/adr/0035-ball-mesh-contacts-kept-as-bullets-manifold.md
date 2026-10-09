# ADR-0035: Ball-mesh contacts are kept as Bullet's manifold keeps them

- Status: Accepted
- Date: 2026-10-03
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-115, FR-106, FR-108, FR-109, FR-113; ADR-0025, ADR-0027, ADR-0028
- Supersedes/Superseded by: —

## Context

`hitjump.jsonl` 97.9 s: the ball hit the +y goal's rounded back corner
and the simulator was 1,036 uu/s off, the worst ball frame in the
captures.

The ball reached 48 facets of the corner's fan. `StaticMesh::sphere_contacts`
kept the 4 deepest. They all sat on the sloped back, two pairs 0.7 uu
apart on adjacent facets. The averaged normal (FR-108) pointed too far
back, and the goal's side wall had no say.

Bullet adds each triangle's point to one persistent manifold:
- `getCacheEntry`: a point within the contact breaking threshold
  (0.02 × the ball's radius, about 1.9 uu) of a kept one replaces it.
- `sortCachedPoints`: when the manifold is full, the new point replaces
  the one whose removal leaves the largest area. The deepest point is
  never replaced.

## Decision

- `sphere_contacts` feeds each triangle's contact, in triangle order,
  through `mesh::add_manifold_point`, which follows those two rules.
- `collision::replacement_slot` is `sortCachedPoints` (3-point area
  variant, `KEEP_DEEPEST_POINT`), written so the box manifold can adopt it
  later.
- The manifold is rebuilt every tick. Bullet keeps it across ticks.

Alternatives considered:
- Merge near duplicates, then keep the 4 deepest. This still clusters
  the 4 on the fan nearest the ball's center. The probe at 97.9 s showed
  4 back-slope points and no side wall.
- A ball manifold that persists across ticks. This is closer to Bullet,
  but it adds state to the world and to `snap_to_frame`. Bullet's
  insertion order (its BVH traversal) is not reproduced either way.
  Deferred until a capture shows a need for it.

## Consequences

- `hitjump` 97.9 s: 225 uu/s (was 1,036).
- `hitjump` one-step ball mean, resets excluded: 0.595 (was 0.615).
- `hitjump` k = 30 ball, resets excluded: 22.43 (was 22.65).
- `test2`, `front` and `side` are unchanged.
- Across `hitjump`, 17 frames got better by more than 20 uu/s and 6 got
  worse.
- New worst ball frame: 127.633 s, 988 uu/s (was 1). The ball meets the
  crossbar's bevel at a vertex shared by 8 facets. Two bevel facets pick
  a coplanar edge in FR-109's nearest-edge tie and keep their downward
  face normal, which now makes the 4. Bullet's nearest-edge choice is
  also a float tie at a shared vertex, so this is the next item to
  investigate, not a reason to keep the deepest-4 rule.
