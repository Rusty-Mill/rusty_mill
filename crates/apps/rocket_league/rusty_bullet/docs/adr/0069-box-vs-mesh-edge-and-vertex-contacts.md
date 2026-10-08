# ADR-0069: Box against a mesh triangle's edge or vertex, by the separating-axis test

- Status: Accepted (validated against the game on 2026-10-07)
- Date: 2026-10-07
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-146`, `FR-106`, `FR-109`, `FR-124`, ADR-0025,
  RB-RESEARCH-O017
- Supersedes/Superseded by: none (extends the box-mesh contact of FR-106)

## Context

A car's box met a mesh triangle only as "its deepest corner while that corner
lies over the triangle". `probe_wall_land` (a car turned 45 degrees to the side
wall) shows the game's impulse along a *different facet's* normal: from the
velocity and spin change of the contact tick the line of action passes within
8 uu of the centre of mass in both the game and the port, but the game's
direction fits the upper (6 degree) facet's normal to 0.03 degrees at friction
0.3, the port's the lower (17 degree) ramp it was over. Bullet's closest-feature
search (GJK) finds the triangle's edge against the box's vertical edge, and its
internal-edge adjustment (the ball already has it, FR-109) turns the normal of
a seam between flat facets into the facet's own.

## Decision drivers

- One mechanism per observed effect; no new constants.
- Leave every corner-on-face contact exactly as it was.
- No dependency.

## Considered options

1. Tune a car-mesh restitution/friction (tried: nothing fits the contact and the
   others, 30 uu at best).
2. A wider `covers` tolerance for the corner test (a fit, not the mechanism).
3. The separating-axis test per triangle, taking over only where its axis of
   least penetration is not the triangle normal, plus the ball's edge-normal
   adjustment.

## Decision

Option 3 (FR-146).

## Consequences

### Positive

- `probe_wall_land` 77 -> 4.4 uu with the material 0.3 / 0.3 unchanged (the
  scratch fits were compensating for the missing contact).
- Reuses `adjust_edge_normal`; contacts on a flat seam never turn into a wall.

### Negative / tradeoffs

- Contacts on convex ridges (open or convex edges) now exist and are untested:
  no recording isolates a ridge.
- The manifold's points may be edge or face points; their margin rounding is
  approximate away from corners.
- A box straddling a seam between non-coplanar facets gets more contacts per
  tick than before.

## Validation and revisit triggers

- The `collision` tests and the golden `probe_wall_land`.
- Revisit if a ramp-ridge or goal-post recording disagrees (the fuzz tapes'
  wall hits, RB-RESEARCH-O017).
