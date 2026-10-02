# ADR-0020: Wheels grip and brake whenever they touch

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-099, FR-081, FR-086, FR-088, FR-089, FR-095;
  ADR-0012, ADR-0017
- Supersedes/Superseded by: revises the on-ground gate of FR-081's tire
  model and FR-088's air-control split

## Context

The candidate ran its whole tire model (side grip, engine, brake) only when
at least three wheels touched (`is_on_ground`), applied the brake at the
centre of mass, and ran air control otherwise. The owner's pure front-flip
capture (`front.jsonl`) lands nose first at 19.058 s and coasts on its two
front wheels for three ticks:

| | forward speed per tick | pitch spin per tick |
|---|---|---|
| recorded | -2.2, -2.2 | -0.36, -0.47, -0.62 |
| candidate | 0, 0 | -0.42, -0.56, -0.70 |

RocketSim's `Car::_UpdateWheels` sets every wheel's engine and brake each
tick regardless of contact count (quartering the engine below three wheels),
and `btVehicleRL::calcFrictionImpulses` grips and brakes with each touching
wheel. `_UpdateAirTorque` runs with fewer than three wheels but only does
air control with none touching. Half the coasting brake (525 uu/s^2 over
two of four wheels) is 2.19 uu/s per tick, as recorded; the candidate's
extra pitch loss is air control's damping, which RocketSim skips here.

## Decision

- `ground::apply_ground_control` runs whenever any wheel touches; the
  engine is quartered below three wheels.
- The brake is per wheel: a quarter of the deceleration against each
  contact point's speed along that wheel's in-plane heading, never reversing
  it (`wheel_impulses`), replacing the single centre-of-mass brake.
- Air control needs no wheel touching (on top of FR-095's one-step delay);
  air throttle, jumps and flip torque keep their airborne rules.

## Consequences

- A landing car slows and steers on the wheels that touch first, as in the
  capture; with all four touching nothing changes.
- The brake now acts at the wheels, so uneven contact yaws the car.
- The car body itself is still frictionless against the floor (FR-081);
  the body impact at 19.083 s in `front.jsonl` is a separate, open issue.
