# RB-PHYSICS-001 — Physics Core Port

- Version: 0.2.0
- Status: In Progress (v0: sphere-vs-plane implemented and tested; box
  bodies, general inertia, split impulse, warm-starting are open follow-up
  work, not yet started)
- Owners: baileyrd
- Depends on: RB-VERIFY-003
- Supersedes: none

## Purpose and scope

Define and implement the physics core that produces a simulated
`PhysicsFrame` sequence for `RB-VERIFY-003` to score, per
[ADR-0004](../../adr/0004-bullet3-source-port-for-physics-core.md): a
from-scratch Rust port of specific Bullet3 (zlib-licensed) algorithms —
rigid-body integration and the sequential-impulse contact solver — not an
integrated third-party engine and not an unguided from-scratch design.

**v0 scope** (implemented, in `crates/rb_physics_bullet`): a dynamic sphere
(the ball) against a static plane (the ground). Gravity, damping,
semi-implicit Euler velocity integration, exponential-map orientation
integration, analytic sphere-vs-plane contact detection, and a
sequential-impulse solver with restitution and Coulomb friction (two
tangent directions).

## Non-goals (this increment)

- **Car bodies.** No box shape, no general 3x3 inertia tensor — a sphere's
  isotropic inertia (scalar) is what v0's solver assumes throughout. Adding
  car boxes is real follow-up work (`RB-PHYSICS-001-FR-004` below), not a
  small extension of the current solver.
- **Split impulse.** v0 always takes Bullet's non-split contact-resolution
  branch (position and velocity correction combined into one `rhs`). See
  `rb_physics_bullet::solver`'s module doc for what this trades away.
- **Warm-starting and sleeping.** Every contact's impulses are re-derived
  from zero each frame. Documented consequence: a bouncy (restitution > 0)
  resting contact never truly settles under v0's solver — see
  `rb_physics_bullet::solver`'s module doc and
  `world::tests::resting_ball_stays_at_rest`.
- **Consuming recorded inputs.** v0 simulates the ball in isolation from
  its own initial state; there's no car to receive throttle/steer/boost
  input yet, so `simulate()` doesn't take an input sequence (see
  `RB-PHYSICS-001-FR-001` below for exactly what's implemented instead).
- **Calibrated constants.** Gravity (-650 uu/s^2), restitution, and
  friction defaults are placeholders (commonly-cited community estimates
  or reasonable guesses), not confirmed against real Rocket League data —
  see `RB-PHYSICS-001-FR-005`.

## Context and terminology

- **Physics core**: `rb_physics_bullet`'s `PhysicsWorld` — whatever
  produces a simulated `PhysicsFrame` sequence, the thing `RB-VERIFY-003`
  scores.
- **Port** (as in "ported from Bullet3"): a from-scratch Rust translation
  of Bullet3's algorithms, not a binding or vendored build — see
  `THIRD_PARTY_NOTICES.md`.

## Requirements

- `RB-PHYSICS-001-FR-001` (implemented): `rb_physics_bullet::simulate`
  given a `PhysicsWorld` (initial sphere + plane state), a duration, and a
  fixed timestep, produces a `Vec<PhysicsFrame>` `RB-VERIFY-003::score` can
  consume directly.
- `RB-PHYSICS-001-FR-002` (implemented): Rigid-body integration
  (`integrate` module) ports `btRigidBody::applyGravity`/`applyDamping`/
  `integrateVelocities` and `btTransformUtil::integrateTransform`'s
  exponential-map orientation update.
- `RB-PHYSICS-001-FR-003` (implemented): Sphere-vs-static-plane contact
  detection and resolution (`collision`, `solver` modules) — restitution
  via `restitutionCurve`, Coulomb friction via two tangent constraint rows
  clamped to the current normal impulse, matching
  `btSequentialImpulseConstraintSolver`'s structure.
- `RB-PHYSICS-001-FR-004` (open): Extend to box-shaped car bodies — general
  3x3 inverse inertia tensor, box-vs-plane and box-vs-sphere collision
  (SAT or GJK/EPA), multi-contact manifolds (a car can touch the ground on
  more than one wheel-adjacent point at once).
- `RB-PHYSICS-001-FR-005` (open): Calibrate gravity/restitution/friction
  constants against real recorded ground truth once `RB-VERIFY-001`/
  `RB-VERIFY-002` produce real data, rather than relying on the current
  placeholder defaults.
- `RB-PHYSICS-001-NFR-001` (implemented): The physics core doesn't force
  Bullet-specific data modeling into `rb_domain` — `rb_domain::state`
  stays a plain state DTO plus general-purpose vector/quaternion algebra;
  `rb_physics_bullet` owns all rigid-body/solver-specific types.

## Architecture and interfaces

`rb_physics_bullet` (new crate, depends only on `rb_domain`):
- `body`: `Sphere` (dynamic), `StaticPlane` (immovable).
- `integrate`: force accumulation, velocity integration, transform
  integration — pure functions over `Sphere`.
- `collision`: `sphere_vs_plane` — analytic contact generation.
- `solver`: sequential-impulse contact + friction resolution.
- `world`: `PhysicsWorld::step`/`frame`, and `simulate()` — the
  composition root Bullet's `btDiscreteDynamicsWorld::stepSimulation`
  corresponds to.

No `PhysicsStateSource`-style trait exists yet for "the physics engine"
specifically — `rb_verify_cli` calls `rb_physics_bullet::simulate`
directly. A trait is worth introducing once a second physics core
implementation actually exists to justify it (per the "no speculative
abstraction before two real call sites" convention this project follows
throughout) — not before.

## Data/state and invariants

World convention: +Z is up (matching Unreal Engine, which Rocket League
runs on). Sphere inertia is always isotropic (`I = 2/5 m r^2`); this stops
being valid the moment a non-spherical body is added (`RB-PHYSICS-001-FR-004`).

## Errors, failure, recovery, and observability

No fallible operations in v0 — `Sphere::new` panics on non-physical input
(zero/negative mass or radius), matching "trust internal callers, validate
at real boundaries" (a physics body's own constructor is such a boundary;
a malformed body is a programming error, not a recoverable runtime
condition).

## Security, privacy, and compatibility

None beyond `THIRD_PARTY_NOTICES.md`'s zlib attribution obligations.

## Acceptance criteria

- v0 (met): free-fall matches semi-implicit Euler kinematics before
  impact; an inelastic resting contact stays at rest; a dropped ball
  settles near the ground; restitution produces a bounce proportional to
  the combined coefficient; friction decelerates a sliding sphere and
  couples into spin. All covered by `rb_physics_bullet`'s unit tests
  (26 tests as of this version).
- FR-004/FR-005 (open): acceptance criteria defined when that work starts.

## Verification plan

Unit tests (existing) for physical sanity; `RB-VERIFY-003` divergence
scoring against real replay/BakkesMod ball trajectories once
`RB-VERIFY-001`/`RB-VERIFY-002` exist — that comparison is what actually
validates (or invalidates) the placeholder constants and v0's fidelity to
Rocket League's real ball behavior, not the unit tests alone.

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- FR-004 and FR-005 above.
- Restitution/friction combine mode (`rb_physics_bullet::solver` currently
  averages; Bullet's actual default is `max` for both) — revisit once real
  data exists to calibrate against.
- No-split-impulse and no-warm-starting/sleeping are documented, deliberate
  v0 gaps (see Non-goals) — worth revisiting once car bodies make resting
  multi-contact stability actually matter (a ball alone tolerates it; a
  car resting on 4 wheels may not).

## Change history

- 0.2.0 (2026-08-28): v0 implemented — sphere-vs-static-plane rigid body
  integration and sequential-impulse contact solver, ported from Bullet3
  per ADR-0004. Resolves the "build-vs-integrate" framing from this spec's
  0.1.0 open questions in favor of a direct source port.
- 0.1.0 (2026-08-28): Placeholder created at bootstrap; full spec deferred
  to Phase 1 start.
