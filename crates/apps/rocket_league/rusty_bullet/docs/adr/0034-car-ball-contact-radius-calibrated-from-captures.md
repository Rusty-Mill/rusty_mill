# ADR-0034: Car-ball contacts use a 92.3 uu ball, calibrated from 28 recorded hits

- Status: Accepted
- Date: 2026-10-03
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-114, FR-036, FR-107, FR-108; ADR-0026, ADR-0027
- Supersedes/Superseded by: —

## Context

`hitjump.jsonl` 83.19 s: the simulator registered a car-ball hit one tick
before the game. That made the ball 2,668 uu/s off. The next tick, the
post-hit state triggered the extra hit velocity a second time.

At the tick the game did not register, the car's hitbox overlaps a
`BALL_RADIUS` (93.15) sphere by 0.57 uu. Across all 28 hits in the four
captures:
- at the tick each hit registers, the overlap is at least 1.13 uu;
- the tick before, it is at most −0.75 uu, apart from that 0.57.

So the game's effective car-ball contact radius lies between 92.02 and
92.58 uu.

93.15 was chosen in FR-036 because a resting ball sits at z = 93.15.
RocketSim's ball is a 91.25 sphere, and it rests at that height through
Bullet's contact band: since ADR-0027, a contact still clear of its
surface stops the approach. That band (about 1.8 uu) would also make
RocketSim's sphere hit at 0.57 uu overlap. So the source does not explain
the recorded hits, and the captures decide.

## Decision

- `body::BALL_CAR_CONTACT_RADIUS = 92.3`, the middle of the band the data
  allows.
- `PhysicsWorld::step` computes car-ball contacts with a copy of the ball
  at that radius. World contacts, mass and inertia keep `BALL_RADIUS`.
- No other radius change: world contacts are already right at 93.15.

## Consequences

- `hitjump.jsonl`, summed over hit frames: ball 821 uu/s (was 5,368), car
  440 (was 696).
- `hitjump.jsonl` one-step mean, resets excluded: ball 0.62 (was 0.75).
- `test2`, `front` and `side` are unchanged.
- This is a calibration, not a ported constant. A capture with a hit
  outside the 92.02–92.58 band would move it. The band and the test
  (`a_car_hits_the_ball_only_inside_the_car_contact_radius`) make that
  visible.
