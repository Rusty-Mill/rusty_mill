# ADR-0026: Car-ball and car-car contacts use RocketSim's materials; the ball gets Psyonix's extra hit velocity

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-107, FR-063, FR-043; RB-VERIFY-003-FR-007;
  ADR-0021, ADR-0024
- Supersedes/Superseded by: closes FR-063's open car-car/car-ball overrides

## Context

After ADR-0025 the next worst one-step errors in `test2.jsonl` were ball
hits: the kickoff (5.758 s, 155 uu/s) and 12.267 s (107 uu/s). The port
combined the car's and ball's placeholder coefficients for those contacts.
RocketSim overrides them in `Ball::_OnHit` (friction 2.0, restitution 0.0)
and adds Psyonix's extra ball-hit velocity: along the car-to-ball offset
with its height scaled by 0.35 and its part along the car's forward axis by
0.65, sized by the relative speed (capped at 4600) times a speed curve,
added to the ball at the end of the tick, at most every other tick per car.
Car-car contacts use friction 0.09, restitution 0.1. `--self-onestep`
reported only the car, so the ball's side of a hit was invisible.

## Decision

- `solver::resolve_manifolds` takes an optional `PairMaterial` per
  dynamic manifold, used as is instead of combining the two bodies'
  coefficients. `PhysicsWorld` gives car-ball manifolds
  `CAR_BALL_MATERIAL` (0.0 / 2.0) and car-car manifolds `CAR_CAR_MATERIAL`
  (0.1 / 0.09).
- `PhysicsWorld::step` adds `extra_ball_hit_velocity` for each car touching
  the ball, computed from the states at contact detection and added after
  the solve, before the speed caps, with RocketSim's one-tick cooldown.
- `TraceRow` carries the recorded and candidate ball; `--self-onestep`
  prints the ball's velocity error per row.
- Car-car bumps and demolitions stay unmodeled (no multi-car capture).

## Consequences

- `test2.jsonl` one-step car error: 5.758 s 21 uu/s (was 155), 12.267 s 15
  (was 107), mean 1.67 (was 1.78). Ball error on those hits 129 and 90 uu/s
  (1192 and 970 without the extra velocity), 14.783 s under 20 (was 672).
- The remaining ball errors are ball-world contacts (7.95 s 250 uu/s,
  13.89 s 1072). The port averages static materials (FR-043) where Bullet
  multiplies them, which may matter there.
