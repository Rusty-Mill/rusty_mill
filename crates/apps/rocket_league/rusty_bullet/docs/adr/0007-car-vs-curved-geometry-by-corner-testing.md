# ADR-0007: Detect car contact with curved static geometry by testing the box's 8 corners

- Status: Accepted (recorded retroactively 2026-09-30; decided 2026-08-31, confirmed 2026-09-01)
- Date: 2026-08-31
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-027, FR-028, FR-032, FR-054; PRs #63, #65,
  #73, #115; ADR-0006
- Supersedes/Superseded by: none

## Context

Before FR-027, `contacts_vs_quarter_pipe` and `contacts_vs_corner_fillet`
returned an empty manifold for `Shape::Box`, so cars drove straight
through every fillet. Only the ball was deflected. Before FR-028, a box
against a `StaticGoalWall` fell through to plain `contacts_vs_plane`, so a
car hit a solid back wall even at the goal mouth.

FR-027 closed the gap by treating each of the box's 8 corners as a
zero-radius sphere against the existing sphere-vs-fillet routines. It
shipped as a declared approximation.

FR-032 then set out to replace it with a general convex narrow phase. It
built `gjk::closest_points` from scratch. That broke two end-to-end tests:
GJK answers a *closest-point* question, but fillet contact is a
*containment* question. The GJK module was reverted and deleted.

## Decision drivers

- Fillet contact asks whether the box's farthest point from the fillet's
  axis (or center) is at or beyond the radius.
- Distance from a line or point is convex. The maximum of a convex
  function over a convex polytope is always reached at a vertex, so the 8
  corners are the whole answer.
- The existing sphere-vs-fillet routines already solve the per-point
  question; reusing them adds no narrow-phase machinery.

## Considered options

1. **Corner testing** (chosen): 8 zero-radius sphere queries per box per
   curved shape, each contact placed at the corner's world position for
   correct torque.
2. **GJK/EPA general convex narrow phase**: built and reverted in FR-032.
   It answers the wrong question for containment contact.
3. **SAT against curved surfaces**: not considered in the docs. SAT needs
   finite separating-axis candidates, which curved surfaces do not have.

## Decision

Use corner testing (option 1) for box contact against `StaticQuarterPipe`,
`StaticCornerFillet` and the rims of `StaticGoalWall`. FR-032 established
that this is exact, not an approximation, for containment-style contact.
The docs were corrected to say so.

## Consequences

### Positive

- Exact for every fillet shape in the arena, backed by an argument rather
  than by tuning. The test
  `no_point_on_a_boxs_face_is_ever_farther_from_a_quarter_pipes_axis_than_its_own_corners`
  samples a 50x50 grid on each of the 6 faces against a 292 uu pipe.
- No new narrow-phase framework to maintain.
- FR-054 extended the argument to goal-window edges: a box larger than the
  window matches an unwindowed plane bit-for-bit.

### Negative / tradeoffs

- When 2 or more corners penetrate at once, each becomes its own contact
  rather than one unified manifold. This is a stated non-goal, not a
  detection bug.
- `box_vs_bounded_wall` has a known gap: a face larger than the bound and
  centered on it yields zero contacts. It cannot occur with this project's
  car and bound sizes; a real fix needs a 2D convex-polygon overlap test.
- The exactness argument depends on containment contact. It does not
  carry over to convex obstacles (e.g. a mesh with outward bumps), which
  would need a different method.

## Validation and revisit triggers

- Revisit if the arena moves to a triangle mesh (ADR-0006): mesh contact
  is not containment contact.
- Revisit if a car or bound size change makes the `box_vs_bounded_wall`
  gap reachable.
