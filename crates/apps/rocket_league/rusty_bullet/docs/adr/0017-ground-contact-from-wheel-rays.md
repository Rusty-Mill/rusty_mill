# ADR-0017: Ground contact from wheel rays, not box contact

- Status: Accepted
- Date: 2026-10-01
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-088; ADR-0009, ADR-0012
- Supersedes/Superseded by: amends ADR-0012 (which ground contact gates
  the tire model)

## Context

`PhysicsWorld` counted a car as on the ground when any corner of its box
touched the floor. The 5.5-6.5 s real-capture trace
(`rb-verify --self-trace test2.jsonl 5.5 6.5`) showed what that costs:

- at 5.575 s the recorded car lands on its wheels at z 40.9, while the
  candidate's box lands later (z ~32), then bounces and tumbles;
- at 5.758 s the recorded car ground-jumps (vz +228). The candidate's box
  is mid-bounce at z 22.6 with no corner touching, so the press fires a
  side dodge instead; velocity error jumps from 430 to 1,248 uu/s;
- everything after that diverges: the 6-7 s `--self-growth` window was
  442 uu / 1.70 rad.

RocketSim (`Car.cpp`, `btVehicleRL::rayCast`, `CarConfig.cpp`, fetched
2026-10-01) casts one ray per wheel down the car's own axis and sets
`isOnGround = numWheelsInContact >= 3`. For the Octane the rays start
20.755 uu above the origin and reach `restLength + radius - 2.5`: 48.755
uu at the front, 49.555 uu at the back. A level car is grounded with its
origin up to ~28 uu above the floor; it rests at ~17.

## Decision drivers

- Every number is taken from RocketSim source; none is tuned.
- The jump-to-dodge misfire, not the landing bounce, causes most of the
  6-7 s error.
- Small and reversible: one function, the box and its contacts unchanged.

## Considered options

1. **Wheel-ray grounding only** (chosen): `drive::wheels_on_ground` casts
   the four rays against the floor plane; `PhysicsWorld` uses it for
   `on_ground`.
2. **Full raycast suspension**: wheel rays plus spring/damper forces and
   RocketSim's hitbox offset (the box rides ~18 uu up). It would also fix
   the landing bounce, but it is a large subsystem (ADR-0009 deferred it;
   ADR-0012 option 3). It can reuse option 1's rays later.

## Decision

Option 1. `on_ground` is three of four Octane wheel rays reaching the
floor plane from its front side. Walls keep box contact for wall jumps.

## Consequences

### Positive

- A car bouncing a few uu off the floor stays grounded, so a jump press
  there ground-jumps, as in the game.
- A car on its roof or side is no longer "grounded" by box contact, so it
  gets air control instead of tire grip, as in RocketSim.

### Negative / tradeoffs

- The tire model now acts while the box hovers up to ~10 uu off the floor,
  with no suspension holding it up; the box still falls onto the floor.
- Only the floor plane is ray-tested; driving on walls, curves or the
  ceiling still isn't grounded (unchanged from ADR-0012).
- The rays use the Octane geometry for every car, as the rest of the port
  does.
- The landing bounce at 5.575 s remains; it needs option 2.

## Validation and revisit triggers

- New `world` test
  `a_jump_press_while_bouncing_just_off_the_floor_jumps_instead_of_dodging`
  fails under box-contact grounding (a 500 uu/s side dodge) and passes now.
- Re-run `--self-trace test2.jsonl 5.5 6.5`: at 5.758 s the candidate
  should jump (vz ~+290), not dodge, and should dodge at 6.058 s like the
  recording.
- Real capture, 2026-10-01: confirmed. At 5.758 s the candidate jumps (vz
  57 to 344, recorded 228); 5-6 s growth 64 to 49 uu. The 4-5 s window
  moved 16 to 23 uu because grip now lasts 3 ticks past the 4.142 s jump
  (5 recorded), so the candidate's existing ~45 uu/s steering error carries
  forward differently; the candidate rides 2.3 uu high and leaves the
  floor 2 ticks early, which option 2 addresses.
- Revisit when suspension forces or surface driving are added.
