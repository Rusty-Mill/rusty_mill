# ADR-0011: Steer by setting yaw rate from Rocket League's steer-angle curve

- Status: Accepted
- Date: 2026-10-01
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-080, FR-065, FR-079; RB-VERIFY-003-FR-005;
  ADR-0009
- Supersedes/Superseded by: partially supersedes ADR-0009 (the steering
  part only; throttle, boost and handbrake stay as ADR-0009 describes);
  amended by ADR-0012 (zero steer targets zero yaw rate while grounded)

## Context

ADR-0009 kept the car as one rigid box driven by direct forces and
torques, and steering was a speed-scaled yaw torque (`STEER_TORQUE`).
FR-065 had already found this has the wrong shape: real Rocket League
turns tightest from a standstill, while the torque was weakest there.

After FR-079 fixed the driving-car hop, the real-capture trace
(`rb-verify --self-trace test2.jsonl`) made steering the dominant error.
The recorded car turned on steer input at 3.742 s and 4.050 s; the
candidate barely turned, leaving a constant ~0.45-0.50 rad orientation
error by 4.1 s.

## Decision drivers

- Real values exist and are confirmed: RocketSim's
  `STEER_ANGLE_FROM_SPEED_CURVE` and
  `POWERSLIDE_STEER_ANGLE_FROM_SPEED_CURVE` (`RLConst.h`), and the Octane
  wheel positions (`CarConfig.cpp`), giving an 85 uu wheelbase.
- A front-wheel steer angle plus a wheelbase fixes a turn rate
  geometrically (the bicycle model), so the real curve transfers to a
  wheel-less box without an invented constant.
- A raycast vehicle with tire slip (ADR-0009's option 2) is still a large
  subsystem; this change is small and can be checked against the trace.

## Considered options

1. **Set the yaw rate from the steer-angle curve via the bicycle model**
   (chosen): `yaw_rate = forward_speed * tan(steer * max_angle(speed)) /
   WHEELBASE`, applied to the car's angular velocity about its up axis
   while grounded and steering.
2. **Keep a torque, reshaped by the curve**: still needs an invented
   torque magnitude and inertia tuning, the "false precision" FR-065
   warned against.
3. **Port the raycast vehicle and tire-slip model**: the most faithful,
   and still the long-term direction, but much larger.

## Decision

Option 1. `drive::ground::steer_yaw_rate` computes the target yaw rate,
using the powerslide curve while the handbrake is held, and
`apply_ground_control` sets the car's yaw rate to it each grounded tick
with steer input. With no steer input the yaw rate is left to contacts
and momentum. `STEER_TORQUE` is removed.

## Consequences

### Positive

- Steering uses only real Rocket League values: two curves and the
  wheelbase. Nothing is tuned.
- Turning is tightest at low speed and gentle at high speed, as in the
  game, and reverses correctly when driving backward.
- Through a full world step the car turns within 10% of the target rate:
  floor friction does not cancel it (world test
  `a_car_steering_on_flat_ground_turns_at_the_steer_curve_rate`).

### Negative / tradeoffs

- No tire slip: the heading turns at the full geometric rate, an upper
  bound. Real cars slip sideways, so the velocity direction lags the
  heading; this port's velocity follows only through the box's uniform
  floor friction until per-axis tire friction exists (FR-066).
- Setting angular velocity directly is kinematic. A collision during a
  steered turn is resolved after it each tick, so contacts still act,
  but the steering itself is not a force that collisions can resist.

## Validation and revisit triggers

- Re-run `rb-verify --self-trace test2.jsonl 3.7 4.3`: the orientation
  error at 4.0 s should fall from ~0.45 rad. If heading now overshoots the
  recording, tire slip is the next thing to model.
- Revisit when per-axis tire friction lands, or if a raycast vehicle is
  adopted (ADR-0009 option 2).
