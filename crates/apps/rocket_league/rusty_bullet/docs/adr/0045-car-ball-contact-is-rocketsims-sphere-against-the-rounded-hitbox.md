# ADR-0045: Car-ball contacts are RocketSim's 91.25 sphere against the margin-rounded hitbox, admitted to the breaking threshold

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-125, FR-114, FR-108, FR-124; ADR-0034, ADR-0044
- Supersedes/Superseded by: supersedes ADR-0034

## Context

ADR-0034 fitted a 92.3 uu car-ball radius to the 28 recorded hits
because, with a sharp hitbox, the game registered hits only from 1.13 uu
overlap with a 93.15 sphere and skipped one tick at 0.57 uu
(`hitjump.jsonl` 83.2 s). RocketSim's 91.25 sphere could not be made to
fit: with Bullet's contact band it would hit at 0.57.

ADR-0044 established that Bullet's `btBoxShape` carries a 2 uu margin and
is rounded at its edges and corners. `btSphereBoxCollisionAlgorithm::
getSphereDistance` sees the same shape: it clamps the sphere center to
`getHalfExtentsWithoutMargin` (the core box) and compares the distance
with `radius + boxMargin`. Re-measuring the 28 hits against that shape:

- every hit tick is at least 0.67 uu inside a 93.15 sphere's reach;
- every tick before a hit is at least 0.04 uu clear. The 0.57 uu "overlap"
  at 83.2 s is a corner contact and reads 0.04 uu clear rounded.

So the effective car-ball radius lies in [92.48, 93.19). RocketSim's
sphere is 91.25 and the pair's contact breaking threshold is 0.02 × the
ball's angular motion disc = 1.825 uu; its Bullet solves a point still
clear of touching as touching (FR-108, no speculative slack). That is an
effective 93.075, inside the band. The sharp-box band (92.02–92.58)
excluded it; the rounded one contains it.

## Decision

- `collision::sphere_vs_box` ports `getSphereDistance`: core box
  (`body::BOX_COLLISION_MARGIN` off each half extent, floored at 0),
  reach `radius + margin`, contact point on the nominal surface. The deep
  case keeps Bullet's face order against the core box.
- `collision::breaking_threshold(body)` is `getContactBreakingThreshold`
  (0.02 × bounding radius plus offset); `contacts_between` admits a
  sphere-box contact up to the smaller of the pair's, in place of the
  0.01 uu processing threshold.
- `PhysicsWorld::step` builds the car-ball sphere at
  `BALL_COLLISION_RADIUS` (91.25). `BALL_CAR_CONTACT_RADIUS` is removed;
  FR-114 is superseded.

Alternatives considered:

- Keep a calibrated radius, refitted to the rounded band (92.83). Rejected:
  the source now explains the data, and one fewer fitted constant.
- `BALL_RADIUS` (93.15) as the car-contact sphere with no band. Inside the
  band but 0.04 uu from its edge; the port is not.

## Consequences

- `hitjump` summed over hit frames: ball 470 uu/s (was 821), car 383 (was
  434). 109.350 s ball 77 → 4, 83.200 s 51 → 5, 229.883 s 51 → 13,
  267.117 s 43 → 11. `test2` hit frames: ball 42 (was 64), car 14 (was 18).
- One-step, resets excluded: `hitjump` ball 0.490 (was 0.500), car 0.482
  (was 0.484); `test2` ball 0.091 (was 0.099), car 0.445 (was 0.447).
  k = 30: `hitjump` ball 21.29 (was 21.55), `test2` ball 1.39 (was 1.62).
  `front`, `side` unchanged; no frame worse.
- `a_car_hits_the_ball_only_inside_the_car_contact_radius` now states the
  face-on band; `a_ball_at_a_hitbox_corner_meets_it_rounded` the corner.
  A solver test's "touching" ball moved to the hitbox's height, since its
  old spot was on the rounded front-bottom edge.
- The 77.667 s aerial top-of-ball hit stays at 135 uu/s (was 138): not
  a contact-shape question.
