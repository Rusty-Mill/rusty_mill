# ADR-0009: Drive the car as one rigid box with direct forces and torques, not a raycast vehicle

- Status: Accepted (recorded retroactively 2026-09-30; decided 2026-08-29, reaffirmed 2026-09-01)
- Date: 2026-08-29
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-007, FR-008, FR-009, FR-057, FR-063,
  FR-065, FR-066, FR-067, FR-005; ADR-0003, ADR-0004
- Supersedes/Superseded by: steering partially superseded by ADR-0011

## Context

Real Rocket League drives cars through `btVehicleRL`, an extension of
Bullet's raycast vehicle. Steering sets a wheel steer angle from
`STEER_ANGLE_FROM_SPEED_CURVE`, and per-wheel lateral tire friction
(`LAT_FRICTION_CURVE`) turns that into motion. Suspension raycasts also
track the driving surface, which is what keeps a car oriented on walls.

This port's car (`drive::apply_driven_forces`) is one rigid box:
- throttle is a central force along local forward, tapered by speed;
- steering is a yaw torque about local up, scaled by speed;
- handbrake multiplies the contact solver's existing Coulomb friction;
- throttle, steering and handbrake apply only while grounded.

FR-007 to FR-009 chose this without comparing it to a raycast vehicle.
The FR-065 audit made the comparison explicit and kept the box model.

## Decision drivers

- The box reuses the rigid-body, contact and friction machinery already
  ported (ADR-0004), with no second grip model.
- A raycast vehicle is a substantially larger architecture change:
  suspension raycasts, per-wheel state, and a tire friction model with
  its own calibration curves.
- FR-065 found that the real steering curve does not transfer to a
  direct-torque model. Reversing `speed_factor` alone would "substitute
  one unconfirmed guess for another".
- Fidelity is measured by divergence (ADR-0003). The model should change
  when data shows the car model is the dominant error, not before.

## Considered options

1. **One rigid box with direct forces and torques** (chosen).
2. **A port of Bullet's raycast vehicle with RocketSim's curves**: closest
   to real Rocket League, but a new subsystem, and most of its curves need
   calibration this project cannot yet do (FR-005 has not started).
3. **A hybrid**: keep the box, add a front/rear friction split or a
   slip-angle-driven lateral force. Listed as future work, to revisit
   once real recorded drift behavior exists.

## Decision

Keep the single rigid box with direct forces and torques (option 1).
- Real Rocket League values are adopted where they transfer:
  - the throttle speed-taper curve's shape, from RocketSim `RLConst.h`;
  - `UNBOOSTED_MAX_CAR_SPEED = 1410`;
  - `MAX_CAR_SPEED = 2300`.
- The rest are documented as uncalibrated placeholders (all in
  `drive::ground` except `AIR_CONTROL_TORQUE`, which is in `drive::air`):
  - `THROTTLE_ACCELERATION = 1600`;
  - `STEER_TORQUE = 1_500_000`;
  - `HANDBRAKE_FRICTION_MULTIPLIER = 0.1`;
  - `AIR_CONTROL_TORQUE`.

## Consequences

### Positive

- The whole car model is small and fully unit-tested, and each mechanic
  is one function in `drive::{ground,air,jump,boost}`.
- There are no parallel grip or suspension systems to keep consistent
  with the contact solver.

### Negative / tradeoffs

- The steering curve runs the opposite way to real Rocket League's:
  - the port: zero torque at a standstill, rising to full at 2300 uu/s;
  - real Rocket League: the steer angle falls from 0.534 rad at 0 uu/s to
    0.035 rad at 3000 uu/s.
- Handbrake friction is isotropic. Real Rocket League is anisotropic
  (lateral x0.1; longitudinal x0.5 to x0.9). Splitting it would touch 5
  solver functions (FR-066).
- There is no suspension-driven surface tracking, so wall driving
  orientation differs (FR-067).
- Several constants have a real reference that does not transfer to this
  model, which limits how far FR-005 calibration can go.
- This is among the likeliest sources of the large divergence FR-077
  measured.

## Validation and revisit triggers

- Revisit when `rb-verify --self-growth` on a real capture shows early,
  abrupt divergence during ground driving, turning or powersliding.
- Revisit when FR-005 calibration of `STEER_TORQUE` or
  `THROTTLE_ACCELERATION` cannot fit recorded data within tolerance. That
  is evidence the model, not the constant, is wrong.
