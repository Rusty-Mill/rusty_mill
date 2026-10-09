# ADR-0048: A car's wheel rays hit the ball

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-129, FR-088, FR-090, FR-122; ADR-0047
- Supersedes/Superseded by: —

## Context

After ADR-0047 the largest ball frame in the recordings was `hitjump.jsonl`
77.667 s (135 uu/s), the car's largest too (110 uu/s). A car at 2158 uu/s,
airborne with `jump` held, passes over a ball resting at 93.14 uu. The
front-bottom edge of its hitbox meets the ball's top at 40° from vertical.

Everything about the contact looked right: the normal, the point, the
extra hit impulse (formula and direction), friction, restitution, solver
order and iteration count (10 to 200 give the same result), the floor
friction (0.2 to 5.0 give the same result). None of those moves the hit.

Decomposing the recorded momentum: the car gains (+149, +215, +95) uu/s,
the sim (+103, +126, +56). The car's extra (+46, +89, +40) uu/s is a
reaction with no source in the contact. Removing the floor from the sim
made the ball's horizontal velocity and spin match the recording to a
few uu/s, so the contact impulse itself was right; the car was
receiving something else.

The car's front wheel rays start at hitbox-centre height and reach 52 uu.
At this tick the front wheels stand about 87 uu horizontally from the
ball's centre, inside its 91 uu radius, and their rays cross the ball's
upper surface at about 40 uu, inside their reach. The suspension then
bottoms out (the extra pushback, ADR-0042) and pushes the car up and,
through the tilted ray, back.

## Decision

- `collision::raycast_sphere` casts a ray against a sphere.
- `PhysicsWorld::wheel_ray` takes the nearer of the arena and the ball,
  the ball solid at `BALL_COLLISION_RADIUS` (91.25 uu, the shape's radius;
  a 93.15 uu ray radius reads 9 vs 8 uu/s on the car and 53 vs 46 on the
  ball, so 91.25).
- The ball is a still surface for the wheel model: its velocity does not
  enter the pushback or friction. A wheel on the ball counts toward the
  three that make the car grounded.

Alternatives considered:

- Keep wheel rays arena-only (RocketSim). Cannot reproduce the frame.
- Model the ball's velocity in the wheel model. Not testable here: one
  sample, ball at rest. Noted for a capture with a moving ball under the
  wheels.

## Consequences

- `hitjump` 77.667 s: car 110 → 8 uu/s, ball 135 → 46, pitch spin
  -0.05 → -1.03 rad/s against -1.04 recorded. 77.675–77.692 s car 76, 62,
  50 → 6, 6, 5. 77.700 s car 0.6 → 5.4, the only frame that gets worse.
- One-step car 0.482 → 0.474, ball 0.435 → 0.433; k = 30 car 11.33 →
  11.24, ball 21.18 → 21.11. No other frame in any capture changes: the
  situation (wheel over ball) occurs once in the recordings.
- Grounded status now follows the ball: a car with three wheels on the
  ball is "on the ground" for jumping and flip refresh, as flip resets in
  the game suggest. Nothing in the recordings tests it.
- The largest ball frames are now 274.217 s (47 uu/s) and 77.667 s (46).
