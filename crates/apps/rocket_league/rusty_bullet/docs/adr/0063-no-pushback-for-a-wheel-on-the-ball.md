# ADR-0063: A wheel ray that hits the ball gets no pushback

- Status: Accepted
- Date: 2026-10-06
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-138, FR-129, FR-137, RB-RESEARCH-O010, ADR-0048, ADR-0062
- Supersedes/Superseded by: none (narrows FR-129)

## Context

After ADR-0062 the car's lift after a ball hit was still front-loaded: +129
uu/s the tick after, where the game gives +40, +30, +23. Tracked runs (the
environment snapped to `hitjump`, parts left free) showed the wheel-ray
force right when fed the game's states, and a cliff otherwise: lowering the
free ball by 0.5 uu moved the push from 535 to 441 uu/s (game 445). The wheel
code has one hard threshold, the pushback (`trace < pushback_reach`): an
impulse that cancels the car's approach speed to the surface plus
`PUSHBACK_ERP` of the penetration, treating the surface as still and
unpushable. FR-129 let the ball's rays use it unchanged.

## Decision drivers

- Pushback is Bullet's response against a static surface; it takes no
  impulse and ignores the surface's velocity. A ball moving away at 1700 uu/s
  breaks both assumptions.
- The change must show on the full-capture k = 30 table, not only the hit
  window.

## Considered options

1. Leave pushback on for the ball's rays (FR-129 as written).
2. Skip pushback for rays that hit a dynamic body; spring and damper stay.
3. Scale pushback by the ball's relative speed (a new model, no evidence).

## Decision

Option 2. `RayHit` gains `dynamic` (true only for the ball's ray); `cast_wheel`
computes pushback only when it is false.

## Consequences

### Positive

- The `car_over_ball` free run's lift after the hit is 400, 436, 463, 483,
  475, 469 uu/s against the game's 405, 445, 475, 498, 493, 488 (was 400,
  529, 548, 540, 534, 529).
- `hitjump` 77.6-77.8 s window car error 25.3 -> 11.0 uu/s; the four k = 30
  scores are unchanged (`hitjump` car 10.49 -> 10.47).

### Negative / tradeoffs

- A wheel resting on a ball for long (a dribble) now has spring and damper
  only; no capture has one, so that case is unverified.
- Slightly under the game still (+36, +27, +20 against +40, +30, +23).

## Validation and revisit triggers

- Test `a_wheel_ray_that_hits_the_ball_gets_no_pushback`.
- Revisit with the first capture of a dribble or a flip reset, where a wheel
  sits on the ball for many ticks.
