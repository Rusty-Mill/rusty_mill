# ADR-0044: A car's hitbox corner meets a mesh facet rounded by Bullet's collision margin, a plane sharp

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-124, FR-105, FR-106, FR-117; ADR-0024, ADR-0025, ADR-0037
- Supersedes/Superseded by: —

## Context

`test2.jsonl` 14.283–14.300 s: the car rides up a side ramp at 1850 uu/s.
At the step from 14.283 s the simulator's car was 31.7 uu/s off, the
largest wall-ride error in the recordings; the ticks around it are within
3 uu/s. Instrumenting the step showed the hitbox's nose corner 1.35 uu
clear of the next ramp facet (normal (0.636, 0, 0.771)), inside the box's
1.99 uu contact breaking threshold, so a speculative contact entered the
manifold and the solver let the corner close only that gap this tick,
taking the rest of its 400 uu/s approach out of the car. The recording
takes the same facet-normal impulse (+29, 0, +38 uu/s) one tick later.

RocketSim's car is a `btBoxShape` in a `btCompoundShape`, with Bullet's
default `CONVEX_DISTANCE_MARGIN` of 0.04 BT (2 uu). `btBoxShape` keeps a
core box shrunk by the margin on every axis and reports the margin
separately, so the shape is its nominal size face-on but rounded at the
corners. How a corner's gap is measured then depends on the other shape:

- Against a mesh triangle (`btConvexConcaveCollisionAlgorithm`, GJK), the
  distance is between the core box and the triangle, less the margin: a
  corner meeting a facet with normal `n` (box frame) reads
  `margin * (|n|_1 - 1)` further away than its sharp point. At 14.283 s
  that is 2.18 uu, past the (now 2.06 uu, margin included) threshold; at
  14.292 s 0.98 uu, the recorded tick.
- Against a static plane (`btConvexPlaneCollisionAlgorithm`), the support
  point is `btBoxShape::localGetSupportingVertex`, which adds the margin
  back axis-wise: the sharp corner. The recordings' corner landings on the
  floor (`hitjump.jsonl` 119.067 s, 96.783 s, 84.600 s) hit at the sharp
  corner's tick; rounding the plane path too put them a tick late, 282
  uu/s off each.

## Decision

- `body::BOX_COLLISION_MARGIN = 2.0` uu, Bullet's margin on the car's box.
- `collision::BoxCorners::gap(local, normal, offset, rounded)` measures a
  corner's gap to a surface plane; `rounded` subtracts
  `margin * (s . n + 1)` for the corner's sign vector `s` and the normal
  `n` in the box's frame, exact for each rounded corner against a plane.
  `ContactManifold::update_mesh` uses it rounded for detection and for
  refreshing stored points; `update_plane` sharp.
- The contact breaking threshold counts the margin in the box's angular
  motion disc, as `btCollisionShape::getAngularMotionDisc` does: 2.06 uu.
- The contact point stays the sharp corner; Bullet's lies within the
  margin of it, and the solver's lever arm does not see 2 uu.

Alternatives considered:

- Rounding against planes too (one code path). Rejected by the floor
  landings above; Bullet's two algorithms genuinely differ.
- A full GJK box-vs-triangle narrow phase. The corner-vs-plane manifold
  of ADR-0024/0025 already reproduces Bullet's point selection on the
  recordings; the margin was the one missing term.

## Consequences

- `test2` 14.292 s: 4.7 uu/s (was 31.7); 9.083–9.117 s 0.0–4.6 (was
  11–17). One-step car 0.447 (was 0.470), k = 30 car 5.53 (was 5.65).
- `hitjump` 170.608 s 0.8 (was 59.5), 170.700 s 1.7 (was 48.5), 177.542 s
  2.5 (was 36.2), 169.667 s 2.8 (was 33.6); one-step car 0.484 (was
  0.489), k = 30 car 11.37 (was 11.39).
- `front` 0.180 (was 0.178; nothing worse than 2.9 uu/s), `side` 0.192
  unchanged. Ball errors unchanged: the ball is a sphere with its own
  radius handling (FR-114, FR-118).
- The same margin applies to Bullet's box-vs-sphere algorithm, which
  measures the ball against the core box plus margins; the car-ball
  contact here (`BALL_CAR_CONTACT_RADIUS`, FR-114) is calibrated rather
  than ported and is not changed by this ADR. A lead for the aerial
  top-of-ball hit at `hitjump` 77.667 s.
