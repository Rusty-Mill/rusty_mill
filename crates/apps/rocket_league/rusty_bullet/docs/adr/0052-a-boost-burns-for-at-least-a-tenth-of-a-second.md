# ADR-0052: A boost burns for at least a tenth of a second

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-133, FR-056, FR-096
- Supersedes/Superseded by: —

## Context

`test2.jsonl` 8.283-8.325 s: boost held for six ticks, then released. For six
more ticks the recorded car kept the air boost acceleration along its
forward axis; the sim, stopping with the button, lacked exactly 3175/3
uu/s^2 for those ticks (8.8 uu/s per tick, along forward, spin exact).
RocketSim's `Car::_UpdateBoost` keeps boosting until `BOOST_MIN_TIME` =
0.1 s of the burn has elapsed.

## Decision

- `boost::update_boosting(held, has_boost, boosting, boosting_time, dt)`:
  boosting = has fuel and (held, or already boosting with the burn under
  0.1 s); the burn timer resets when it stops.
- `apply_boost` acts on that result: force and drain follow `boosting`, not
  the button. Throttle forcing (`effective_throttle`, `air_throttle`) still
  reads the button, as RocketSim does (`controls.boost`).

Alternatives considered: counting minimum time in the verifier from the
recorded input. Rejected: it is game physics, not input handling.

## Consequences

- `test2` one-step error over 8.342-8.383 s 8.8 -> 0; k = 30 car 2.55 ->
  2.12. Other captures unchanged. Fuel drain also follows the burn, so a tap
  now costs 0.1 s of fuel.
