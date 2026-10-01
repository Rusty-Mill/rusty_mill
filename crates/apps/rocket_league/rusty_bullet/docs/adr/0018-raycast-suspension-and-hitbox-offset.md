# ADR-0018: Raycast suspension and the Octane hitbox offset

- Status: Accepted
- Date: 2026-10-01
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-090, FR-088, FR-089; ADR-0009, ADR-0012,
  ADR-0017
- Supersedes/Superseded by: adopts ADR-0017's deferred option 2; amends
  ADR-0012 (tire contacts are the wheel rays' hits, not the box's floor)

## Context

After FR-088 and FR-089 the earliest real-capture error
(`rb-verify --self-trace test2.jsonl 3.7 4.2`) was the car's height:

- the candidate's box rested on the floor with the origin at 19.3 uu; the
  recorded car rests at 17.0;
- after the 4.142 s jump its wheel rays left the floor 2 ticks before the
  recording's, and velocity error grew from 0.5 to 28 uu/s by 4.183 s;
- on landing at 5.575 s its box bounced, where the recorded car's
  suspension absorbed the landing;
- the car–ball hit at 5.758 s landed 3 ticks late.

RocketSim (`Car::_BulletSetup`, `btVehicleRL::rayCast`/`updateSuspension`,
`Car::_UpdateWheels`, `CarConfig.cpp`, `RLConst.h`, `btContactConstraint.cpp`;
fetched 2026-10-01) carries the car on four raycast wheels:

- the hitbox is a compound child offset by `(13.8757, 0, 20.755)` uu, with
  the box's own inertia about the origin;
- each wheel's spring is `(rest - length) * 500`, damped by 25 compressing
  and 40 relaxing, scaled by 35.75 (front) or 54.265 (back), never pulling;
- a pushback (`resolveSingleCollision`, ERP 0.2) stops a wheel sinking past
  its rest reach;
- a sticky force of half default gravity pushes a car with any wheel
  touching into the surface, plus `1 - |normal.z|` on slopes while driving.

## Decision drivers

- Every constant is RocketSim's; none is tuned. Bullet's suspension
  formulas give the same numbers in uu, since force and length scale
  together.
- The offset and the suspension go together: offset alone leaves the box
  floating with nothing holding it; suspension alone leaves the box on the
  floor.

## Considered options

1. **Port both** (chosen): `RigidBody::shape_offset` moves where the box
   collides; `drive::wheels` casts the rays and applies suspension and the
   sticky force each step.
2. **Lower the box by 2.3 uu** (a smaller box or an offset without
   suspension): it would match the rest height but not the landing, the
   jump take-off, or the ball contacts, and the numbers would be invented.
3. **Leave it**: the height error now leads the real-capture error.

## Decision

Option 1:

- `RigidBody::shape_offset` (zero by default; `CAR_HITBOX_OFFSET` for
  `standard_car`) and `RigidBody::shape_center`. Every box collision routine
  is called with the shape centre; contact lever arms stay about the origin.
- `drive::wheels`: `cast_wheels` returns the four hits once per step;
  `is_on_ground` is FR-088's three-wheel rule over them;
  `apply_wheel_forces` applies suspension, pushback and sticky force.
- `apply_driven_forces` takes the wheel hits instead of `on_ground`. Tire
  grip and engine force (FR-086, FR-089) act at the hits, with the slip
  ratio from each wheel's hard point, as in `_UpdateWheels`.
- `PhysicsWorld::step` casts the wheels at the start of the step, then
  calls `apply_driven_forces` and `apply_wheel_forces` before integrating.

## Consequences

### Positive

- A standard car settles at 17.000 uu, the recorded rest height, with its
  box clear of the floor.
- Full throttle from rest reaches ~414 uu/s in 0.3 s, the speed-taper value;
  before, the box's floor contact cost ~20%.
- Landings are absorbed by springs and dampers instead of a box bounce, and
  the ball meets the box where the real hitbox is.

### Negative / tradeoffs

- Wheels are cast against the floor plane only, as in FR-088; walls, curves
  and the ceiling still meet the box.
- Brake is still applied at the centre of mass, not per wheel.
- Cars from `car_box` (most unit tests) have no offset; with their box on
  the floor their suspension is near its rest length and adds almost
  nothing.
- A grounded car now feels the sticky force; a few jump tests' expected
  velocities account for one tick of it.

## Validation and revisit triggers

- `world` test `a_standard_car_settles_on_its_suspension_at_the_real_ride_height`;
  `drive` tests for the sticky force, no forces airborne, and a spring that
  pushes when compressed and never pulls; `collision` test
  `a_cars_hitbox_offset_moves_where_the_ball_hits_it`.
- Re-run `--self-trace test2.jsonl 4.1 4.3` and `5.5 6.0` and
  `--self-growth`: the rest height should read ~17.0 like the recording,
  the wheels should keep touching until ~4.183 s after the 4.142 s jump,
  and the 5.575 s landing should not bounce.
- Real capture, 2026-10-01: confirmed. Rest height 17.0 as recorded; 0-3 s
  growth 2.2 to 0.02 uu; 0.1 uu / 0.3 uu/s at 4.1-4.133 s. The 5.575 s
  landing still comes ~6 ticks late, because the candidate falls slower
  after the 4.94 s boost, not because of the suspension.
- Revisit for wall and curve driving (wheel rays against every surface).
