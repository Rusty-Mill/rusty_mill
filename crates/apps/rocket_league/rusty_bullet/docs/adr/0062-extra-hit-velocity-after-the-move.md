# ADR-0062: The extra ball-hit velocity is added after the ball moves

- Status: Accepted
- Date: 2026-10-06
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-137, FR-107, FR-129, RB-RESEARCH-O010, ADR-0061
- Supersedes/Superseded by: none (corrects the placement in FR-107)

## Context

The game's car-ball hit keeps lifting the car for three ticks after the ball
has left (`RB-RESEARCH-O010`). A parameter sweep (ADR-0061) found no setting
that passed the full-capture gate, so the cause was structural. Reading the
recorded hit tick by tick (`hitjump.jsonl` 77.642-77.76 s) showed the ball
moves only 8.3 uu on the hit tick in the game against 16.6 in the port,
with the same post-hit velocity. `Ball::_FinishPhysicsTick` in RocketSim
adds the extra hit velocity after Bullet has integrated the step; the port
added it before integrating.

## Decision drivers

- One contact tick moves the ball half a tick too far, which puts it out of
  reach of the wheel rays that the game's ball still touches next tick.
- The change is a reordering inside one function, with no new parameter.
- It must pass the full-capture k = 30 table, not a window.

## Considered options

1. Keep the order and tune the hit (rejected by the sweep, O010).
2. Add the extra velocity after the ball's transform integrates, then apply
   the speed caps.
3. Also move the speed caps back to before the integration only (a
   variant); no evidence either way, caps rarely bind.

## Decision

Option 2: after `integrate_transform_and_refresh_inertia(ball)`, add
`ball_hit_velocity` and clamp. Cars integrate as before.

## Consequences

### Positive

- `test2` k = 30 ball distance 0.54 -> 0.17 uu; `hitjump` car 10.60 ->
  10.49 uu/s and ball 92.95 -> 92.86; `front` and `side` unchanged.
- The `car_over_ball` hit tick now matches the game's ball position to
  under 1 uu in each axis.

### Negative / tradeoffs

- The car's push after the hit is front-loaded (+129 uu/s then +19, against
  +40, +30, +23): the ray-ball suspension force is not yet right.
- The speed caps now apply after the move; a ball above the caps moves once
  with the uncapped velocity. Rare, and RocketSim's order.

## Validation and revisit triggers

- Test `the_extra_hit_velocity_changes_the_ball_after_it_has_moved_not_before`.
- Revisit with the next capture of a car hitting the ball (the tape-bot
  `car_over_ball` run on the game machine) for the remaining push shape.
