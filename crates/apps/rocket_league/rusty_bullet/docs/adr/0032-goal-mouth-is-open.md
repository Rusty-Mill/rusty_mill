# ADR-0032: The goal mouth is open: no cutout fillets, no floor seam across it

- Status: Accepted
- Date: 2026-10-03
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-112, FR-024, FR-026, FR-020; ADR-0025
- Supersedes/Superseded by: reverses FR-024's cutout fillets and FR-026's
  goal corner fillets in `standard_arena`

## Context

The owner's new capture, `hitjump.jsonl`, is the first that puts the ball
and the car in a goal. Two one-step errors stood out:
- **56.3 s:** a ball shot into the goal mouth at z = 355 bounced back out
  (vy +2,919 → −1,740, 4,667 uu/s off).
- **277.0 s:** a car driving inside the goal was shoved back (1,909 uu/s
  off).

The causes:
- **Cutout fillets.** FR-024's fillets are concave quarter-pipes between
  the back wall and the post and crossbar planes. The concave region
  between the back wall and the crossbar's underside is the goal mouth
  itself, so the crossbar fillet filled the top of the opening and the
  post fillets its sides. Seen from the field, the real edges are convex.
  FR-026's corner fillets join those two kinds.
- **Floor seam.** The back wall's floor seam (FR-020) is infinite along x,
  so it crosses the goal mouth, where the real floor runs flat into the
  goal.

## Decision

- `PhysicsWorld::standard_arena` no longer adds
  `standard_goal_cutout_fillets` or `standard_goal_corner_fillets`. The
  functions and their isolated tests remain, so this is reversible.
- `StaticQuarterPipe` gains `span`, the stretch of its axis it covers
  (infinite by default; `with_span`). `standard_curves` splits each back
  wall's floor seam into the two stretches beside the goal (|x| from
  `GOAL_HALF_WIDTH` to `SIDE_WALL_X`).

## Consequences

- `hitjump.jsonl` one-step car error 2.28 uu/s (was 23.41); k = 30 car
  42.4 uu / 35.5 uu/s (was 91.2 / 48.3). `test2`, `front` and `side` are
  unchanged (no goals).
- The goal's edges are now sharp, where the real ones are rounded convex,
  and the back of the goal is a flat plane, where the real one slopes:
  `hitjump` 119.8 s (ball off the goal's back, 3,680 uu/s) and 29.6 s
  (ball beside the post, 2,834). RLUtilities' `soccar_goal` mesh (GPL-3.0,
  like the vendored meshes) is the next step.
