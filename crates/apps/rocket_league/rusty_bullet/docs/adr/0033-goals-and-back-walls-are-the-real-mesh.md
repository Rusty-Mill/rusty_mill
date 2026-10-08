# ADR-0033: The goals and back walls are the real collision mesh

- Status: Accepted
- Date: 2026-10-03
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-113, FR-112, FR-106, FR-024, FR-026, FR-029,
  FR-033; ADR-0025, ADR-0032
- Supersedes/Superseded by: replaces the analytic goal (FR-024, FR-026,
  FR-029), the soft net (FR-033) and the back-wall seams in
  `standard_arena`

## Context

After ADR-0032 opened the goal mouth, `hitjump.jsonl`'s worst one-step
errors were all in the goal area:
- **119.8 s:** a ball off the back of the goal, 3,680 uu/s off. The real
  back slopes; ours was a flat wall further back.
- **29.6 s:** a ball off the back wall beside the post, 2,834 uu/s off.
- **169.95 s:** a car climbing the back wall beside the goal, 471 uu/s off.

RLUtilities ships the game's goal collision mesh, `soccar_goal`, under the
same source and license as the meshes vendored in ADR-0025. It covers
`|x| <= 2688` from the field side of the back wall to the back of the
goal: the back wall with its floor and ceiling seams, the rounded goal
mouth, and the goal's sloped back, sides, roof and floor.
`Field::initialize_soccar` builds the arena from the 4 corners, 2 goals,
the ramps, and a floor, ceiling and side-wall planes. RocketSim also
collides with the game's meshes, and has no soft net.

## Decision

- Vendor `soccar_goal` and add it twice (`arena::standard_goal_meshes`):
  moved to `y = -5120` and mirrored for `+y`.
- Its triangles keep the file's winding (`StaticMesh::from_wound_buffers`,
  `Triangle::wound`), flipped back under a reflecting mirror. The roof
  faces down and the back faces the field, so no single "inside" point
  orients every face.
- `PhysicsWorld::standard_arena` becomes RLUtilities' arena: ground, side
  walls and ceiling as planes, plus 10 meshes. The back-wall seams, goal
  walls, goal boxes, goal fillets and nets are no longer part of it. Their
  shapes and tests remain.

## Consequences

- `hitjump.jsonl` one-step (resets excluded):
  - car mean 0.58 uu/s (was 2.06);
  - ball mean 0.75 uu/s (was 2.35);
  - the 119.8, 29.6 and 169.95 s errors leave the top steps.
- `hitjump.jsonl` at k = 30: car velocity 11.6 uu/s (was 35.5).
- `test2`, `front` and `side` are unchanged.
- The net no longer gives: a ball hits the goal's rigid back, as in
  RocketSim. FR-033's soft net is unused.
- Edge contacts at the goal posts now use ADR-0028's convex clamp.
