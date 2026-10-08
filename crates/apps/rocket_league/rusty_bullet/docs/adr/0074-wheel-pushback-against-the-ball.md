# ADR-0074: A wheel's pushback against the ball reads the ball's velocity

- Status: Accepted
- Date: 2026-10-08
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-150`, amends FR-138 and ADR-0063; `RB-RESEARCH-O014`, O010; `docs/roadmap/PARITY-PLAN.md` (workstream B)
- Supersedes/Superseded by: amends ADR-0063 (a wheel on the ball gets no pushback)

## Context

FR-138 removed the wheel pushback against the ball: with the ball treated as a
still, unpushable surface, a car at 2000 uu/s over a ball leaving at 1700 uu/s was
pushed +129 uu/s where the game's gains +40. A clean set of drops of a level car onto a
resting ball (`wob_*`, repeatable to 0.0 uu in the game) shows the other side of it: a
car held up by its wheels on the ball sags in the port, with a steady +2.5 uu/s per tick
missing (about 300 uu/s^2), and the car mean error is 28.2 uu over the nine drops.

## Decision drivers

- Keep `car_over_ball` (a fast pass) and the 62 golden captures where they are.
- Bullet's vehicle code applies its collision response to the car only.
- Prefer the smallest change that the data supports.

## Considered options

1. Keep FR-138 (no pushback on the ball).
2. Pushback against the ball as a still surface (undoes FR-138; `car_over_ball` 33.6 -> 62.5 uu).
3. Pushback with the ball's velocity at the hit subtracted from the car's approach speed.
4. Option 3 with the ball's inverse mass in the denominator (car 28.9 uu, worse).
5. Option 3 with the opposite impulse on the ball (car 59.7 uu, ball 82.5 uu, much worse).

## Decision

Option 3. `RayHit` carries the surface velocity (zero for the arena, the ball's linear
velocity plus its spin at the point for the ball) and `stopping_impulse` subtracts it
from the car's speed at the wheel. The ball takes no reaction.

## Consequences

### Positive

- Car mean error on the nine drops 28.2 -> 21.8 uu (every drop equal or better);
  `car_over_ball`, the golden gate and `--self-kstep` unchanged.
- Three of the drops are golden fixtures (`wob_0_200`, `wob_0_500`, `wob_100_200`).

### Negative / tradeoffs

- The ball error is 2 percent worse on average (67.8 -> 69.3 uu) and worse on three
  off-centre drops (`wob_50_200` 22 -> 50, `wob_50_500` 6 -> 18, `wob_50_1000` 60 -> 94).
- Off-centre drops are still 14 to 45 uu off the game: not parity (O014 stays open).

## Validation and revisit triggers

- `drive` tests: a still ball gives the floor's pushback; a ball leaving as fast as the
  car closes gives a fifth or less; a rising ball gives more.
- Revisit when the off-centre drops are explained (the ball's side of the contact).
