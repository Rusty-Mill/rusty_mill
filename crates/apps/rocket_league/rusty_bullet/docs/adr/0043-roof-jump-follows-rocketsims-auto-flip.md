# ADR-0043: A car on its roof pops up and rolls over on a jump press, as RocketSim's auto-flip

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-123, FR-077, FR-088; ADR-0042
- Supersedes/Superseded by: —

## Context

`hitjump.jsonl` 279.742–279.750 s: the car lies on its roof (up axis
(0, 0, −1)) sliding at 514 uu/s. Jump is pressed and the next tick it
gains 207 uu/s upward and 4.6 rad/s of roll about its forward axis; the
roll then holds at about 5 rad/s. The simulator did nothing: no wheel
touches, so the jump is not a ground jump, and the press was 205 uu/s
off, the largest car error in the recordings after the resets.

RocketSim models this as `Car::_UpdateAutoFlip`: on a fresh jump press
while the car's body touched a surface last tick (`worldContact`, any
static contact, normal `z > 1/sqrt 2`) and the car is rolled past 2.8
rad, it applies `CAR_AUTOFLIP_IMPULSE` (200 uu/s) along minus the car's
up axis and adds `CAR_AUTOFLIP_TORQUE` (50 rad/s^2) about the forward
axis, toward upright, for `CAR_AUTOFLIP_TIME * |roll| / pi` seconds
(0.4 s times the fraction of a half turn). Its roll is minus Bullet's
`getEulerYPR` roll, `-atan2(right.z, up.z)`, +3.10 rad for the recorded
orientation, so the torque runs along +forward, as recorded.

## Decision

- `drive::jump::auto_flip` ports `_UpdateAutoFlip` as written; it runs
  every tick after the jump hold, on the ground or not.
- `DriveState::world_contact_normal` is RocketSim's `worldContact`: the
  normal of the last static contact the car's body made in the previous
  step, set by `PhysicsWorld::step` from the car's static manifolds.
  `DriveState::auto_flip` holds the running flip.
- One recorded sample; the port is taken from the source, not fitted.

Alternatives considered:
- An instant roll impulse of 4.6 rad/s, which is what the recording
  shows on the press tick. One sample is not enough to replace a sourced
  model, and RocketSim's own comment calls its accuracy a TODO; noted
  for a capture with several roof jumps.

## Consequences

- `hitjump` 279.750 s: car velocity error 12.6 uu/s (was 205); the pop
  reads 182 against 195 recorded. The spin error on the press tick stays
  at 4.1 rad/s (RocketSim's torque gives 0.42 rad/s in one tick, the
  game 4.6); the ticks after it improve (279.758 s spin 0.73, was 0.97;
  279.767 s 0.15, was 0.32), since the torque keeps the roll up as the
  game does.
- `hitjump` one-step car 0.489 (was 0.494); `test2`, `front` and `side`
  unchanged.
- A car upside down in the air gets no pop: the surface contact is
  required.
