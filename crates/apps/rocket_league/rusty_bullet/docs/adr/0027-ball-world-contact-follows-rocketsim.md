# ADR-0027: Ball-world contacts fold into one velocity-only contact; no speculative term

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-108, FR-043, FR-048, FR-107; RB-VERIFY-003-FR-007;
  ADR-0026
- Supersedes/Superseded by: replaces FR-043's averaged ball-world material
  with RocketSim's combine rule

## Context

After ADR-0026 the worst one-step ball errors in `test2.jsonl` were
ball-world contacts: 13.892 s (1072 uu/s, the ball on two ramp facets and
the floor at once) and 7.95 s (~250). The port solved each ball-world
contact on its own with position correction, so a ball touching several
triangles bounced once per triangle. RocketSim's Bullet marks ball-world
contacts "special": `convertContactSpecial` folds them into one contact
per body at the average normal (not renormalized) and the average
distance, solved for velocity only. Its arena material is restitution 0.3,
friction 0.6, combined with the ball's as max / min. It also comments out
Bullet's speculative `velocityError -= penetration * invTimeStep` ("it
ruins ball bounces at low velocities").

## Decision

- `PhysicsWorld::step` folds every ball-world contact into one
  (`combined_ball_world_contact`, `penetration_depth` 0 so no position
  correction) with `ball_world_material` (restitution max(ball, 0.3),
  friction min(ball, 0.6)). Opposite contacts that cancel give none.
- `solver::setup_rows` and `setup_two_body_rows` drop the speculative
  term for every contact, as RocketSim does.
- `friction_directions` normalizes the normal before `plane_space`, since
  an averaged normal is not unit length.
- `--self-onestep` rows print the ball's recorded and simulated velocity.

## Consequences

- `test2.jsonl` one-step ball error: mean 0.41 uu/s (was 1.00), max 252
  (was 1072); 13.892 s under 20. `front` car mean 0.302 (was 0.385).
- `test2` car mean 1.72 (was 1.67): 9.067-9.125 s after the corner impact
  rise to 45-50. Kept, since it is what RocketSim does.
- An embedded ball is no longer pushed out of the arena; tests that relied
  on that now drive the ball into the surface. A dropped bare box can rest
  up to ~1 uu above the floor (a manifold point inside its threshold stops
  the approach).
- Remaining ball errors: 13.967 s (252), 7.95 s (201). RocketSim's
  `m_erp2 = 0.8` (the port uses 0.2) is not applied yet.
