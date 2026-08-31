# RB-PHYSICS-001 — Physics Core Port

- Version: 0.38.0
- Status: In Progress (sphere-vs-plane, box-vs-plane, sphere-vs-box
  (ball-vs-car), box-vs-box (car-vs-car), body-vs-arena-wall, and
  ball-and-car-vs-curved-fillet collision all implemented, tested, and wired into a
  real N-body `PhysicsWorld` scene; ground-driving car input (throttle,
  steering), boost, handbrake, a variable-height ground jump, a double jump
  (plain or a directional dodge, itself flip-cancelable), a wall jump
  (itself dodgeable), air control (pitch/yaw/roll), a gentle landing
  auto-orientation assist, a modeled arena footprint
  (`PhysicsWorld::standard_arena`'s octagonal boundary plus a ceiling), and
  curved fillets throughout the arena's vertical boundary — wall-to-floor/
  wall-to-ceiling seams for all 9 walls (the 4 cardinal walls and, since
  FR-021, the 4 diagonal corner walls too), all 8 of the corner walls' own
  vertical edges where they meet their neighboring side/back walls (since
  FR-022), a spherical patch at each of the 16 compound corners where a
  vertical-edge fillet meets a floor- or ceiling-seam fillet (since
  FR-023), and, since FR-024, an actual goal-mouth window cut into each
  back wall with its own 3 rounded edges per goal, with the 4 diagonal
  corner walls' own floor/ceiling-seam arches and all 16 compound-corner
  fillets that touch them now sized with a distinctly larger,
  non-cardinal-wall radius since FR-025, and, since FR-026, a
  compound-corner fillet rounding each goal's own remaining sharp
  post-crossbar vertex too (4 total, 2 per goal), and, since FR-027, a car
  (box) actually being deflected by every one of those fillets too — a
  car's own box, ignored by every fillet's contact test until now, is
  tested via its 8 corners against the curved surface
  (`collision::box_vs_quarter_pipe`/`box_vs_corner_fillet`, the same
  "test every corner" technique `box_vs_plane` already used for a flat
  plane) — exact, not an approximation, for this containment-style
  contact (`RB-PHYSICS-001-FR-032` rigorously confirmed a once-suspected
  under-detection gap here doesn't actually exist) — implemented,
  and, since FR-028, a car actually driving into a goal too — a new
  `collision::box_vs_goal_wall` tests each of the car's 8 corners against
  the goal-mouth window exactly the same way `sphere_vs_goal_wall`
  already tests the ball's single center point — implemented, and, since
  FR-029, a modeled bounded interior behind each goal window too — a
  solid bounding box, stopping the ball or a car
  that passes through the window instead of letting it sail into
  unbounded open space — implemented, and, since FR-033, a real mass-spring
  net panel per goal catching the *ball* specifically (a car still passes
  through the panel's own footprint untouched, stopped instead by that same
  pre-existing solid bounding box) — implemented; and, since FR-030, every
  ball-vs-car and car-vs-car contact touching in the same step is now
  resolved by one shared, interleaved solve
  (`solver::resolve_dynamic_manifolds`) instead of a sequence of fully
  independent pairwise solves — implemented; and, since FR-031, a
  constant-calibration audit against the RocketSim/RLUtilities/RLBot-wiki
  community reverse-engineering effort corrected `JUMP_SPEED`,
  `JUMP_HOLD_ACCELERATION`, and split `MAX_CAR_SPEED` into separate
  boosted/unboosted caps (`UNBOOSTED_MAX_CAR_SPEED`), confirmed several
  more constants already correct, and explicitly flagged the rest as
  audited-but-still-uncalibrated rather than silently unresolved —
  implemented, and explicitly does NOT close `FR-005`'s real-data
  calibration, still blocked on `PHASE-0-EXIT`; and, since FR-033, each
  goal gets a real mass-spring net panel catching the ball (`net::NetMesh`)
  — implemented, scoped to the ball only (a car still passes through a net
  panel's own footprint, stopped instead by FR-029's pre-existing solid
  bounding box); and, since FR-034, every contact's positional/penetration
  correction runs on its own separate split-impulse "push" channel instead
  of folding into the body's real velocity, so resolving deep overlap no
  longer injects the spurious velocity a combined Baumgarte term used to —
  implemented; and, since FR-035, `solver::resolve_dynamic_manifolds`
  (every ball-vs-car/car-vs-car manifold) warm-starts each call from the
  previous one's converged impulses instead of zero, converging measurably
  closer to the true answer for an under-converged manifold like FR-030's
  own extreme-mass-ratio example — implemented for that one call site;
  and, since FR-036, real source-level research (RocketSim, RLUtilities,
  and the RLBot wiki, read directly rather than guessed at) corrected the
  ball's collision radius (`92.75` to `93.15`, not the real games'
  separate, smaller inertia radius `91.25`, since this port's single
  unified radius field has no room for that distinction) and
  `arena::CEILING_Z` (`2044.0` to `2048.0`, confirmed the same reference
  point as RocketSim's `ARENA_HEIGHT`), and corrected two mis-documented
  claims that `arena::CORNER_LENGTH` and `arena::GOAL_DEPTH` were
  uncalibrated placeholders when both are in fact confirmed exact —
  implemented; and, since FR-037, sleeping
  (`body::RigidBody::update_sleep_state`/`wake`) forcibly zeroes a body's
  velocity once it's stayed below both a linear and an angular threshold
  for a sustained time, fixing the "bouncy resting contact never settles"
  limitation warm-starting alone couldn't (see FR-035's own entry) — a car
  wakes unconditionally on any genuinely active input, before that input's
  own force has had a chance to move it, so an asleep car can always start
  moving again — implemented; and, since FR-039, a wall jump at a corner
  (a car touching two walls at once) pushes off along every touched wall's
  normal summed and normalized, instead of picking whichever wall came
  first in `PhysicsWorld.walls` — implemented; static-contact
  warm-starting, a car's own contact against a net,
  `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS` calibration, and that
  real-data calibration are open follow-up work)
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

**Implemented scope** (in `crates/rb_physics_bullet`): a dynamic sphere
(the ball) and zero or more dynamic boxes (cars), each against a static
plane (the ground), against zero or more arena walls (`PhysicsWorld.walls`
— generic flat `StaticPlane`s, not a modeled Rocket League arena footprint),
and against every other dynamic body in the scene.
Gravity, damping, semi-implicit Euler velocity integration, exponential-map
orientation integration, analytic sphere-vs-plane and box-vs-plane contact
detection (the latter generating a 1-4 point manifold depending on the
box's orientation), analytic sphere-vs-box contact detection (always
exactly one point), a separating-axis box-vs-box contact test (0 to 4
points — a clipped face manifold or a single edge-edge point), and a
sequential-impulse solver with restitution and Coulomb friction (two
tangent directions) — resolving an entire ground-contact manifold together
(`resolve_contacts`) or an entire two-dynamic-body manifold
(`resolve_contacts_between`) — using a general 3x3 inverse inertia tensor
(`RigidBody`/`Mat3`, see Architecture) shared by both shapes.
`PhysicsWorld` carries `cars: Vec<RigidBody>` and, every step, collects
every non-empty ball-vs-car and car-vs-car manifold and resolves all of
them together in one shared, interleaved solve
(`solver::resolve_dynamic_manifolds`, since FR-030), so `box_vs_box` now
runs for real in a live scene, not just in isolation under a unit test.
Each car
also has a current `ControllerInput` (`PhysicsWorld::set_car_input`)
driving ground throttle and steering forces/torques on it (`drive`
module) — see FR-007 — plus a depletable boost resource
(`PhysicsWorld::set_car_boost`) giving it a flat forward force usable in
the air, unlike throttle — see FR-008 — a handbrake that temporarily
reduces its ground friction while held, letting it slide instead of
gripping cleanly through a turn — see FR-009 — a ground jump, fired once
per fresh press — see FR-010 — air control (pitch/yaw/roll torque about
its own local axes while airborne) — see FR-011 — a double jump, an
airborne impulse spendable once per airborne period and restored on
landing — see FR-012 — a wall jump, an outward-plus-upward impulse fired
while touching an arena wall, which also restores the double jump the same
way landing does — see FR-013 — a dodge, a directional variant of the
double jump fired when the stick is held in a direction at the moment of
the press — see FR-014 — variable height on that ground jump, adding
extra upward acceleration for as long as jump stays held, up to a cap —
see FR-015 — flip-cancel, letting a further jump press stop a dodge's
spin early instead of always completing it — see FR-016 — a wall-jump
dodge, the same directional-flip treatment applied to the wall jump's own
fresh press — see FR-017 — a landing auto-orientation assist, a
gentle continuous restoring torque nudging an airborne car's local up
axis back toward world up whenever it isn't actively air-controlling or
mid-jump-press — see FR-018 — a modeled arena footprint,
`PhysicsWorld::standard_arena` building Rocket League's real octagonal
boundary and a ceiling from the same generic `StaticPlane`/`with_wall`
machinery FR-013 introduced, rather than a caller assembling ad-hoc walls
itself — see FR-019 — and curved fillets throughout the arena's vertical
boundary: wall-to-floor/wall-to-ceiling transitions at all 9 walls (the 4
cardinal walls, FR-020, and, since FR-021, the 4 diagonal corner walls
too), all 8 of the corner walls' own vertical edges where they meet their
neighboring side/back walls (since FR-022) — a `StaticQuarterPipe` fillet
each deflecting the ball, and, since FR-027, a car too (via corner-testing,
confirmed exact rather than approximate for this containment-style contact
by `RB-PHYSICS-001-FR-032` — see FR-027) away from the sharp edge a flat wall and the floor/ceiling, or
two flat walls, would otherwise meet at — a `StaticCornerFillet` spherical
patch at each of the 16 compound corners
where a vertical-edge fillet meets a floor- or ceiling-seam fillet, near a
corner wall's own top/bottom endpoint (since FR-023), and, since FR-024,
an actual goal-mouth window — a `StaticGoalWall` — cut into each back
wall, letting the ball, and, since FR-028, a car too, pass straight
through into the goal, with its own 3 rounded edges (two posts and a
crossbar, `StaticQuarterPipe`s again), and, since FR-025, a distinctly
larger `arena::CORNER_ARCH_RADIUS` (instead of the cardinal walls' own
`arena::FILLET_RADIUS`) governing a corner wall's own floor/ceiling-seam
arches and all 16 compound-corner fillets that touch them, and, since
FR-026, a compound-corner fillet rounding each goal's own remaining sharp
post-crossbar vertex too (4 total, 2 per goal), and, since FR-029, a
modeled bounded interior volume behind each goal window too — a solid
bounding box (2 plain back-of-net `StaticPlane`s, 4 `StaticBoundedWall`
side walls reusing the goal posts' own planes, and 2 `StaticBoundedWall`
roofs reusing the crossbar's own plane), stopping the ball or a car that
passes through the window instead of letting it sail into unbounded open
space — see
FR-020/FR-021/FR-022/FR-023/FR-024/FR-025/FR-026/FR-027/FR-028/FR-029.

## Non-goals (this increment)

- **Team structure, car limits, or any Rocket-League-specific scene
  policy.** `with_car` can be called any number of times; this crate
  itself imposes no cap (Rocket League's real max is 8, but that's a
  gameplay/matchmaking rule, not a physics-core one) and has no concept of
  teams — a caller (eventually `rb_verify_cli`, once real multi-car
  recorded data exists) owns that policy.
- **Any geometry finer than a flat plane, single-radius edge fillet, or
  single-radius corner fillet per boundary segment.** (A car actually being
  deflected by a curved fillet
  was this same bullet's other half through FR-026 — that half is now
  resolved, see `RB-PHYSICS-001-FR-027`. A car actually driving into a
  goal was this bullet's third half through FR-027 — that half is now
  resolved too, see `RB-PHYSICS-001-FR-028`. A modeled bounded interior
  volume behind the goal window was this bullet's fourth half through
  FR-028 — that half is now resolved too, see `RB-PHYSICS-001-FR-029`. A
  genuine net *mesh* catching the ball — the "ball tangles in netting"
  behavior — was this bullet's fifth half through FR-029; that half is now
  resolved too, see `RB-PHYSICS-001-FR-033`'s `net::NetMesh` (a real
  mass-spring grid, not FR-029's own solid bounding box), scoped to the
  ball only — a car's own contact against a net, a full 3D "sock" shape,
  and bending stiffness remain open, see FR-033's own Non-goals.)
  `arena::standard_curves`
  builds 24
  `StaticQuarterPipe` fillets — 16 floor/ceiling-seam fillets (one
  floor-side and one ceiling-side per wall, for all 9 walls including the 4
  diagonal corner walls since FR-021) plus, since FR-022, 8 vertical-edge
  fillets (one per corner wall endpoint, where it meets its neighboring
  side/back wall) — `arena::standard_corner_fillets` builds 16
  `StaticCornerFillet`s (since
  FR-023, one per compound corner where a vertical-edge fillet meets a
  floor- or ceiling-seam fillet) — and `arena::standard_goal_cutout_fillets`
  builds 6 more `StaticQuarterPipe`s (since FR-024, two posts and a crossbar
  per goal, rounding the goal-mouth window's own rim) and
  `arena::standard_goal_corner_fillets` builds 4 more `StaticCornerFillet`s
  (since FR-026, one per goal post per goal, rounding each goal's own
  post-crossbar compound corner) — every one of these now deflects the ball
  and, since FR-027, a car too: `collision::contacts_vs_quarter_pipe`/
  `contacts_vs_corner_fillet` dispatch a box to `box_vs_quarter_pipe`/
  `box_vs_corner_fillet`'s corner-testing (testing the box's
  own 8 corners — exact, not an approximation, for this containment-style
  contact, per `RB-PHYSICS-001-FR-032` — see FR-027's own entry) instead of the "always empty" no-op they returned for
  a box through FR-026. `contacts_vs_goal_wall`, unaffected by FR-027 since
  a goal wall isn't a curved fillet, deliberately ignored the goal-mouth
  window for a box too through FR-027 — that gap is now closed by
  `RB-PHYSICS-001-FR-028`'s own `collision::box_vs_goal_wall`, which tests
  each of a car's 8 corners against the window exactly the way
  `sphere_vs_goal_wall` tests the ball's single center point, so a car can
  now drive into a goal in this port too. The goal-mouth
  window itself (since FR-024) opened onto completely open, unbounded
  space through FR-028 — nothing behind it could stop the ball or a car
  that passed through; `RB-PHYSICS-001-FR-029` closes that gap with a
  modeled bounded interior volume (a solid bounding box: 2 back-of-net
  planes, 4 side walls, 2 roofs) — a solid volume standing
  in for the net's functional role of stopping the ball or car, at the time
  deliberately not yet a genuine net mesh; `RB-PHYSICS-001-FR-033` closes
  that gap for the ball specifically with a real mass-spring `net::NetMesh`
  panel in front of that same solid back-of-net plane (a car still passes
  through the panel's own footprint, stopped by the pre-existing solid
  volume instead — see FR-033's own entry). `FR-020`'s fillet
  radius (`arena::FILLET_RADIUS`, also reused by FR-022's vertical-edge
  fillets, FR-024's goal-cutout fillets, and FR-026's goal post-crossbar
  compound-corner fillets; FR-021's corner-wall seams and
  FR-023's compound corners instead reuse FR-025's `arena::CORNER_ARCH_RADIUS`) remains this project's own uncalibrated placeholder, not
  measured against real field mesh data — `SIDE_WALL_X`/`BACK_WALL_Y`/`CEILING_Z`
  are commonly-cited, sourced dimensions, and, since `RB-PHYSICS-001-FR-036`,
  so is `FR-019`'s corner-cut inset distance (`arena::CORNER_LENGTH`),
  confirmed exact against real extracted collision-mesh data rather than
  the uncalibrated placeholder this project previously took it for; `FR-024`'s
  own `arena::GOAL_HALF_WIDTH`/`GOAL_HEIGHT` are commonly-cited too, but
  likewise not independently confirmed. `FR-029`'s own `arena::GOAL_DEPTH`,
  by contrast, is also now confirmed against the current RLBot wiki's own
  cited value (`RB-PHYSICS-001-FR-036`) — this project's earlier claim
  that no reference existed for it at all, making it an uncalibrated
  invention, was itself incorrect. FR-026's 4
  goal post-crossbar compound-corner fillets introduce no new radius
  constant at all — they reuse `FILLET_RADIUS` unchanged. `FR-025`'s
  own `arena::CORNER_ARCH_RADIUS` — the distinctly larger radius now
  governing a corner wall's own floor/ceiling-seam arches and all 16
  compound-corner fillets that touch them, in place of `FILLET_RADIUS` —
  is likewise this project's own uncalibrated placeholder, chosen only to
  read as visibly larger than `FILLET_RADIUS` (enforced at compile time by
  a `const _: () = assert!(CORNER_ARCH_RADIUS > FILLET_RADIUS);` check),
  not measured against real field mesh data either.
- ~~Disambiguating or blending a car's simultaneous contact with two walls
  at a corner, for wall-jump purposes.~~ Physical collision resolution
  already handled a car touching two walls at once correctly — `step`
  resolves every wall independently, so both contacts are resolved on the
  same step regardless of the arena's shape (see FR-013). Which wall's
  normal `drive::apply_driven_forces` uses to decide a wall jump's push-off
  direction when a car is touching more than one wall at a corner at once
  used to pick whichever wall came first in `PhysicsWorld.walls`, not a
  blend of the two normals — a documented simplification for a case
  FR-019's new corner walls made reachable in the standard arena but this
  port's test scenes didn't yet exercise. `RB-PHYSICS-001-FR-039` closes
  that gap: the push-off direction is now every touched wall's normal
  summed and normalized, so a corner wall jump pushes diagonally away from
  the corner instead of along only one of the two walls.
- **Per-axis air-control torque, and any assisted/auto-rotation
  behavior.** FR-011's `AIR_CONTROL_TORQUE` is one shared constant for
  pitch, yaw, and roll; real Rocket League's actual per-axis rates differ
  from each other (roll fastest, pitch and yaw slower), which this port
  doesn't model. Real Rocket League also has an "air roll only" input mode
  and camera-relative stick mapping subtleties — none of that is modeled
  here; this increment is a direct, camera-independent pitch/yaw/roll
  torque, nothing more. (A landing auto-orientation assist is now
  implemented, as a separate, deliberately gentler continuous
  restoring torque rather than an extension of this per-axis input
  torque — see FR-018.)
- **A per-wheel tire/slip model.** Handbrake (FR-009) is modeled as a
  uniform, temporary reduction of the car's single `RigidBody.friction`
  value, not a distinct front/rear grip split or a slip-angle-driven tire
  curve — this port has no wheels at all (the car is one rigid box), so
  there's no rear-specific grip to lose the way a real car's handbrake
  works. See FR-009 and `drive`'s own module doc.
- **Consuming a recorded input sequence.** `PhysicsWorld::set_car_input`
  sets a car's *current* input, persisting until changed — a caller can
  drive a car through a whole `simulate()` run, or update it every step,
  but nothing here yet reads a real `RB-VERIFY-002` capture file
  frame-by-frame to do that automatically; that's `rb_verify_cli`'s
  concern once real capture data exists.
- **Split impulse.** This port always takes Bullet's non-split contact-resolution
  branch (position and velocity correction combined into one `rhs`). See
  `rb_physics_bullet::solver`'s module doc for what this trades away.
- **Warm-starting for `resolve_contacts`/`resolve_contacts_between`.**
  `resolve_dynamic_manifolds` gained warm-starting in
  `RB-PHYSICS-001-FR-035`; the other two paths still re-derive every
  contact's impulses from zero each frame — a difference in convergence
  speed only, not correctness, since this port's fixed `SOLVER_ITERATIONS`
  already fully converges every scenario those two paths cover (see
  `rb_physics_bullet::solver`'s module doc). Sleeping is no longer part of
  this gap — `RB-PHYSICS-001-FR-037` implemented it, fixing the bouncy
  (restitution > 0) resting contact that previously never truly settled
  (see `world::tests::a_bouncy_resting_ball_actually_settles_once_asleep`).
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
- `RB-PHYSICS-001-FR-004` (implemented): Extend to box-shaped car bodies,
  including their collision with the ball. Delivered: a general 3x3
  inverse inertia tensor (`Mat3`, recomputed from orientation each step
  via `RigidBody::update_inertia_tensor`, shared by both sphere and box
  bodies), analytic box-vs-plane contact generation (testing all 8
  corners against the plane — exact for a box vs. an infinite plane, not
  an approximation), multi-contact manifold resolution (the solver
  resolves all of a manifold's 1-4 points together, sharing one
  accumulated velocity delta, rather than one contact at a time), analytic
  sphere-vs-box contact generation (`collision::sphere_vs_box`, a
  closed-form closest-point-on-box query handling both the ordinary
  exterior case and a deep-penetration interior case), and a
  two-dynamic-body manifold solver path (`solver::resolve_contacts_between`)
  that carries both bodies' mass/inertia contributions instead of assuming
  one side is a static plane. `PhysicsWorld::step` now detects and resolves
  a ball-vs-car contact every step a car is present.
- `RB-PHYSICS-001-FR-005` (open): Calibrate gravity/restitution/friction
  constants against real recorded ground truth once `RB-VERIFY-001`/
  `RB-VERIFY-002` produce real data, rather than relying on the current
  placeholder defaults.
- `RB-PHYSICS-001-FR-006` (car-vs-car collision, implemented): A general
  separating-axis test between two oriented boxes (`collision::box_vs_box`),
  producing either a clipped face manifold (0-4 points) or a single
  edge-edge point, reusing the two-body solver path FR-004 introduced
  (`resolve_contacts_between` was generalized from a single contact to a
  manifold for this). `PhysicsWorld` now carries `cars: Vec<RigidBody>`
  (any number, via repeated `with_car` calls) and resolves every car-vs-car
  pair each step, so this pairing runs for real in a live scene — not just
  under a unit test, as it did before multi-car `PhysicsWorld` support
  landed.
- `RB-PHYSICS-001-FR-007` (driven car input, ground throttle and steering,
  implemented): `drive::apply_driven_forces` couples
  `rb_domain::ControllerInput` into forces/torques on a car: throttle
  (accelerate/reverse along the car's local forward axis, capped at
  `MAX_CAR_SPEED`) and steering (yaw torque about the car's local up axis,
  scaled by current speed so a stationary car can't turn in place), both
  gated on the car actually touching the ground. `PhysicsWorld` gains
  `set_car_input` (persists a car's current input across steps) and
  `frame()` now reports each car's actual driving input instead of
  `None`. A car with no input set behaves exactly as before this
  requirement existed (neutral `ControllerInput::default()` applies zero
  force/torque).
- `RB-PHYSICS-001-FR-008` (boost, implemented): `drive::apply_driven_forces`
  also applies a flat forward force (`BOOST_ACCELERATION * mass`, not
  speed-tapered like throttle) along the car's local forward axis whenever
  `ControllerInput.boost` is set and the car has boost remaining, capped at
  the same `MAX_CAR_SPEED` ceiling as throttle. Unlike throttle and
  steering, boost is *not* gated on ground contact — it's modeled as a
  rocket, not an engine, so it works identically airborne. Boost is a
  depletable resource: `PhysicsWorld` gains a parallel `car_boost: Vec<f32>`
  (initialized to a full tank, `drive::MAX_BOOST`, by `with_car`) and
  `set_car_boost` to set it directly; holding boost input drains the tank
  at `BOOST_CONSUMPTION_RATE` per second whenever held, even if the forward
  force itself doesn't apply because the car is already at `MAX_CAR_SPEED`
  (matching real Rocket League's "holding boost drains fuel regardless of
  whether it's still accelerating you"), and the tank clamps at zero
  (no effect once empty). `frame()` now reports each car's actual
  `boost_amount` instead of a hardcoded `0.0`.
- `RB-PHYSICS-001-FR-009` (handbrake, implemented): `drive::apply_driven_forces`
  temporarily multiplies the car's `RigidBody.friction` by
  `HANDBRAKE_FRICTION_MULTIPLIER` (an uncalibrated placeholder) whenever
  `ControllerInput.handbrake` is held and the car is grounded, restoring it
  to the car's own base friction otherwise — gated on ground contact like
  throttle/steering (a free-floating box has no wheels to lock regardless).
  `PhysicsWorld::with_car` snapshots each car's constructed `friction` into
  a new parallel `car_base_friction: Vec<f32>` so handbrake has the car's
  own value, not a hardcoded default, to restore to on release. This models
  handbrake as a temporary grip reduction — letting the car's existing
  momentum carry it into a slide rather than tracking a new heading
  cleanly — reusing the ground-contact solver's existing Coulomb-friction
  machinery rather than a separate lateral-slip system (this port has no
  per-wheel tire model to build a real rear-grip-loss mechanic on top of;
  see Non-goals).
- `RB-PHYSICS-001-FR-010` (single ground jump, implemented):
  `drive::apply_driven_forces` applies a fixed `JUMP_SPEED` instantaneous
  upward velocity change (via `RigidBody::apply_impulse`, not a continuous
  force) on the *rising edge* of `ControllerInput.jump` while the car is
  grounded — a fresh press, not merely "held"; holding the button through
  the resulting airborne period doesn't re-fire it, and releasing then
  re-pressing while still airborne doesn't fire it either (no double jump
  in this scope). Edge detection needs one bit of state per car,
  remembering "was jump held as of the previous step" — `PhysicsWorld`
  gains a parallel `car_jump_held: Vec<bool>` (initialized `false` by
  `with_car`) threaded into `apply_driven_forces` as `jump_held`, the same
  pattern `boost_amount` already uses for cross-call state. A second
  airborne jump and wall jump are explicitly out of scope for this
  requirement — see FR-012. Variable jump height (holding for a higher
  jump) was originally out of scope here too, but is now implemented as
  FR-015.
- `RB-PHYSICS-001-FR-011` (air control, implemented):
  `drive::apply_driven_forces` applies torque about the car's local right,
  up, and forward axes, scaled directly by `ControllerInput.pitch`/`yaw`/
  `roll` (each an `Option<f32>`, `None` treated as zero) times one shared
  `AIR_CONTROL_TORQUE` constant, gated on the car *not* touching the
  ground — the mirror image of throttle/steering/handbrake/jump's
  ground-only gating, so it never competes with ground steering for the
  yaw axis. Unlike ground steering, air control is not speed-scaled: a
  car can spin from a standing start in the air, since there's no wheel
  grip requiring momentum. `AIR_CONTROL_TORQUE` is an uncalibrated
  placeholder shared by all three axes — a documented simplification,
  since real Rocket League's pitch/yaw/roll rates differ from each other
  — see Non-goals.
- `RB-PHYSICS-001-FR-012` (double jump, implemented):
  `drive::apply_driven_forces` fires one additional, identical `JUMP_SPEED`
  instantaneous upward velocity change on a fresh (`jump_pressed`) press of
  `ControllerInput.jump` while the car is airborne — reusing the ground
  jump's own rising-edge detection rather than a second edge-detector, and
  reusing `JUMP_SPEED` itself rather than a separately-calibrated constant
  (this port has no public reference for a distinct double-jump speed
  either). Gated on a new per-car `double_jump_available` flag instead of
  on ground contact: landing (any step where `on_ground` is true)
  unconditionally restores it to `true`, and a fresh airborne press that
  fires the double jump sets it to `false` until the next landing, so it
  can fire at most once per airborne period regardless of how many times
  jump is released and re-pressed after that. `PhysicsWorld` gains a
  parallel `car_double_jump_available: Vec<bool>` (initialized `true` by
  `with_car`, matching a car that's effectively "just landed" before its
  first step) threaded into `apply_driven_forces` alongside `jump_held`.
  Deliberately excludes the directional "dodge" impulse/torque a real
  double jump pairs with — see Non-goals.
- `RB-PHYSICS-001-FR-013` (arena walls and wall jump, implemented):
  `PhysicsWorld` gains `walls: Vec<StaticPlane>` (via a new `with_wall`
  builder, mirroring `with_car`) — generic flat static-plane geometry every
  body (ball and cars alike) now collides with via the same
  body-vs-static-plane machinery the ground already uses
  (`resolve_ground_contact` is renamed `resolve_plane_contact` and called
  once per wall in addition to the ground, for both the ball and every
  car). On top of that physical substrate, `drive::apply_driven_forces`
  gains a wall jump: a fresh `jump_pressed` press while airborne and
  touching a wall (`wall_normal: Some(normal)`, computed by `PhysicsWorld`
  up front the same way `on_ground` is) fires an impulse combining a new
  `WALL_JUMP_HORIZONTAL_SPEED` (uncalibrated placeholder) outward along the
  wall's normal with `JUMP_SPEED` upward. Wall jump takes priority over the
  double jump on that press but is otherwise independent of it: it doesn't
  consume `double_jump_available`; merely touching a wall (whether or not
  jump is pressed) unconditionally restores it, the same "any surface
  contact refills your second jump" rule landing already uses — so a
  player can wall-jump and still have a double jump left afterward, and
  can wall-jump again off the same or a different wall with no
  once-per-airborne-period limit of its own. Deliberately excludes
  variable jump height and any modeled arena footprint beyond generic flat
  walls — see Non-goals. (The directional "dodge" a real wall jump can pair
  with was excluded at the time this requirement first shipped; it is now
  implemented as FR-017.)
- `RB-PHYSICS-001-FR-014` (dodge, implemented): the double jump's fresh
  press (see FR-012) now checks `ControllerInput.pitch`/`roll` at the
  moment it fires: if either exceeds a new `DODGE_DEADZONE`, it fires a
  directional dodge instead of the plain vertical double jump — a purely
  horizontal `DODGE_SPEED` impulse (along `forward_axis`, scaled by
  `pitch`, and/or `right_axis`, scaled by `roll`) plus an instantaneous
  `DODGE_ANGULAR_SPEED` spin added directly to `RigidBody.angular_velocity`
  about the perpendicular axis (`right_axis` for pitch, `forward_axis` for
  roll) — reusing air control's own pitch/roll axis and sign conventions,
  so a forward dodge looks like a fast version of a forward air-control
  pitch. Both axes can contribute at once (a diagonal dodge), simply summed
  rather than normalized — a documented simplification, since real Rocket
  League normalizes the stick direction so a diagonal dodge isn't faster
  than an axis-aligned one. A dodge has no vertical component (unlike the
  plain double jump); below `DODGE_DEADZONE` on both axes, the plain
  vertical double jump fires exactly as it did before this requirement.
  Either way the press still spends the shared `double_jump_available`
  resource — a dodge and a plain double jump aren't separate resources.
  Wall jump was untouched at the time this requirement shipped: it never
  checked `pitch`/`roll` at all, so touching a wall always got the fixed
  wall-jump push-off, never a dodge. `DODGE_SPEED` is now `pub`, alongside
  the newly-`pub` `WALL_JUMP_HORIZONTAL_SPEED`, so `world.rs`'s end-to-end
  tests can assert against — and distinguish between — both. (The wall jump
  itself gained its own dodge variant later, as FR-017.)
- `RB-PHYSICS-001-FR-015` (variable jump height, implemented): the ground
  jump (FR-010) gains a hold window — continuing to hold
  `ControllerInput.jump` after the fresh press that fires it adds a
  continuous `JUMP_HOLD_ACCELERATION` upward force, for up to
  `JUMP_HOLD_MAX_DURATION` seconds, on top of the press's own fixed
  `JUMP_SPEED` impulse. A new per-car `jump_hold_time_remaining: f32`
  (`PhysicsWorld`'s parallel `car_jump_hold_time_remaining: Vec<f32>`,
  starting `0.0`) is checked and decremented at the very top of
  `apply_driven_forces` — against whatever value the *previous* call left
  it at — before that same call's own `on_ground`/`jump_pressed` handling
  can re-arm it to `JUMP_HOLD_MAX_DURATION`, so a fresh ground-jump press's
  own step always fires only the plain `JUMP_SPEED` impulse; only
  continued holding into later calls earns the extra height. Releasing
  `jump` zeroes the remaining window immediately, stopping the extra
  acceleration right away even if time was left — matching real Rocket
  League's held-vs-tapped jump height difference. Scoped to the ground
  jump alone: the double jump, a dodge, and the wall jump are each still a
  single fixed instantaneous impulse, unaffected by how long jump is held,
  since firing any of them requires releasing jump first (a fresh press),
  which itself unconditionally zeroes the ground jump's hold window before
  that press's own branch ever fires. `JUMP_HOLD_MAX_DURATION` and
  `JUMP_HOLD_ACCELERATION` are both uncalibrated placeholders — this port
  has no public reference for real Rocket League's actual hold-window
  length or acceleration the way `JUMP_SPEED` does.
- `RB-PHYSICS-001-FR-016` (flip-cancel, implemented): a dodge's spin
  (FR-014) can be canceled early — a further fresh `ControllerInput.jump`
  press while airborne, not touching a wall, with `double_jump_available`
  already spent by that dodge, zeroes `RigidBody.angular_velocity` outright
  instead of leaving the flip to spin indefinitely. A new per-car
  `dodge_flip_active: bool` (`PhysicsWorld`'s parallel
  `car_dodge_flip_active: Vec<bool>`, starting `false`) tracks whether the
  most recent double-jump-or-dodge press was a dodge whose spin hasn't been
  canceled or superseded yet: the directional-dodge branch sets it `true`;
  the plain-double-jump branch explicitly sets it `false` rather than
  leaving it alone, so a stale `true` left over from an earlier,
  already-landed-from dodge can't leak into spuriously canceling a later,
  unrelated plain double jump's non-existent flip. Flip-cancel doesn't
  touch linear velocity (the dodge's own translation is unaffected) and
  doesn't consume or restore `double_jump_available` (already spent by the
  dodge that set the flag). Wall jump keeps its existing priority — checked
  first in the airborne branch, unchanged — so a fresh press while touching
  a wall always wall-jumps, never flip-cancels. This port has no timed
  flip animation to interrupt (a dodge is one instantaneous
  angular-velocity kick, not a sustained torque over a fixed duration —
  see FR-014), so "mid-flip" here means "any time before landing or a wall
  touch re-arms the double jump," a documented simplification of real
  Rocket League's actual flip-duration window. No new physics constants —
  this is a state-flag-gated zeroing action, not a magnitude to calibrate.
- `RB-PHYSICS-001-FR-017` (wall-jump dodge, implemented): the wall jump's
  own fresh press (see FR-013) now checks `ControllerInput.pitch`/`roll`
  the same way the ground double jump's press does (FR-014): at or above
  `DODGE_DEADZONE` on either axis, it fires a **wall-jump dodge** instead
  of the plain fixed push-off — the same outward-plus-upward impulse
  combined with a horizontal `DODGE_SPEED` component and
  `DODGE_ANGULAR_SPEED` spin (identical axis/sign conventions to the ground
  dodge), leaving `dodge_flip_active` set so its spin is flip-cancelable
  (FR-016) exactly like a ground dodge's. Below `DODGE_DEADZONE` on both
  axes, the plain wall jump fires exactly as it did before this
  requirement, still never touching `double_jump_available`. Unlike the
  plain wall jump, a wall-jump dodge *does* consume `double_jump_available`
  — the same resource a ground dodge spends — a deliberate simplification:
  since touching a wall unconditionally restores `double_jump_available`
  before this check ever runs (see FR-013), gating the dodge variant on it
  would be vacuous (it's always true here), so this port instead has the
  dodge variant spend it, the same way a ground dodge does, keeping the
  invariant "`dodge_flip_active` is only ever true while
  `double_jump_available` is false" intact without any changes to
  flip-cancel's own branch ordering or new landing/wall-touch-clearing
  logic. This port has no way to separately account for "a wall touch
  refilled the double jump, then the wall-jump dodge spent it" versus a
  genuinely independent wall-dash resource, and real Rocket League's
  precise accounting here isn't public to the precision this project would
  need to model that distinction. No new physics constants — reuses
  `DODGE_SPEED`/`DODGE_ANGULAR_SPEED`/`WALL_JUMP_HORIZONTAL_SPEED`/
  `JUMP_SPEED`, all already introduced by earlier requirements. Two
  pre-existing tests (`drive::wall_jump_fires_instead_of_a_dodge_when_touching_a_wall`, `world::wall_jump_still_fires_instead_of_a_dodge_when_touching_a_wall`) asserted the *old* "wall jump always ignores stick
  input" premise this requirement deliberately reverses; both were
  repurposed (not silently deleted) to assert the new wall-jump-dodge
  behavior instead, keeping their scenario (touching a wall with
  directional stick input) but updating the expected outcome.
- `RB-PHYSICS-001-FR-018` (landing auto-orientation assist, implemented):
  `drive::apply_driven_forces` gains a gentle continuous restoring torque,
  applied while airborne, that nudges the car's local up axis back toward
  world up — real Rocket League auto-corrects a car's orientation somewhat
  on approach to landing; this port has no ground-proximity raycast or
  distance query to replicate that trigger condition, so instead the assist
  applies continuously whenever airborne, gated on two conditions instead:
  no active `pitch`/`roll` air-control input this step (`pitch == 0.0 &&
  roll == 0.0`, so the assist never fights the player's own air control —
  it only fills in when the stick is neutral) and no fresh
  `ControllerInput.jump` press this step (so it never interacts, within the
  same `integrate_velocities` call, with a dodge's, wall-jump-dodge's,
  double-jump's, or flip-cancel's own same-step direct velocity/
  angular-velocity change). The correction itself is `up_axis(car).cross(
  &world_up) * LANDING_AUTO_UPRIGHT_TORQUE`: since both vectors are unit
  length, the cross product's magnitude is already proportional to the
  sine of the car's tilt off level, so a level car earns no correction and
  a heavily tilted one earns a proportionally stronger nudge, with no
  separate angle computation needed. `LANDING_AUTO_UPRIGHT_TORQUE` is a new
  uncalibrated placeholder, deliberately one full order of magnitude
  smaller than `AIR_CONTROL_TORQUE` so the assist reads as "gentle
  assistance," not "full control." Known, accepted, unaddressed limitation:
  a car resting exactly upside-down gives an exactly antiparallel
  `up_axis`/`world_up` pair, whose cross product is also zero — no
  correction is computed in that unlikely exact singularity. No new
  `PhysicsWorld` state — the assist is a pure function of the car's current
  orientation, input, and ground contact, all already in scope.
- `RB-PHYSICS-001-FR-019` (modeled arena footprint, implemented): a new
  `arena` module builds Rocket League's real standard-arena boundary
  entirely from FR-013's existing generic `StaticPlane`/`with_wall`
  machinery — no new collision code, since a ceiling and a corner-cut wall
  are each just another flat plane. `arena::standard_ground` is the flat
  floor at `z = 0` (identical to the `flat_ground()` test helper this crate
  has used since v0); `arena::standard_walls` returns 9 `StaticPlane`s: 2
  side walls (`x = ±SIDE_WALL_X`), 2 back walls (`y = ±BACK_WALL_Y`), a
  ceiling (`z = CEILING_Z`), and 4 diagonal corner walls (one per
  quadrant) that cut off the true rectangular corner where a side wall
  would otherwise meet a back wall at 90 degrees — giving the field its
  real octagonal footprint instead of a plain rectangle. `SIDE_WALL_X`
  (4096), `BACK_WALL_Y` (5120), and `CEILING_Z` (2048, corrected from an
  earlier `2044` by `RB-PHYSICS-001-FR-036`) are commonly-cited
  community-measured field dimensions (the same sourcing convention as
  `drive::MAX_CAR_SPEED`/`JUMP_SPEED`); the corner walls' inset distance
  (`CORNER_LENGTH`, equal along both axes, giving a 45-degree cut) is
  likewise now confirmed exact, not the uncalibrated placeholder this
  project once took it for (`RB-PHYSICS-001-FR-036`) — its flat corner-wall
  plane matches real extracted collision-mesh data precisely, even though
  the real arena's corners aren't a single flat plane all the way to the
  floor/ceiling seam (they curve, and blend into ramps this port doesn't
  model either). `PhysicsWorld::standard_arena`
  is a new convenience constructor — `PhysicsWorld::new(ball,
  arena::standard_ground())` followed by a `with_wall` call for each of
  `standard_walls()`'s 9 planes — offered alongside, not replacing,
  `PhysicsWorld::new`/`with_wall`'s existing ad-hoc-wall capability (a
  caller building a non-standard test scene, as most of this crate's own
  tests do, still uses those directly). Still not modeled at the time this
  requirement shipped: curved wall-to-floor/wall-to-ceiling transitions
  (now implemented for the ball at the 4 cardinal walls, see FR-020), goal
  cutouts in the back walls, and disambiguating or blending a car's
  simultaneous contact with two walls at a corner for wall-jump purposes
  (see Non-goals) — `resolve_plane_contact`'s own physical resolution of a
  car touching two walls at once already works correctly regardless (each
  wall is resolved independently every step), only the wall-jump push-off
  direction picker still isn't.
- `RB-PHYSICS-001-FR-020` (curved wall-to-floor/wall-to-ceiling transitions,
  implemented): a new `body::StaticQuarterPipe` shape — an immovable
  partial-cylinder fillet connecting two perpendicular flat planes, infinite
  along its own axis like `StaticPlane` — and `collision::contacts_vs_quarter_pipe`, a sphere-only narrow-phase test (a box/car
  always returns no contact — see FR-020's own Non-goals). The playable side
  is the *inside* of the fillet's concave face (the same geometry a
  skateboard quarter-pipe is named after and ridden on the inside of): a
  point is governed by a fillet at all only when its direction from
  `axis_point`, projected perpendicular to `axis_direction`, falls within
  the 90-degree sector from `sector_start` to `sector_end` (checked via
  `dot(dir, sector_start) >= 0 && dot(dir, sector_end) >= 0`, exact for a
  90-degree sector since the two vectors are perpendicular); within that
  sector, contact fires as the sphere's surface approaches or crosses the
  fillet's own radius *from the inside*, and the correction pushes the
  sphere back toward the axis — the opposite direction convention from
  `sphere_vs_plane`'s always-away-from-the-plane push. `StaticQuarterPipe::between_planes(plane_a, plane_b, radius, axis_direction)` derives a
  fillet's axis/sector automatically from the two flat planes it bridges
  (offsetting each plane inward by `radius` along its own normal, and
  negating each plane's normal for the sector vector pointing back to its
  own tangent point) — exact whenever `plane_a`/`plane_b`'s normals and
  `axis_direction` form an orthonormal basis, which only requires the two
  bridged planes to be mutually *perpendicular* (true for every arena wall's
  own floor/ceiling seam, cardinal or diagonal — see FR-021 — not for two
  walls meeting at a corner, which generally aren't perpendicular — see
  Non-goals). `PhysicsWorld` gains `curves: Vec<StaticQuarterPipe>` and
  a `with_curve` builder (mirroring `walls`/`with_wall`), resolved via a new
  `resolve_curve_contact` alongside `resolve_plane_contact` for the ball and
  every car (a no-op for cars at the time this requirement shipped, since
  the box arm of `contacts_vs_quarter_pipe` was always empty — now
  implemented, see FR-027). `solver::resolve_contacts`'s second parameter changed
  from `&StaticPlane` to plain `restitution: f32, friction: f32` — the only
  two fields it ever actually used — so this same solver path serves a
  `StaticQuarterPipe` fillet exactly as it already served a `StaticPlane`,
  with no new solver code needed. `arena::standard_curves` builds the 8
  fillets (floor-side and ceiling-side, for each of the 4 cardinal walls)
  the standard arena needs via `between_planes`, using a new uncalibrated
  placeholder `FILLET_RADIUS` (this port has no verified reference for the
  real transition radius either); `PhysicsWorld::standard_arena` now adds
  these 8 curves alongside its existing 9 walls. Still not modeled at the
  time this requirement shipped: a car actually being deflected by a
  fillet (now implemented, see FR-027), fillets at the 4 diagonal corner
  walls (now implemented, see FR-021), and goal cutouts (see Non-goals).
- `RB-PHYSICS-001-FR-021` (curved corner-wall-to-floor/wall-to-ceiling
  transitions, implemented): extends FR-020's fillet treatment to the 4
  diagonal corner walls `FR-019` introduced — `arena::standard_curves` now
  builds 16 `StaticQuarterPipe`s total (still one floor-side and one
  ceiling-side fillet per wall, now for all 9 walls) instead of 8.
  `StaticQuarterPipe::between_planes` itself needed no code changes: its
  only real correctness requirement is that the two bridged planes'
  normals, plus `axis_direction`, form an orthonormal basis, which only
  needs the two planes to be mutually *perpendicular* — true for a corner
  wall meeting the floor or ceiling regardless of the corner wall's own
  horizontal rotation (a vertical wall's normal always has zero Z component,
  and the floor/ceiling's is always purely Z), not something limited to
  axis-aligned cardinal walls the way FR-020's own doc comment had
  (incorrectly) claimed. The only new work is in `arena.rs`'s
  `standard_curves`: a cardinal wall's fillet axis direction was always
  hand-picked as a coordinate axis (`(0,1,0)` for a side wall, `(1,0,0)` for
  a back wall — each wall's own "along the wall" direction), but a corner
  wall's "along the wall" direction isn't a coordinate axis, so it's instead
  computed via a cross product, `floor.normal.cross(&wall.normal)` (and the
  ceiling equivalent) — already exactly unit length by construction (the two
  operands are always-perpendicular unit vectors, so `|a x b| = |a||b|
  sin(90 deg) = 1` exactly, up to floating-point precision), so no
  `.normalize()`/`.unwrap()` is needed or used (avoiding a
  `clippy::unwrap_used` violation in production code, which the workspace's
  lint config promotes to a hard CI error). A new `corner_wall_plane(sx,
  sy)` helper in `arena.rs` factors out the existing (unchanged)
  `standard_walls` corner-wall construction so `standard_curves` can reuse
  it, rather than duplicating the corner-wall plane math. `PhysicsWorld::standard_arena` picks up the extra 8 curves automatically, since it
  already loops over every curve `arena::standard_curves()` returns. Still
  not modeled at the time this requirement shipped: a car actually being
  deflected by any fillet (now implemented, see FR-027), a fillet at a
  corner wall's own *vertical* edges (now implemented, see FR-022), and
  goal cutouts (see Non-goals).
- `RB-PHYSICS-001-FR-022` (curved corner-wall vertical-edge fillets,
  implemented): rounds off the last sharp edges the standard arena's
  octagonal footprint has — the 8 vertical edges where each of the 4
  diagonal corner walls meets its neighboring side or back wall.
  `arena::standard_curves` now builds 24 `StaticQuarterPipe`s total (the 16
  floor/ceiling-seam fillets FR-020/FR-021 already built, plus 8
  vertical-edge fillets, one per corner-wall endpoint). Unlike every prior
  fillet in this port, the two planes a vertical-edge fillet bridges
  *aren't* perpendicular: a corner wall meets its neighboring side/back wall
  at 135 degrees (given `standard_walls`' 45-degree corner cut), not 90.
  This exposed a real gap in `StaticQuarterPipe::between_planes`, which
  previously only worked correctly for perpendicular planes (silently
  computing the wrong axis point otherwise, via a shortcut formula — adding
  the two scaled normals together — that only happens to equal the correct
  answer when the normals are orthogonal). `between_planes` is now fully
  general: it solves the axis point via the actual 2x2 linear system in the
  (possibly non-orthogonal) basis the two normals form, and its own sector
  angle comes out to exactly `arccos(dot(plane_a.normal, plane_b.normal))`
  — a right angle for perpendicular planes as before, or (for this
  requirement's own corner-wall geometry) a shallow 45 degrees, the
  supplement of the 135-degree dihedral angle the two flat walls actually
  meet at. `sphere_vs_quarter_pipe`'s sector-membership test is likewise
  generalized: the old two-dot-products check only worked because a
  90-degree sector's two edges are perpendicular; the new test uses signed
  cross products against `axis_direction` instead (`dir` is in-sector iff
  sweeping from `sector_start` toward it, and from it toward `sector_end`,
  both go the positive way around `axis_direction`), which is exact for any
  sector up to 180 degrees, the widest a sensible fillet-replacing-a-corner
  can ever be. Since a general (non-orthogonal) sector's own containment
  test depends on `axis_direction`'s sign/handedness — unlike the old
  perpendicular-only test, which never used `axis_direction` at all —
  `between_planes` now also self-corrects a "backwards" `axis_direction`
  internally (flipping it if `cross(sector_start, sector_end)` doesn't
  already point the same way), so a caller can pass either of the two
  opposite directions along the shared edge line without needing to reason
  about which one is "correct." New `arena::corner_wall_plane` reuse aside,
  the vertical-edge fillets' own `axis_direction` is simply `(0, 0, 1)` (the
  edge itself is vertical) — no cross product needed here, unlike the
  corner-wall floor/ceiling-seam case FR-021 introduced. `FILLET_RADIUS` is
  reused as-is once again, rather than a separate, smaller radius for these
  visibly shallower edges. Still not modeled: a car actually being
  deflected by any fillet (now implemented, see FR-027), and goal cutouts
  (see Non-goals) — the
  compound corner where a vertical-edge fillet meets a floor- or
  ceiling-seam fillet, near a wall's own endpoint, is addressed next (see
  FR-023).
- `RB-PHYSICS-001-FR-023` (compound-corner fillets, implemented): rounds off
  the 16 remaining sharp vertices in the standard arena's vertical
  boundary — the compound corners where a corner wall's own vertical-edge
  fillet (FR-022) meets a floor- or ceiling-seam fillet (FR-020/FR-021),
  near that corner wall's own top or bottom endpoint. Unlike every prior
  fillet in this port, which each bridge exactly two flat planes with a
  cylindrical (`StaticQuarterPipe`) shape, a compound corner is where
  *three* planes meet (floor or ceiling, the neighboring side or back wall,
  and the corner wall itself) — a cylinder can't blend three planes at
  once, so this requirement introduces a genuinely different static shape,
  `body::StaticCornerFillet`: an immovable sphere, riding the concave
  inside of the sharp vertex exactly the way each `StaticQuarterPipe`
  already rides the concave inside of its own sharp edge. Its constructor,
  `StaticCornerFillet::between_three_planes(plane_a, plane_b, plane_d,
  radius)`, exploits the same "radius-in from every bridged plane"
  invariant `StaticQuarterPipe::between_planes` already established: since
  the fillet's center must sit exactly `radius` in from all three planes,
  it's also exactly `radius` in from each *pair* of them — meaning it
  already lies on all three of that vertex's own pairwise
  `StaticQuarterPipe::between_planes` axis lines simultaneously, so the
  center is nothing more than those three lines' common intersection
  point, solved directly via the classic three-plane-intersection
  cross-product form of Cramer's rule (`center = (cross(n_b, n_d) *
  target_a + cross(n_d, n_a) * target_b + cross(n_a, n_b) * target_d) /
  (n_a . (n_b x n_d))`, where each `target` is that plane's own offset plus
  `radius`) rather than solved from scratch. Containment
  (`sphere_vs_corner_fillet` in `collision.rs`) generalizes a
  `StaticQuarterPipe`'s 2-sided sector test to a "spherical triangle": a
  direction from the center is inside the fillet iff its dot product with
  each of 3 `bounds` vectors is non-negative. Each bound is the raw
  (deliberately non-normalized, since only its sign is used) cross product
  of one pair of the three normals, sign-corrected — via
  `signed_pair_axis`, checking the third (non-pair) plane's own normal
  against it — to always point toward the sharp corner this fillet
  replaces, which is provably correct because that dot product is exactly
  the derivative of the third plane's own signed distance along a
  candidate direction. This sign-correction, like `between_planes`'s own
  `axis_direction` self-correction (FR-022), needs no
  `.normalize()`/`.unwrap()` anywhere in production code. `arena::standard_corner_fillets` builds all 16 (4 per corner wall — floor+side,
  floor+back, ceiling+side, ceiling+back — times the 4 corner walls) by
  calling `between_three_planes` directly on the same three flat planes
  `standard_walls` already builds, reusing `FILLET_RADIUS` once again
  rather than introducing a fourth radius constant (later switched to the
  distinctly larger `CORNER_ARCH_RADIUS`, since FR-025, once a corner
  wall's own floor/ceiling-seam arches did — see FR-025's own entry).
  `PhysicsWorld` gains a
  parallel `corner_fillets: Vec<StaticCornerFillet>` field and
  `with_corner_fillet` builder, resolved for the ball and every car exactly
  like `curves` (a no-op for a car at the time this requirement shipped,
  since `contacts_vs_corner_fillet`
  returned no contact for a box — the same documented deferred case as every
  other fillet here; now implemented, see FR-027); `PhysicsWorld::standard_arena` wires in all 16 via
  `arena::standard_corner_fillets` automatically. Still not modeled: a car
  actually being deflected by any fillet (now implemented, see FR-027), and
  goal cutouts (addressed next, see FR-024).
- `RB-PHYSICS-001-FR-024` (goal cutouts, implemented): opens an actual
  goal-mouth window in each back wall — until now every back wall was a
  single solid, flat `StaticPlane` spanning the full width, with no
  opening at all. A plain `StaticPlane` has no notion of a bounded hole, so
  this requirement introduces a new static shape, `body::StaticGoalWall`:
  a flat plane (`plane`) plus a rectangular window defined in the plane's
  own local 2D coordinate system (`window_center`, unit `u_axis`/`v_axis`,
  `half_width`/`half_height`) — the same "derive an axis/window in the
  plane's own local frame rather than assuming a world axis" discipline
  `StaticQuarterPipe::between_planes`'s `axis_direction` generalization
  (FR-022) established, even though every arena wall this port builds
  today happens to be axis-aligned. `contains_in_window` tests a point by
  its projection onto `u_axis`/`v_axis` alone — the point's own depth from
  the plane along `plane.normal` doesn't matter, since both axes are
  perpendicular to it by construction, so the test is exactly as correct
  approaching the plane as sitting right on it. Containment
  (`collision::sphere_vs_goal_wall`) suppresses contact entirely for a
  sphere (the ball) whose center falls inside the window, letting it pass
  straight through; `contacts_vs_goal_wall`'s box (car) path deliberately
  ignores the window altogether, falling straight through to the ordinary
  `contacts_vs_plane` against the wrapped `plane` — ball-only deflection,
  the same documented deferred case as every fillet here, and a
  zero-regression choice for a car, which now sees literally the same
  contact-generation call it always did against a back wall. `arena::standard_walls` accordingly drops the 2 back-wall `StaticPlane`s it used
  to return (now 7 planes instead of 9); the new `arena::standard_goal_walls` returns them instead as 2 `StaticGoalWall`s, each
  wrapping the same `back_wall_plane` construction as before, windowed at
  `GOAL_HALF_WIDTH`/`GOAL_HEIGHT` (new commonly-cited, uncalibrated-against-
  real-field-mesh-data constants, same sourcing status as `SIDE_WALL_X`)
  centered on the wall at half the goal's own height. `PhysicsWorld` gains
  a parallel `goal_walls: Vec<StaticGoalWall>` field and `with_goal_wall`
  builder, resolved for the ball *and* every car (unlike `curves`/
  `corner_fillets`'s ball-only resolution) — safe precisely because the box
  path is a no-op change from the prior plain-`StaticPlane` behavior.
  New `arena::standard_goal_cutout_fillets` rounds each window's 3 edges
  (two vertical posts, one horizontal crossbar, times 2 goals — 6
  `StaticQuarterPipe`s total, added to the same `curves` list
  `standard_curves`'s 24 already populate), each derived via the existing
  `StaticQuarterPipe::between_planes` from the real back-wall plane and a
  second, purely-geometric plane (`goal_post_plane`/`goal_crossbar_plane`)
  representing the post's or crossbar's own inward-/downward-facing
  surface — positioned at exactly the window's own edge, so the fillet's
  tangent point lands exactly on the window boundary with no gap or
  overlap, the same "sits radius-in from the real wall" property every
  other fillet-to-window/wall pairing in this crate already has. Unlike
  `standard_walls`' corner-wall planes or `standard_curves`' bridged walls,
  `goal_post_plane`/`goal_crossbar_plane` are never themselves added as
  real collision walls — an infinite plane perpendicular to X (or capping
  Z) would incorrectly wall off the *entire* rest of the field at that
  coordinate, unlike a diagonal corner wall's own orientation, which stays
  non-binding everywhere except right at the true corner. At the time this
  requirement shipped, the two compound corners per goal where a post's own
  fillet meets the crossbar's were left as independent, additive fillets,
  not blended into a single smooth vertex — the same "no blended 3D corner"
  approach the arena's other edge fillets used before FR-023 introduced one
  for the corner walls specifically; a dedicated `StaticCornerFillet` was
  later added for these goal corners too, see FR-026. Still not modeled: a
  car actually being deflected by any fillet (now implemented, see FR-027)
  or driving into a goal (now implemented, see FR-028), and a
  modeled goal interior/net beyond the cutout itself (the goal's own two
  compound top corners are now modeled, see FR-026).
- `RB-PHYSICS-001-FR-025` (corner-wall floor/ceiling arch radius,
  implemented): gives a diagonal corner wall's own floor-seam and
  ceiling-seam fillets — 8 of `standard_curves`' 24 entries, the ones
  bridging one of the 4 corner walls to the floor or ceiling — a
  distinctly larger, dedicated radius instead of reusing the cardinal
  walls' own `arena::FILLET_RADIUS`, matching real Rocket League's
  corner-boost area reading as a noticeably bigger, more swept curve than
  a cardinal wall's small rounding, not just a scaled-down copy of the
  same shape. New constant `arena::CORNER_ARCH_RADIUS` (750.0, versus
  `FILLET_RADIUS`'s 292.0) — documented, like every other arena dimension
  in this module, as this project's own uncalibrated placeholder, not
  measured against real field mesh data — governs those 8 arches; a
  compile-time `const _: () = assert!(CORNER_ARCH_RADIUS >
  FILLET_RADIUS);` right after the constant's own definition enforces the
  "distinctly larger" relationship so the two constants can't quietly
  converge or invert under a future edit. Because
  `StaticCornerFillet::between_three_planes` derives its center and
  containment `bounds` from a single shared `radius` argument blended
  across all three planes it bridges — there is no way to ask it for a
  center that sits one radius in from two planes and a different radius in
  from the third, and doing so would also break the "meets its adjoining
  edge fillets exactly where their axes cross" no-gap property FR-023
  established — all 16 of `standard_corner_fillets`'s compound-corner
  fillets switch to `CORNER_ARCH_RADIUS` too, since every one of them
  touches one of the 8 now-larger arches. Unaffected, and still using
  `FILLET_RADIUS` exactly as before: the 8 cardinal-wall (side/back)
  floor/ceiling-seam fillets, the 8 vertical corner-edge fillets
  (`RB-PHYSICS-001-FR-022`, where a corner wall meets its neighboring
  side/back wall), and the 6 goal-cutout edge fillets
  (`RB-PHYSICS-001-FR-024`) — these remain independent, additive contact
  sources next to the bigger arches, not blended with them, the same
  "no blended 3D corner" convention this port has used since before
  FR-023. Still not modeled: a car actually being deflected by any fillet
  (now implemented, see FR-027),
  and everything else `FR-024`'s own Non-goals already cover.
- `RB-PHYSICS-001-FR-026` (goal post-crossbar corner fillets, implemented):
  rounds off the two remaining sharp compound corners per goal — where a
  post's own vertical-edge fillet (`arena::standard_goal_cutout_fillets`,
  FR-024) meets the crossbar's own horizontal-edge fillet — an explicitly
  documented gap FR-024's own doc comment left open ("The two compound
  corners per goal where a post's fillet meets the crossbar's are
  deliberately not blended into a single smooth vertex"). New
  `arena::standard_goal_corner_fillets` returns 4 `StaticCornerFillet`s —
  one per goal post per goal (2 posts x 2 goals) — mirroring FR-023's own
  approach for the arena's 16 compound corners exactly: each is built by
  calling `StaticCornerFillet::between_three_planes` directly on the three
  real flat planes that meet at that vertex (the back wall, that post's own
  plane, and the crossbar), rather than derived from the two
  `StaticQuarterPipe` edge fillets already built at that vertex — since a
  corner fillet's center is already exactly those two edge fillets' own
  common axis intersection, the same property `between_three_planes`'s own
  doc comment (FR-023) already explains. No new shape or collision code was
  needed: `StaticCornerFillet` and `sphere_vs_corner_fillet` (FR-023) are
  already fully generic to any three non-parallel planes. Unlike FR-025 (a
  new, distinctly larger `CORNER_ARCH_RADIUS` was needed there because the
  arena's own cardinal-vs-corner-wall arches use different radii), this
  requirement reuses `FILLET_RADIUS` unchanged for all 4 new fillets, since
  both edge fillets meeting at a goal's post-crossbar corner already share
  `FILLET_RADIUS` — no mismatched-radius concern exists here.
  `PhysicsWorld::standard_arena` wires these 4 in via the same generic
  `with_corner_fillet` builder `standard_corner_fillets`'s 16 already use,
  so `PhysicsWorld.corner_fillets` now holds 20 total, not 16 — the
  pre-existing test `standard_arena_has_sixteen_compound_corner_fillets`
  was renamed `standard_arena_has_twenty_compound_corner_fillets` and its
  assertion updated to match. Explicitly still out of scope: the goal's
  other two corners, where a post meets the floor — the window's own bottom
  edge sits exactly at floor level (`z = 0`), so a post's own fillet there
  simply ends flush with the ground, with no sharp, unrounded vertex left
  to round off, unlike the top post-crossbar corner. Still not modeled at
  the time this requirement shipped: a car actually being deflected by any
  fillet (now implemented, see FR-027) or driving into a goal (now
  implemented, see FR-028), and a
  modeled goal interior/net beyond the cutout itself (see Non-goals).
- `RB-PHYSICS-001-FR-027` (car deflection by curved fillets, implemented):
  a car (box) drove straight through every curved fillet in this port —
  `StaticQuarterPipe` and `StaticCornerFillet` alike — until now, since
  `collision::contacts_vs_quarter_pipe`/`contacts_vs_corner_fillet` always
  returned an empty manifold for a `Shape::Box`, a documented deferred case
  repeated across FR-020 through FR-026's own Non-goals. This requirement
  closes that gap by reducing a box-vs-curve test to the same "test every
  corner" technique `box_vs_plane` already uses for a flat plane (already
  in this codebase, generating up to 4 contacts for a box resting on a
  plane): new `collision::box_vs_quarter_pipe(position, orientation,
  half_extents, pipe)` checks each of the box's 8 corners as a
  zero-radius sphere via the existing `sphere_vs_quarter_pipe(world_corner,
  0.0, pipe)` call, collecting every corner that reports a contact into the
  manifold; new `collision::box_vs_corner_fillet` does the identical thing
  against `sphere_vs_corner_fillet` for the sphere-shaped compound-corner
  fillets. `contacts_vs_quarter_pipe`/`contacts_vs_corner_fillet` now
  dispatch a `Shape::Box` to these new functions instead of returning
  `Vec::new()`. Each surviving contact's `point` field is overwritten to
  the corner's own world position rather than the fillet-surface point
  `sphere_vs_quarter_pipe`/`sphere_vs_corner_fillet` itself computes — the
  same rel_pos/torque-accuracy reason `box_vs_plane`'s own doc comment
  already documents: a tilted box's corner isn't generally "below" the body
  center along the surface normal, so the solver needs the true
  contact-to-center offset to compute correct torque, not a point that
  merely lies on the curved surface along the contact normal. This was
  originally documented, in both new functions' own doc comments, as an
  approximation, not a full convex-vs-curved-surface narrow phase — the
  concern being that a face of the box resting flush against a shallow
  curve (a radius large relative to the box) could have every one of its
  own corners still just clear of the fillet while the face's middle
  already overlaps it, under-detecting that case.
  `RB-PHYSICS-001-FR-032` subsequently investigated this concern directly
  (building a genuine GJK-based replacement specifically to fix it) and
  found it doesn't actually apply here: a quarter-pipe/corner-fillet's
  contact test is a *containment* question (is the box's farthest point
  from the axis/center at or beyond radius), and distance-from-a-line/point
  is a convex function whose maximum over a convex polytope (the box) is
  always attained at a corner — so per-corner testing is mathematically
  exact for this question, not an approximation. See
  `RB-PHYSICS-001-FR-032`'s own entry and `box_vs_quarter_pipe`'s current
  doc comment for the full argument and its empirical confirmation. No new
  shape or fundamentally new collision primitive was needed — this is a
  generalization of existing dispatch, not new physics/math machinery.
  `PhysicsWorld::step` already called `resolve_curve_contact`/
  `resolve_corner_fillet_contact` for every car in the scene — that call
  was wired in from FR-023 onward, just always a no-op for a car until now,
  since `contacts_vs_quarter_pipe`/`contacts_vs_corner_fillet` always
  returned empty for a box — so `world.rs`'s step loop itself needed no
  changes, only doc-comment updates reflecting the new real behavior (the
  "no-op for a box" language in `resolve_curve_contact`/
  `resolve_corner_fillet_contact`'s own doc comments, the `curves`/
  `corner_fillets` field doc comments, the `with_curve`/`with_corner_fillet`
  builder doc comments, and `arena.rs`'s own module-level "Still not
  modeled" list). Explicitly unaffected by this requirement:
  `StaticGoalWall`/`contacts_vs_goal_wall` — a car still sees the exact same
  solid, full-width back wall it always has, deliberately ignoring the
  goal-mouth window, since a goal wall isn't a curved fillet at all and this
  generalization only touches `StaticQuarterPipe`/`StaticCornerFillet`
  dispatch; a car actually driving into a goal is now implemented, see
  `RB-PHYSICS-001-FR-028`.
- `RB-PHYSICS-001-FR-028` (car actually driving into a goal, implemented):
  a car (box) drove straight into the full, solid back wall even at the
  goal mouth until now — `collision::contacts_vs_goal_wall` dispatched a
  `Shape::Box` straight through to plain `contacts_vs_plane` against the
  wrapped `StaticPlane`, completely ignoring the goal-mouth window
  `StaticGoalWall` carries, even though a sphere (the ball) already
  passed through the window via `sphere_vs_goal_wall` — a documented
  Non-goal repeated across FR-024 through FR-027's own "Still not
  modeled" lists. This requirement closes that gap with a new
  `collision::box_vs_goal_wall(position, orientation, half_extents, wall)
  -> Vec<Contact>`: it iterates the box's 8 corners exactly like
  `box_vs_plane` does, but for each corner first checks
  `wall.contains_in_window(&world_corner)` — if the corner's own
  projection onto the plane's `u_axis`/`v_axis` falls inside the window,
  that corner contributes no contact at all (skipped via `continue`),
  regardless of how deep it might be penetrating along the plane's
  normal (this exactly mirrors `sphere_vs_goal_wall`'s existing
  convention of ignoring distance-along-normal for the window test —
  `contains_in_window` was already written that way, no changes needed
  to `body::StaticGoalWall` itself). A corner outside the window falls
  through to an ordinary `box_vs_plane`-style corner test (signed
  distance against the wrapped plane, contact if within
  `CONTACT_PROCESSING_THRESHOLD`). This is the exact same "test every
  corner" approximation technique `RB-PHYSICS-001-FR-027` established
  for curved fillets (`box_vs_quarter_pipe`/`box_vs_corner_fillet`),
  applied here to a flat windowed plane instead of a curved one. One
  real behavioral consequence worth documenting explicitly: because
  each corner is tested independently, a car that's only partially
  lined up with the window (e.g. straddling one of its edges) gets a
  *partial* block — the corners still outside the window register
  contacts and stop the car there, while the corners inside the window
  register none — rather than an all-or-nothing result the way a
  single-point sphere test necessarily produces; this is a real
  emergent behavior of the technique, not a separate feature.
  `contacts_vs_goal_wall`'s dispatch changed from `Shape::Box { .. } =>
  contacts_vs_plane(body, &wall.plane)` to `Shape::Box { half_extents } =>
  box_vs_goal_wall(body.position, body.orientation, half_extents, wall)`.
  No `world.rs` step-loop changes were needed — exactly like FR-027's own
  discovery: `PhysicsWorld::step`'s `resolve_goal_wall_contact` was
  *already* being called for every car in the scene (it always had been,
  since a car needed the wall's plain-plane collision even before this
  fix), so this is a pure dispatch-function change; only doc-comment
  updates were needed — in `world.rs` (`goal_walls`' field doc comment,
  `with_goal_wall`'s doc comment, and `resolve_goal_wall_contact`'s doc
  comment all now say a car passes through too, instead of describing it
  as unwindowed or falling straight through to an ordinary plane
  contact), `body.rs`'s `StaticGoalWall` doc comment, `arena.rs`'s
  module-level and `standard_goal_walls`'s own doc comments, and
  `lib.rs`'s crate-level module doc comment. Explicitly still not
  implemented by this requirement: a modeled goal interior/net — the
  goal still opens onto open, unbounded space beyond the back wall for
  both the ball and now a car (see Non-goals); the goal-cutout edge
  fillets and goal-corner fillets (FR-024/FR-026) already deflect a car
  via FR-027's own generalization, unaffected by this specific change —
  this requirement is purely about the flat windowed wall itself, not
  the fillets around its rim.
- `RB-PHYSICS-001-FR-029` (modeled goal interior, implemented): until now
  the goal-mouth window (`StaticGoalWall`, since FR-024) opened onto
  completely open, unbounded space — a ball or car passing through (the
  ball since FR-024, a car since FR-028) sailed forever, with nothing
  behind the window to stop it — a documented Non-goal repeated across
  FR-024 through FR-028's own "Still not modeled" lists. This requirement
  closes that gap by modeling a bounded interior volume behind each
  goal-mouth window: explicitly a solid bounding box, **not** a
  springy/catching net mesh — no cloth/soft-body simulation was added.
  This is a deliberate, honest scoping decision, not an oversight: a
  genuine net mesh was a real, separate, not-yet-implemented Non-goal at
  the time (see Non-goals) — since resolved for the ball specifically by
  `RB-PHYSICS-001-FR-033`'s `net::NetMesh`, sitting in front of this
  requirement's own solid back-of-net plane (unaffected, still a car's real
  backstop). Three pieces, all in `crates/rb_physics_bullet`: (1) a
  new constant `arena::GOAL_DEPTH: f32 = 880.0` — an uncalibrated
  placeholder, since this port has no verified reference for Rocket
  League's real net depth at all (unlike `GOAL_HALF_WIDTH`/`GOAL_HEIGHT`),
  chosen only to be a visibly real interior volume comparable in scale to
  the goal mouth's own dimensions; (2) a new shape type,
  `body::StaticBoundedWall` — a flat `StaticPlane` that only collides
  *within* a rectangular bound in the plane's own local 2D frame
  (`bound_center`/`u_axis`/`v_axis`/`half_u`/`half_v`, plus a
  `contains_in_bound` method) — the **opposite** gate convention from
  `StaticGoalWall`'s window (which collides everywhere *except* inside a
  rectangle). This is needed because the goal's own side walls and roof
  can't be plain unbounded `StaticPlane`s: an infinite plane at, say,
  `x = GOAL_HALF_WIDTH` would incorrectly wall off the *entire* main field
  at that x coordinate — the exact same problem `arena::goal_post_plane`'s
  own pre-existing doc comment already documented for a different
  purely-geometric plane used only to derive fillets. New dispatch
  functions in `collision.rs`: `sphere_vs_bounded_wall`/
  `box_vs_bounded_wall`/`contacts_vs_bounded_wall` — `box_vs_bounded_wall`
  uses the same "test every corner" technique established by
  FR-027/FR-028, but a corner *outside* the bound is skipped (the opposite
  of `box_vs_goal_wall`'s per-corner window test, where a corner *inside*
  the window is skipped); and (3) new arena geometry functions in
  `arena.rs`: `goal_back_wall_plane(sign)`/`standard_goal_back_walls()` —
  2 plain, unbounded `StaticPlane`s (one per goal), positioned
  `GOAL_DEPTH` behind the real back wall, added to `PhysicsWorld.walls` via
  the existing `with_wall` builder (no new field needed) — deliberately
  plain planes, not `StaticBoundedWall`s, since nothing can ever reach this
  plane except by first passing through the goal-mouth window (the real
  back wall is solid everywhere else), so an unbounded plane here is
  exact, not an approximation; `goal_side_wall(back_sign,
  post_sign)`/`standard_goal_side_walls()` — 4 `StaticBoundedWall`s total
  (2 per goal), each reusing `goal_post_plane(post_sign)` completely
  unchanged as its own flat plane (the post's own inward-facing surface
  already sits exactly where the goal box's own side wall needs to be),
  bounded to the goal's own depth (`y` from the real back wall out to
  `GOAL_DEPTH` behind it) and height (`z` from the floor up to
  `GOAL_HEIGHT`) range; and `goal_roof(sign)`/`standard_goal_roofs()` — 2
  `StaticBoundedWall`s total (1 per goal), each reusing
  `goal_crossbar_plane()` completely unchanged, bounded to the goal's own
  width (`x` within `GOAL_HALF_WIDTH` either way) and depth range.
  `PhysicsWorld` (in `world.rs`) gains a new field
  `bounded_walls: Vec<StaticBoundedWall>` and a `with_bounded_wall` builder
  (mirroring `with_goal_wall`), plus a new `resolve_bounded_wall_contact`
  (mirroring `resolve_goal_wall_contact`), resolved for the ball and every
  car in `PhysicsWorld::step`, exactly like `goal_walls`.
  `PhysicsWorld::standard_arena` wires in `standard_goal_back_walls()` via
  `with_wall`, and `standard_goal_side_walls()`/`standard_goal_roofs()` via
  `with_bounded_wall`. No changes to the actual step-loop resolution
  pattern were needed beyond adding the new loop — unlike FR-027/FR-028,
  this isn't a "silent no-op that later got activated" story; it's
  straightforwardly new geometry wired in from the start.
  `PhysicsWorld.walls` grew from 7 to 9 real entries once
  `standard_arena` is built (the 2 new back-of-net planes), so the
  pre-existing `world.rs` test
  `standard_arena_has_seven_walls_and_the_standard_ground`
  was renamed `standard_arena_has_nine_walls_and_the_standard_ground` and
  its assertion updated — a test-count correction, not new capability.
  New tests (242 total, net +21 over FR-028's 221): 4 new in `body.rs` for
  `StaticBoundedWall::contains_in_bound`
  (`contains_in_bound_is_true_for_the_bounds_own_center`,
  `contains_in_bound_is_true_just_inside_each_edge`,
  `contains_in_bound_is_false_just_outside_each_edge`,
  `contains_in_bound_ignores_distance_from_the_plane_itself`) — mirroring
  the pre-existing `StaticGoalWall::contains_in_window` tests exactly, just
  with the boolean gate meaning inverted; 5 new in `collision.rs`
  (`sphere_inside_the_bound_behaves_like_an_ordinary_plane`,
  `sphere_outside_the_bound_has_no_contact`,
  `box_squarely_inside_the_bound_behaves_like_an_ordinary_plane`,
  `box_straddling_the_bounds_edge_only_collides_on_the_corners_still_inside_it`,
  `box_entirely_outside_the_bound_has_no_contact`) against a synthetic
  fixture, mirroring the `StaticGoalWall` collision tests with the gate
  inverted; 8 new in `arena.rs` proving the geometry functions place
  things correctly (`standard_goal_back_walls_has_two_walls`,
  `every_goal_back_wall_sits_goal_depth_behind_the_real_back_wall`,
  `standard_goal_side_walls_has_four_walls`,
  `every_goal_side_walls_plane_matches_some_goal_post_plane`,
  `every_goal_side_walls_bound_covers_the_real_goal_depth_and_height`,
  `standard_goal_roofs_has_two_roofs`,
  `every_goal_roofs_plane_is_the_goal_crossbar_plane`,
  `every_goal_roofs_bound_covers_the_real_goal_width`) — the same "prove
  the real geometry, not an arbitrary point" discipline this crate's other
  arena tests already use; and 4 new in `world.rs` (plus the renamed
  wall-count test above) — 1 wiring-count test
  (`standard_arena_has_six_bounded_walls`) and 3 new live end-to-end
  `PhysicsWorld` proofs
  (`a_ball_shot_into_the_goal_is_stopped_by_the_goal_back_wall`,
  `a_ball_shot_sideways_inside_the_goal_is_stopped_by_a_goal_side_wall`,
  `a_ball_shot_upward_inside_the_goal_is_stopped_by_the_goal_roof`).
  These 3 end-to-end tests are **deliberately** isolated to a minimal
  `PhysicsWorld` built from just the specific new wall(s) under test (via
  `PhysicsWorld::new` plus `with_wall`/`with_bounded_wall`, **not**
  `PhysicsWorld::standard_arena`), rather than the full standard arena
  every other end-to-end goal test in this file uses — see Verification
  plan for the two real test-design findings (a sector-membership
  isolation issue, and a wall-restitution-zeroing fix) this discovery led
  to. Explicitly still not implemented by this requirement: a genuine net
  *mesh* — no cloth/soft-body simulation, no visual net sag, no "ball
  tangles in netting" behavior; this is a solid bounding volume standing
  in for the net's functional role (stopping the ball/car), nothing more
  (see Non-goals).
- `RB-PHYSICS-001-FR-030` (combined multi-body solve, implemented): until
  now, `PhysicsWorld::step` resolved every ball-vs-car and car-vs-car
  contact manifold with its own independent call to
  `solver::resolve_contacts_between` — each call ran its own full
  `SOLVER_ITERATIONS` (10) Gauss-Seidel pass and applied the resulting
  velocity change to both bodies before the next pair's setup even read a
  body's velocity, so a body touching two others in the same step (e.g. a
  car pinned between the ball and another car) never had both contacts
  reasoned about together — the second-resolved pair's setup used the
  first pair's already-finished result as if it were the honest starting
  velocity, and could end up discarding almost all of the first contact's
  effect (see Verification plan for the measured magnitude). This
  requirement closes that gap with a new `solver::resolve_dynamic_manifolds(bodies: &mut [RigidBody], manifolds: &[(usize, usize,
  Vec<Contact>)], dt: f32)`: it sets up rows for every manifold once (the
  same per-contact `setup_two_body_rows` as before), gives every body
  *index* that appears in at least one manifold its own `DeltaVelocity`
  accumulator, and runs `SOLVER_ITERATIONS` iterations where *every*
  iteration processes *every* manifold once, each manifold's rows reading
  and updating the shared accumulators for its own two body indices —
  only after all iterations finish does it apply each body's own
  accumulated delta to its actual velocity, once. This is the real fix: a
  shared, interleaved iteration budget across every dynamic-vs-dynamic
  manifold touching in the scene that step, not a sequence of
  fully-independent pairwise solves. A new helper,
  `solver::delta_pair_mut(deltas: &mut [DeltaVelocity], a: usize, b:
  usize) -> (&mut DeltaVelocity, &mut DeltaVelocity)`, is a
  `split_at_mut`-based disjoint-borrow helper for arbitrary index pairs,
  generalizing the `Vec::split_at_mut` trick `PhysicsWorld::step`'s
  car-vs-car loop already used for the special case `b == a + 1`, so
  multiple manifolds sharing a body index can also share that body's
  single accumulator. The old `TwoBodyDelta` struct is removed:
  `resolve_two_body_row` now takes two separate `&mut DeltaVelocity`
  parameters (`delta_a`, `delta_b`) instead of one combined struct, which
  is what makes sharing an accumulator across manifolds possible.
  `resolve_contacts` (one dynamic body vs. static geometry) and
  `resolve_contacts_between` (still present, used directly by
  two-body-only callers and tests) are unchanged in their public
  behavior — `resolve_contacts_between`'s internals were adjusted to use
  two separate `DeltaVelocity`s instead of `TwoBodyDelta`, a pure refactor
  matching the new shared-solver internals, not a behavior change. In
  `world.rs`, `PhysicsWorld::step` no longer calls the two independent
  loops (`for car in &mut self.cars { Self::resolve_dynamic_contact(&mut
  self.ball, car, dt); }` then a nested car-vs-car loop, both through a
  now-deleted private `resolve_dynamic_contact` helper); instead it builds
  a `Vec<RigidBody>` (index 0 the ball, index `i+1` `self.cars[i]`),
  collects every non-empty ball-vs-car and car-vs-car
  `collision::contacts_between` result into a `Vec<(usize, usize,
  Vec<Contact>)>`, calls `solver::resolve_dynamic_manifolds` on the whole
  set once, then copies the resolved velocities back into `self.ball` and
  `self.cars`.
  - **Non-goals (this requirement).** Scoped strictly to dynamic-vs-dynamic
    contacts. Static contacts — ground, arena walls, curves, corner
    fillets, goal walls, and bounded walls — are deliberately *not* part
    of this combined solve: each body's contact with static geometry only
    depends on that one body, so resolving it independently loses no
    cross-body information, and `resolve_contacts` is untouched. Split
    impulse, warm-starting/sleeping, and the average-not-max
    restitution/friction combine mode remain exactly as documented before
    this requirement (see this spec's own Non-goals and Open questions) —
    this requirement shares the iteration *budget* across manifolds, it
    doesn't change what each contact row itself solves for.
  - **Acceptance criteria.**
    - `solver::resolve_dynamic_manifolds` runs `SOLVER_ITERATIONS`
      iterations, each iteration visiting every manifold in the input
      slice exactly once, before any body's velocity is updated from its
      accumulated delta.
    - Two (or more) manifolds that share a body index read and write that
      body's *same* `DeltaVelocity` accumulator across iterations — never
      a separate, independent one per manifold.
    - `resolve_contacts` and `resolve_contacts_between`'s existing
      public behavior (a single manifold solved against a static body, or
      between exactly two dynamic bodies in isolation) is unchanged;
      every pre-existing test exercising either function still passes
      with no assertion changes.
    - `PhysicsWorld::step` produces exactly one `solver::resolve_dynamic_manifolds` call per step covering every ball-vs-car and car-vs-car
      manifold that step, not one call per pair.
  - **Verification plan.** A dedicated solver-level test,
    `solver::tests::resolve_dynamic_manifolds_keeps_more_of_every_bodys_contact_than_resolving_pairs_independently`, and a world-level
    end-to-end test,
    `world::tests::a_ball_pinched_between_two_closing_cars_is_resolved_by_a_shared_multi_body_solve`, both build a left-right symmetric
    "pinch": a ball (mass 1) exactly touching two identical cars (mass
    180 each) closing in from opposite sides at equal (100 units/s)
    speed, restitution zero throughout. The true simultaneous-solve
    answer for this exact setup is all three bodies ending near zero
    velocity (total momentum is exactly zero, and every body is mutually
    constrained to the others). Measured results: resolving each pair
    independently (the pre-FR-030 approach, reproduced directly by
    calling `resolve_contacts_between` twice in sequence) leaves the ball
    at around 98-99% of a single car's own closing speed (~98.9 units/s)
    — as if the first-resolved contact's effect was almost entirely
    discarded by the second; the new combined `resolve_dynamic_manifolds`,
    at this crate's existing `SOLVER_ITERATIONS = 10`, leaves the ball
    noticeably slower (~89.5 units/s in the isolated solver-level
    measurement) — a real, measurable improvement, but *not* full
    convergence to the true zero-velocity answer. This residual error at
    only 10 iterations is a known, common limitation of projected
    Gauss-Seidel iterative contact solvers for an extreme mass-ratio
    "sandwiched" body configuration (a light body pinned between two much
    heavier ones), not unique to this port. It was confirmed during test
    design (not shipped as a change) that increasing the iteration count
    substantially (verified manually with 300 iterations) converges the
    combined solve's result much closer to the true zero-velocity answer,
    while the old independent-pairwise approach's result does *not*
    change at all no matter how many iterations each individual pairwise
    call gets — proving the old approach's error is structural
    (information genuinely thrown away), not an iteration-count
    shortfall. Both tests assert the *direction and magnitude* of the
    improvement (the combined result measurably slower/closer-to-centered
    than the independent-pairwise result), not exact convergence to zero
    — a deliberate, honest test-design choice, matching this project's
    convention (see FR-025's, FR-027's, and FR-029's own entries above) of
    not overclaiming what a test actually proves. New tests: 1 in
    `solver.rs`, 1 in `world.rs` (net +2 tests over FR-029's 242, bringing
    the crate to 244).
- `RB-PHYSICS-001-FR-031` (constant-calibration audit, implemented —
  explicitly does NOT close `FR-005`): `FR-005`'s real calibration against
  recorded ground truth stays blocked on `PHASE-0-EXIT` (no real BakkesMod
  capture exists yet), so this requirement is narrower: source every
  uncalibrated placeholder constant in `drive.rs`/`arena.rs`/`world.rs`
  against the best available community reference, correct the ones with a
  solid one, and explicitly flag the rest as still uncalibrated rather
  than silently leaving their status ambiguous. Sources cross-checked
  directly from source code (not blog paraphrases, which is where this
  project's own earlier placeholder numbers came from): the **RocketSim**
  project (`ZealanL/RocketSim`, `src/RLConst.h`/`Car.cpp`) and
  **RLUtilities** (`samuelpmish/RLUtilities`,
  `src/simulation/car.cc`/`ball.cc`, `src/mechanics/jump.cc`/`dodge.cc`) —
  two independently-written reverse-engineering projects — plus the
  **RLBot community wiki**'s "Useful Game Values" page. Agreement across
  all three, independently, is treated as high-confidence; a single source
  or a casual/older reference is treated as lower-confidence and flagged
  as such below, not silently trusted.
  - **Corrected, with code changes (all expressed in this port's own
    linear-velocity/acceleration/time/distance units, which — unlike a
    torque or an angular rate — don't depend on matching real Rocket
    League's specific car mass/inertia tensor to port correctly):**
    - `drive::JUMP_SPEED`: `292.0` → `875.0/3.0` (≈291.667 uu/s) —
      RocketSim's `JUMP_IMMEDIATE_FORCE = 875.f/3.f` and RLUtilities'
      `Jump::speed = 291.667f` agree exactly; both projects also confirm
      the double jump reuses this same value unmodified, matching what
      this port already did.
    - `drive::JUMP_HOLD_ACCELERATION`: `1400.0` → `4375.0/3.0` (≈1458.33
      uu/s²) — RocketSim's `JUMP_ACCEL = 4375.f/3.f` and RLUtilities'
      `Jump::acceleration = 1458.3333f` agree exactly. Real Rocket League
      also scales this down (0.62x) during the jump's first 0.025s
      (RocketSim's `JUMP_MIN_TIME`) rather than applying it at full
      strength immediately; that two-phase ramp is a further refinement
      this port doesn't model, tracked as a documented simplification, not
      silently dropped.
    - `drive::JUMP_HOLD_MAX_DURATION` (`0.2`): audited, not changed — this
      port's pre-existing value already matches RocketSim's
      `JUMP_MAX_TIME = 0.2f` and RLUtilities' `Jump::max_duration = 0.2f`
      exactly. Recorded here as *confirmed*, not just *unchanged*, so a
      future reader doesn't have to re-derive that this one was already
      right.
    - New `drive::UNBOOSTED_MAX_CAR_SPEED = 1410.0`: a genuine behavioral
      fix, not just a doc update. Before this audit, `drive::MAX_CAR_SPEED`
      (2300, Rocket League's *boosted* top speed — confirmed correct by
      all three sources) doubled as throttle's own speed cap too, letting
      a car reach boosted top speed on throttle alone. RocketSim's
      `DRIVE_SPEED_TORQUE_FACTOR_CURVE` drives available drive torque to
      exactly zero at 1410 uu/s, and the RLBot wiki independently states
      the same 1410 uu/s unboosted cap — so throttle now caps at this new,
      separate constant instead, while `MAX_CAR_SPEED` keeps its
      already-correct role as boost's own cap (and as the turning-torque
      scale-up reference, an arbitrary normalization unaffected by this
      change).
    - `drive::BOOST_ACCELERATION` (`991.667`), `drive::MAX_BOOST` (`100`),
      `world::PhysicsWorld::new`'s gravity (`-650`), and
      `arena::GOAL_DEPTH` (`880`): all audited and confirmed correct
      against the same three sources — no change needed, recorded here as
      confirmed rather than merely unchanged.
  - **Explicitly flagged, NOT changed (real reference numbers exist but
    don't safely port, or no reference exists at all — recorded as
    audited-and-still-uncalibrated, not silently left ambiguous):**
    - `drive::DODGE_SPEED` (`1400.0`): real Rocket League's dodge impulse
      is not a single flat magnitude — RocketSim/RLUtilities agree on a
      base `500.0` uu/s scaled by direction (forward ×1.0, side ×1.9,
      backward ×2.5-ish) and by current forward speed (reduced as speed
      approaches `MAX_CAR_SPEED`). Adopting just the bare `500` would
      collide with `WALL_JUMP_HORIZONTAL_SPEED` (`550`, currently used to
      keep a wall jump distinguishable from a dodge in this port's own
      tests) and would still misrepresent the real direction/speed-scaled
      shape as a flat number — a real fix here is a mechanic redesign
      (its own future FR), not a constant substitution, so the placeholder
      stays as-is.
    - `drive::DODGE_ANGULAR_SPEED` (`5.5` rad/s): real Rocket League
      implements a flip's spin as an applied *torque* (RocketSim's
      `FLIP_TORQUE_X=260`/`FLIP_TORQUE_Y=224` for 0.65s, min 0.41s;
      RLUtilities matches exactly), not a flat angular-velocity kick —
      the resulting spin rate depends on the real game's own specific
      hitbox inertia tensor, which this port's placeholder car body
      doesn't match, so there is no single rad/s number to safely port.
    - `drive::WALL_JUMP_HORIZONTAL_SPEED` (`550.0`): real Rocket League has
      no separate wall-jump speed constant at all — a wall jump just fires
      the ordinary jump impulse (`JUMP_SPEED`) along the wall's contact
      normal instead of straight up (confirmed in both RocketSim and
      RLUtilities). This port's separate, faster constant is a deliberate
      structural simplification (a distinguishable wall-jump feel for
      tests), not an unconfirmed guess anymore, but still not what real
      Rocket League does.
    - `drive::STEER_TORQUE`, `drive::AIR_CONTROL_TORQUE`,
      `drive::HANDBRAKE_FRICTION_MULTIPLIER`,
      `drive::LANDING_AUTO_UPRIGHT_TORQUE`: real reference numbers exist
      for some of these (air-control torque/damping coefficients — pitch
      130, yaw 95, roll 400, capped at an overall 5.5 rad/s, confirmed
      identically in RocketSim and RLUtilities; handbrake as a slip-ratio
      friction *curve*, not a flat multiplier), but all of them are
      calibrated against real Rocket League's own specific car mass and
      inertia tensor, which this port's placeholder `RigidBody::car_box`
      isn't independently confirmed to match — porting the raw numbers
      into a different inertia scale would produce false-precision
      angular accelerations, not a real fix. Remain uncalibrated
      placeholders, now with their real-world analog documented instead of
      simply absent.
    - `arena::FILLET_RADIUS`/`arena::CORNER_ARCH_RADIUS`: audited with no
      analytic single-number reference found anywhere in the serious
      community sources — Rocket League's actual corner geometry is a
      triangulated collision mesh (see `ZealanL/RLArenaCollisionDumper`,
      a tool that extracts the raw mesh rather than a formula), not a
      mathematical arc, and even the RLBot wiki explicitly notes "the
      curvature at the intersections is ignored" in its own published
      wall-length figures. Remain uncalibrated placeholders; closing this
      one for real would mean ingesting an actual dumped mesh, not sourcing
      a better number.
  - **Open ambiguity surfaced by the audit, deliberately not acted on
    without stronger confidence (recorded as a genuinely open question,
    not resolved either way):**
    - Ball radius: this port's `92.75` is an older, casually-cited figure;
      RocketSim's `BALL_COLLISION_RADIUS_SOCCAR`, RLUtilities'
      `Ball::soccar_radius`, and the current `wiki.rlbot.org/v4` rewrite
      all instead converge on `91.25` as the actual simulation collision
      radius (with the ball's *rest height* separately at `93.15`, larger
      because of a mesh collision margin — a different number for a
      different purpose, easy to conflate with the collision radius
      itself). Not changed here: `92.75` is load-bearing across a large
      fraction of this crate's existing tests as a magic number, and
      medium-confidence evidence isn't enough justification to ripple a
      change through all of them in an audit whose own scope is
      "don't misrepresent this as full calibration." Tracked as an open
      question for a future, deliberate change.
    - `arena::CEILING_Z` (`2044.0`) vs. RocketSim's `ARENA_HEIGHT = 2048.f`:
      a small (4 uu) discrepancy the audit surfaced but couldn't resolve
      with confidence — it's unclear whether the two numbers describe the
      same reference point (e.g. usable ceiling height vs. the raw mesh's
      own extent) without deeper investigation this audit's scope doesn't
      cover. Not changed; tracked as an open question.
  - **Acceptance criteria.** Every named constant in `drive.rs`/`arena.rs`
    this audit examined has an explicit, current disposition (corrected,
    confirmed, or flagged-uncalibrated-with-reason) in this entry or in its
    own doc comment — none left in the prior "not derived from any measured
    or documented Rocket League value" state without this audit having
    actually looked for one. No behavior change is claimed beyond the two
    corrected constants and the new `UNBOOSTED_MAX_CAR_SPEED` split; every
    other constant's runtime value is provably unchanged (see the diff).
  - **Verification plan.** `drive.rs` gains
    `throttle_stops_accelerating_at_unboosted_max_speed` (renamed from
    `throttle_stops_accelerating_at_max_speed`, now asserting against
    `UNBOOSTED_MAX_CAR_SPEED` instead of `MAX_CAR_SPEED`) and a new
    `throttle_alone_cannot_reach_the_boosted_top_speed`, holding throttle
    for 20 simulated seconds and confirming the car plateaus near
    `UNBOOSTED_MAX_CAR_SPEED`, well short of the boosted `MAX_CAR_SPEED` —
    the real bug this audit fixed, made concrete as a regression test
    rather than left as a doc-comment-only claim. 1 net new test (245
    total in `rb_physics_bullet`, +1 over FR-030's 244); the `JUMP_SPEED`/
    `JUMP_HOLD_ACCELERATION` precision refinements needed no new tests —
    every existing assertion involving either already used a tolerance
    (`.abs() < 1.0` or looser) comfortably wider than the ~0.33 and ~58.33
    uu/s²-scale differences from their prior placeholder values.
- `RB-PHYSICS-001-FR-032` (genuine convex-vs-curved-surface narrow phase
  investigation, resolved — no code change to the narrow phase itself):
  this requirement set out to replace `box_vs_quarter_pipe`/
  `box_vs_corner_fillet`'s per-corner technique with a real GJK/EPA
  convex-vs-convex narrow phase, on the strength of a limitation FR-027's
  own doc comments claimed: "a face resting flush against a shallow curve
  (a radius large relative to the box) can have every one of its own
  corners still just clear of the fillet while the face's middle already
  overlaps it, under-detecting that case." Building that replacement (a
  from-scratch GJK closest-points implementation, `gjk::closest_points`,
  treating a quarter-pipe's axis as a long finite segment and a corner
  fillet's center as a single point) surfaced a wrong assumption before it
  ever shipped: swapping it in broke two pre-existing, previously-passing
  end-to-end tests
  (`world::tests::a_car_resting_within_a_curved_transitions_footprint_is_pushed_up_off_the_flat_floor_height`,
  `world::tests::a_car_embedded_in_a_compound_corner_fillets_footprint_has_its_penetration_reduced`),
  because it answered a *different* question than the one this contact
  actually needs. A quarter-pipe/corner-fillet's contact test is a
  *containment* question — "is the box's farthest point from
  `axis_point`/`axis_direction` (or a corner fillet's `center`) at or
  beyond `radius`" (see `sphere_vs_quarter_pipe`'s own doc comment: the
  playable side is the *inside* of the curve, so a body is only in
  contact once some part of it reaches or exceeds the boundary) — not a
  *closest*-point question. `gjk::closest_points` correctly finds the
  box's single *nearest* point to the axis/center, which is the *opposite*
  of what determines contact here, and can be a genuinely different point
  of the box than the farthest one. Distance-from-a-line (or
  distance-from-a-point) is a *convex* function of position, and the
  *maximum* of a convex function over a convex polytope (the box) is
  always attained at one of its extreme points — its 8 corners — never a
  face's interior; this is standard convex-analysis fact, not specific to
  this shape. So the per-corner technique isn't approximating the
  farthest-point question at all: it's computing the exact same answer a
  full box-vs-cylinder narrow phase would, just via enumeration instead of
  an iterative solver. The claimed under-detection gap doesn't exist for
  this contact's actual question.
  - **Non-goals (this requirement).** Does not address manifold
    *richness* (when 2+ corners simultaneously violate the radius, each
    is still resolved as its own independent contact point rather than a
    single unified manifold a full convex-vs-convex narrow phase might
    produce) — a genuinely different question from detection accuracy,
    and out of this requirement's scope, since nothing in this
    investigation found evidence it causes a real problem either.
  - **Acceptance criteria.** The claimed under-detection gap is either
    fixed with a genuine narrow phase, or rigorously shown not to exist;
    either way, `box_vs_quarter_pipe`/`box_vs_corner_fillet`'s own doc
    comments (and this crate's other doc comments referencing the same
    claim — `lib.rs`'s crate doc, this spec's own Open questions) reflect
    the actual, verified state, not an unverified inherited claim.
  - **Verification plan.** Proven two ways: (1) analytically — the
    convex-maximum argument above, which is a general mathematical fact,
    not something that needs simulating to be true; (2) empirically, via
    a new `collision.rs` test,
    `no_point_on_a_boxs_face_is_ever_farther_from_a_quarter_pipes_axis_than_its_own_corners`,
    which densely samples (50×50 grid per face) every point across all 6
    faces of a car-sized box positioned exactly the way the two broken
    end-to-end tests above did (resting flat on the floor, close enough to
    a large-radius — 292 uu — wall-floor quarter-pipe that its corners
    straddle the curve), and confirms no sampled face-interior point ever
    exceeds the box's own 8 corners' maximum distance from the axis.
    `box_vs_quarter_pipe`/`box_vs_corner_fillet` themselves are unchanged
    from `RB-PHYSICS-001-FR-027` (the GJK-based replacement was built,
    found to regress two real tests, and reverted rather than shipped —
    the honest outcome of this investigation is a corrected doc comment,
    not new production code). 1 new test (246 total in `rb_physics_bullet`,
    +1 over FR-031's 245).
- `RB-PHYSICS-001-FR-033` (genuine net mesh, implemented, ball only):
  closes the "genuine net mesh" Non-goal `RB-PHYSICS-001-FR-029`'s own doc
  comment left open — until now, `RB-PHYSICS-001-FR-029`'s solid bounding
  box was the ball/car's *entire* interior boundary behind each goal
  window, with no springy/catching netting anywhere. New module `net`
  (`net::NetMesh`) is a rectangular mass-spring grid: `NetMesh::
  rectangular_grid` builds a `cols` x `rows` grid of point masses (each a
  real `RigidBody::sphere`, deliberately tiny and light — see the
  module's own doc comment for why reusing this crate's existing
  rigid-body/collision/solver machinery, rather than a bespoke
  penalty-force system, was the design choice) spanning the goal-mouth
  window's own `arena::GOAL_HALF_WIDTH`/`GOAL_HEIGHT` footprint, anchoring
  (fixing in place) every point on the grid's own perimeter — representing
  the net's real attachment to the rigid goal frame (crossbar, both posts,
  the ground/back line) — and leaving every interior point free, connected
  by structural (horizontal/vertical) and shear (diagonal) springs
  (Hooke's law plus velocity damping along each spring's own axis, the one
  genuinely new piece of physics math this requirement adds — no
  precedent for it exists anywhere else in this Bullet3-derived port,
  since Bullet's own soft-body code was never part of this port's scope —
  see ADR-0004). `NetMesh::step` advances the mesh's own internal physics
  by the caller's `dt`, split into `net::NET_SUBSTEPS` smaller sub-steps
  for numerical stability (a mass-spring system this stiff would go
  unstable integrated with a single large Bullet-style step), and resolves
  the ball's contact against every free point it currently overlaps via a
  new `collision::sphere_vs_sphere` (this crate's first real sphere-vs-sphere
  contact test — `contacts_between` previously returned empty for that
  shape pairing since it had no caller at all) plus the *existing*
  `solver::resolve_contacts_between` two-body path, mutating the ball's own
  velocity progressively across sub-steps exactly the way any other
  dynamic-vs-dynamic contact in this crate already does. New
  `arena::standard_nets` builds one `net::NetMesh` per goal, `NET_DEPTH`
  (an uncalibrated placeholder, less than `GOAL_DEPTH`) behind the real
  back wall — well in front of `FR-029`'s own rigid back-of-net plane,
  which stays completely unchanged as an always-there backstop.
  `PhysicsWorld` gains `nets: Vec<net::NetMesh>` and `with_net`, resolved
  after every other contact each step (`net.step(&mut self.ball, ...)`,
  mutating the ball directly rather than through
  `solver::resolve_dynamic_manifolds`' shared multi-body solve, since a
  net's own points aren't part of that scene-wide body list at all).
  - **Non-goals (this requirement).** A car's own contact against a net —
    a car still passes straight through a `net::NetMesh`'s spatial
    footprint untouched, exactly as it did before this module existed,
    stopped instead by `FR-029`'s pre-existing rigid machinery. A full 3D
    "sock" shape billowing backward from the goal mouth (this models a
    single flat rest-shape panel, which still deforms backward dynamically
    under a real ball impact via its own springs — just not a pre-shaped
    pocket). Bending stiffness (only structural + shear springs; no
    springs resisting the mesh folding along a diagonal). Manifold
    richness beyond one contact per overlapping point. Every new constant
    (`net::NET_POINT_MASS`, `NET_POINT_RADIUS`, `NET_SPRING_CONSTANT`,
    `NET_SPRING_DAMPING`, `NET_LINEAR_DAMPING`, `NET_RESTITUTION`,
    `NET_FRICTION`, `arena::NET_DEPTH`) is an uncalibrated placeholder —
    real Rocket League's actual net material properties have never been
    published, and this port's own point-mass/spring topology is already
    a simplification of a real net's continuum cloth behavior, so a
    "correct" numeric match isn't a coherent target yet either way.
  - **Acceptance criteria.** A ball fired at a lone net panel loses a
    large fraction of its speed compared to firing it through the same
    empty space with no net present — the real "catching" behavior, not
    just a geometric pass/fail contact test. An undisturbed net settles to
    a low residual velocity instead of oscillating forever. Every anchored
    point never moves under any force.
  - **Verification plan.** `net.rs` unit tests, in isolation from
    `PhysicsWorld`: perimeter points are anchored and interior points are
    not; every spring starts at exactly zero stretch (proving
    `rectangular_grid` measures `rest_length` from its own just-built flat
    geometry, not a hardcoded value); anchored points never move under
    120 steps of gravity alone; an undisturbed net's maximum free-point
    speed settles below a low threshold after 600 steps (the mass-spring
    analog of `world::tests::resting_ball_stays_at_rest`); and the real
    catching proof — a ball fired at the net's own center loses over half
    its speed within 1 simulated second (gravity zeroed to isolate the
    net's own effect), verified by manually integrating the ball's own
    transform between `NetMesh::step` calls in the test itself, since
    `NetMesh::step` (matching `PhysicsWorld`'s own staged pipeline)
    mutates only velocity, never a body's position. `collision.rs` gains
    a direct `sphere_vs_sphere` proof (two overlapping spheres produce a
    contact with the correct normal/penetration; two far-apart spheres
    produce none), replacing the old
    `contacts_between_two_spheres_is_empty` regression test whose entire
    premise this requirement reverses. `arena.rs` proves `standard_nets`
    returns exactly 2 nets, each sitting exactly `NET_DEPTH` behind the
    real back wall and spanning exactly the goal mouth's own
    `GOAL_HALF_WIDTH`/`GOAL_HEIGHT` footprint (not an arbitrary size).
    `world.rs` adds a wiring-count test (`standard_arena` carries exactly
    2 nets) plus the real live end-to-end proof: a ball fired at a lone
    net panel (`PhysicsWorld::new` plus `with_net`, isolated from the full
    `standard_arena` for the same full-arena-interference reason
    `RB-PHYSICS-001-FR-029`'s own isolated proofs are isolated) loses at
    least half its speed compared to the identical shot through the same
    scene with no net added — the actual `PhysicsWorld`-integrated
    "catching" proof, not just the isolated `net.rs`-level one. 10 net new
    tests (5 in `net.rs`, 2 in `collision.rs` net of replacing 1, 2 in
    `arena.rs`, 2 in `world.rs`, minus 1 test replaced), bringing the crate
    to 256 total (+10 over FR-032's 246).
- `RB-PHYSICS-001-FR-034` (split impulse, implemented): every contact's
  normal row now also solves a second, entirely separate "push"
  pseudo-velocity channel (`solver::resolve_push_row`/
  `resolve_two_body_push_row`), fed only by that contact's own positional
  (penetration/ERP) error — `ConstraintRow`/`TwoBodyRow` gained a new
  `rhs_penetration` field, split out of the single combined `rhs` this
  port used before this requirement — never the velocity/restitution
  error, which stays on the real `rhs`/`applied_impulse` channel exactly
  as before. Each of `SOLVER_ITERATIONS` iterations resolves both channels
  for every contact's normal row, the push channel's own accumulator
  (`applied_push_impulse`) entirely separate scratch state from the real
  channel's `applied_impulse`; after the loop, the real delta is applied
  to the body's velocity exactly as before, and the new push delta is
  applied directly to the body's position/orientation via a new
  `solver::apply_push_delta` (built on the existing
  `integrate::integrate_transform`, not new integration math) — mirroring
  Bullet's own `btSolverBody::writebackVelocity`, which performs the
  identical second, independent `integrateTransform` call using the push
  velocity right after writing back the real velocity delta. Wired into
  all three of this module's resolve entry points — `resolve_contacts`,
  `resolve_contacts_between`, and `resolve_dynamic_manifolds` (the last
  carrying one `push_deltas[i]` accumulator per body index, shared across
  manifolds the same way its pre-existing real `deltas[i]` already is) —
  with zero call-site changes anywhere outside `solver.rs` (`world.rs`,
  `net.rs`, and every other caller of these three functions is
  unaffected). A friction row never receives positional correction (its
  `rhs_penetration` is always `0.0`), matching Bullet's own split-impulse
  resolve, which only ever runs against a contact's normal row.
  - **Non-goals (this requirement).** Warm-starting and sleeping remain
    exactly as documented before this requirement (see this spec's own
    Non-goals and Open questions) — split impulse and warm-starting are
    independent fixes for two different symptoms of "this port re-derives
    every contact from zero each frame": split impulse stops deep
    penetration from injecting spurious velocity; warm-starting, still
    open, is what would actually let a bouncy resting contact settle. The
    restitution/friction average-not-max combine mode is untouched.
    `LINEAR_SLOP` stays `0.0` (Bullet's own default), so a contact with
    zero or negative penetration still takes the same `positional_error =
    0.0` branch as before this requirement — nothing about this
    requirement changes behavior for an already-settled, non-penetrating
    contact.
  - **Acceptance criteria.** A deeply-penetrating contact between two
    bodies starting and staying at rest (zero restitution, zero incoming
    velocity) leaves the real post-solve velocity along the contact normal
    near zero — no spurious velocity injected purely from resolving
    penetration — while the bodies' positions measurably separate to
    relieve the overlap. A body embedded well past a curved fillet's own
    resting distance, given enough simulated time, settles at (not past)
    that resting distance instead of coasting past it under residual
    velocity the old combined `rhs` term would have left behind.
  - **Verification plan.** Two new `solver.rs` unit tests prove the core
    claim directly:
    `split_impulse_corrects_deep_penetration_via_position_not_velocity`
    (`resolve_contacts`, one body vs. a static plane) and
    `split_impulse_corrects_deep_penetration_via_position_not_velocity_between_two_bodies`
    (`resolve_contacts_between`, two dynamic bodies) — each starts a body
    deeply overlapping with zero restitution and zero incoming velocity,
    resolves once, and checks the real velocity along the contact normal
    stayed near zero while position/separation moved measurably. All 12 of
    `solver.rs`'s pre-existing tests (resting, bouncing, friction,
    momentum, multi-body-pinch, box-symmetry) pass unchanged, confirming
    the `rhs`/`rhs_penetration` split is behavior-preserving for every
    case they already covered — the key piece of upfront analysis this
    requirement relied on: every one of those fixtures uses either zero
    relative velocity at exact, zero-penetration contact (`touching_ball`,
    `symmetric_pinch`, `resting_sphere`) or otherwise already computed
    `positional_error` as `0.0` before this requirement, so splitting it
    into a separate channel changes nothing observable for them.
    `world.rs`'s existing curved-fillet "embedded past resting distance"
    live end-to-end proofs
    (`a_ball_embedded_in_a_vertical_corner_edges_fillet_footprint_is_pushed_toward_the_axis`,
    `a_ball_embedded_in_a_compound_corner_fillets_footprint_is_pushed_toward_the_center`,
    `a_ball_embedded_in_a_goal_posts_fillet_footprint_is_pushed_toward_the_axis`,
    `a_ball_embedded_in_a_goal_corner_fillets_footprint_is_pushed_toward_the_center`)
    had their own assertions tightened by this requirement: before it,
    each only checked the ball moved "meaningfully" toward the resting
    surface, since the old combined `rhs` term left residual velocity for
    the ball to keep coasting on after the correction resolved; after it,
    each instead checks the ball settles at (not past) its exact resting
    distance, since the push channel no longer leaves any such residual
    velocity behind — a direct, live-`PhysicsWorld` confirmation of this
    requirement's own claim, not just an isolated `solver.rs`-level one.
    2 new tests, bringing the crate to 258 total (+2 over FR-033's 256).
- `RB-PHYSICS-001-FR-035` (warm-starting, implemented for
  `resolve_dynamic_manifolds` only): a new `solver::ContactCache` carries a
  manifold's converged real-channel impulses (normal plus both friction
  rows) from one call to the next, matched by each contact's approximate
  world position (`CONTACT_MATCH_DISTANCE`, an uncalibrated placeholder
  mirroring Bullet's own fixed `gContactBreakingThreshold`-style matching,
  since this port's narrow phase re-derives every contact from scratch and
  has no genuine per-point stable ID to track instead). Before iterating,
  a new `warm_start_two_body_row` applies each row's cached impulse
  directly to that manifold's shared `DeltaVelocity` accumulators — not
  merely setting `TwoBodyRow::applied_impulse` to the cached value, which
  (with `GLOBAL_CFM` always `0.0`) would leave the first iteration's
  correction unchanged from a cold start; the seed has to be baked into
  the starting delta itself, mirroring Bullet's own warm-start applying
  the cached impulse to the solver body's temporary velocity at setup
  time, before any iteration runs. `resolve_dynamic_manifolds` gained a
  new `caches: &mut HashMap<(usize, usize), ContactCache>` parameter (key
  normalized `(min, max)` so either argument order finds the same entry);
  every call rebuilds `caches` from only that call's own manifolds,
  replacing rather than merging, so a pair no longer touching is dropped
  automatically. `PhysicsWorld` gains one `dynamic_manifold_caches` field,
  persisted across steps and passed into its one
  `resolve_dynamic_manifolds` call site.
  - **Non-goals (this requirement).** `resolve_contacts`/
    `resolve_contacts_between` don't take a `ContactCache` — this port's
    fixed `SOLVER_ITERATIONS` already fully converges every one-body and
    two-body scenario this crate tests, so warm-starting either has no
    scenario to demonstrate value against yet; the wiring is deliberately
    scoped to the one call site (`resolve_dynamic_manifolds`) where a
    documented under-convergence limitation already exists
    (`RB-PHYSICS-001-FR-030`'s own extreme-mass-ratio "sandwiched" case),
    since that's the only place in this crate warm-starting can currently
    change anything observable. Consequently, static contacts (ground,
    walls, curves, corner fillets, goal walls, bounded walls, for both the
    ball and every car) are NOT warm-started, even though the same
    `ContactCache` mechanism could seed `resolve_contacts` identically if
    a future scenario needs it — deferred, not an oversight. Warm-starting
    does NOT resolve this spec's own documented "bouncy resting contact
    never settles" limitation (see this module's own doc comment): that
    symptom comes from restitution re-triggering off a fresh
    gravity-induced closing velocity every frame, not from where the
    solver's iteration starts, so warm-starting converges the same
    wrong-looking bounce faster without stopping it from recurring —
    sleeping (still unimplemented) is the actual fix for that, a
    genuinely separate mechanism.
  - **Acceptance criteria.** All of `solver.rs`'s pre-existing tests
    (`resolve_dynamic_manifolds`'s own included) pass unchanged when
    called with a fresh, empty `ContactCache` map, confirming
    warm-starting is a no-op unless something has actually been cached.
    Given the same post-call-1 state, seeding call 2 from call 1's
    converged impulses lands measurably closer to the true converged
    answer than repeating call 2 cold (a fresh, empty cache) does, for a
    manifold `RB-PHYSICS-001-FR-030` already showed doesn't fully converge
    within one call's `SOLVER_ITERATIONS`.
  - **Verification plan.** One new `solver.rs` test — mangled here only by
    this spec's own line width, its real name has no line break —
    `warm_starting_a_sandwiched_ball_across_two_calls_converges_closer_than_a_repeated_cold_start`,
    reuses `symmetric_pinch` (the same extreme-mass-ratio scenario
    `resolve_dynamic_manifolds_keeps_more_of_every_bodys_contact_than_resolving_pairs_independently`
    already uses): call 1 (cold) partially converges and populates a
    cache; from that identical post-call-1 state, call 2 is then run
    twice on independent copies — once warm (reusing call 1's cache) and
    once cold (a fresh map) — with identical positions, contacts,
    velocities, and iteration budget both times, isolating exactly what
    the warm seed itself contributes. The warm run's ball ends up
    measurably slower (closer to the true zero-velocity equilibrium) than
    the cold repeat's. All 14 of `solver.rs`'s pre-existing tests pass
    unchanged when given an empty cache, confirming this requirement is
    behavior-preserving for every case they already covered. 1 new test,
    bringing the crate to 259 total (+1 over FR-034's 258).
- `RB-PHYSICS-001-FR-036` (constant-ambiguity resolution, implemented): a
  dedicated follow-up to `RB-PHYSICS-001-FR-031`'s own audit, resolving the
  two genuine ambiguities that audit surfaced but deliberately didn't act
  on, using real source-level research (cloning and reading RocketSim's and
  RLUtilities' own source, and the current RLBot wiki, rather than guessing
  from prior training-data recall the way FR-031's own initial framing
  apparently had). Ball radius: FR-031 framed this as a straight "`92.75`
  vs. `91.25`" choice, but the real games actually split the ball into two
  separate radii — a smaller inertia radius (`91.25`) and a distinctly
  larger collision radius (`93.15`, the mesh's own collision margin) — a
  distinction this port's `RigidBody::sphere` has no room for, since it
  carries a single unified radius used for both roles and this port has no
  separate Bullet-style collision margin of its own. The mathematically
  correct single-constant analog for this port's unified field is therefore
  the collision radius, so every `92.75` literal (`solver.rs`, `world.rs`,
  `net.rs`, `collision.rs`) became `93.15`, not `91.25` — switching to
  `91.25` would have been a regression, not a fix. `arena::CEILING_Z`:
  confirmed, via both RocketSim's own `ARENA_HEIGHT = 2048.f` and an
  independent reconstruction from real extracted collision-mesh geometry,
  to describe the same reference point this port's `CEILING_Z` does, so
  `2044.0` became `2048.0`. Two further corrections were made as a low-risk
  byproduct of the same research pass, not new findings requiring their own
  requirement: `arena::CORNER_LENGTH` (the octagon corner-cut inset) was
  wrongly documented, by FR-031 and FR-019 before it, as an uncalibrated
  placeholder with no public reference at all — it's actually confirmed
  exact against real extracted collision-mesh data, so only its doc
  comments changed, not its value (`1152.0`, already correct); likewise
  `arena::GOAL_DEPTH` was wrongly documented, by FR-029, as an uncalibrated
  invention with no reference — it's confirmed against the current RLBot
  wiki's own cited value, so again only its doc comments changed
  (`880.0`, already correct).
  - **Non-goals (this requirement).** `arena::FILLET_RADIUS` and
    `arena::CORNER_ARCH_RADIUS` remain untouched and still genuinely
    uncalibrated — the research this requirement relied on found no
    analytic single-number reference for either anywhere in the serious
    community sources (the real corner/transition geometry is a
    triangulated collision mesh, not a mathematical arc), the same
    conclusion FR-031's own audit already reached; closing that gap for
    real would mean ingesting an actual dumped mesh, a genuinely different
    and more involved follow-up this requirement's research explicitly
    recommended treating separately rather than folding in here. No new
    behavioral bug motivated this requirement — like FR-031 before it, it's
    a constant-correctness change with no associated fix to solver or
    collision logic, so (matching FR-031's own precedent) no new test was
    added; the fix is proven by the existing suite still passing unchanged.
    This requirement does not touch `RB-VERIFY-002` real-data calibration,
    still blocked on `PHASE-0-EXIT`.
  - **Acceptance criteria.** `arena::CEILING_Z` reads `2048.0` and every
    ball-radius literal in `solver.rs`/`world.rs`/`net.rs`/`collision.rs`
    reads `93.15` (not `91.25`); `arena::CORNER_LENGTH` and
    `arena::GOAL_DEPTH` keep their existing values, with doc comments no
    longer misdescribing either as uncalibrated. All 259 pre-existing tests
    across the crate pass unchanged, since every affected test used the
    ball-radius/`CEILING_Z` constants themselves rather than a hardcoded
    duplicate of the old values, so the correction is transparent to them.
  - **Verification plan.** No new tests: this is a constant-only correction
    with no new behavior to characterize, the same precedent FR-031
    established for its own constant changes. `cargo test --workspace`
    re-run clean at 259 total (unchanged from FR-035) confirms the four
    source-file substitutions and the two `arena.rs` constant/doc-comment
    edits are behavior-preserving everywhere the old values were previously
    exercised. A targeted `grep -c "892\.75"` across the four
    non-`arena.rs` files, run before the substitution, confirmed zero
    matches in each — ruling out any risk of the mechanical `92.75` →
    `93.15` substitution corrupting `arena::GOAL_HALF_WIDTH`'s unrelated
    `892.755` literal, which contains `92.75` as a substring but lives only
    in `arena.rs`, deliberately excluded from that substitution and edited
    by hand instead.
- `RB-PHYSICS-001-FR-037` (sleeping, implemented): closes the "no sleeping"
  half of `solver`'s own documented gap `RB-PHYSICS-001-FR-035` left open
  (warm-starting closed the other half — see that entry) — the actual fix
  for a *bouncy* resting contact (restitution > 0) never settling, since
  restitution re-triggers off a fresh gravity-induced closing velocity
  every frame regardless of where the solver's own iteration starts, so
  nothing about warm-starting or split impulse could ever stop the
  residual bounce, only refusing to integrate it at all once it's small
  and old enough to call "at rest" does. New `body::RigidBody` fields
  `is_sleeping: bool` (public) and `sleep_timer: f32` (private), and two
  new methods: `update_sleep_state(&mut self, dt: f32)` — call once per
  step, after every contact is resolved but before the transform
  integrates — accumulates `sleep_timer` while both
  `linear_velocity.length()` and `angular_velocity.length()` stay under
  new constants `body::LINEAR_SLEEP_VELOCITY_THRESHOLD`/
  `ANGULAR_SLEEP_VELOCITY_THRESHOLD`, setting `is_sleeping = true` once
  `sleep_timer` reaches `body::SLEEP_TIME_THRESHOLD` and forcibly zeroing
  both velocities every call thereafter (repeated every subsequent call
  while still under threshold, since gravity/restitution would otherwise
  recompute a fresh nonzero value each step); crossing either threshold
  resets the timer and clears `is_sleeping` immediately — and
  `wake(&mut self)`, which does the same reset unconditionally, independent
  of velocity. `PhysicsWorld::step` calls `update_sleep_state` for the ball
  and every car right after the net panels step (every other contact
  already resolved) and right before `integrate_transform_and_refresh_inertia`,
  so a body newly asleep this step also freezes in place this same step.
  `drive::apply_driven_forces` calls `car.wake()` unconditionally, before
  anything else in that call runs, whenever a new private helper
  `input_is_active` finds `input` genuinely active (any nonzero
  throttle/steer/analog channel, or `jump`/`boost`/`handbrake` set) —
  necessary because a resultant-velocity-only wake check isn't enough for a
  driven body: a car accelerating from rest under a small per-frame driving
  force whose own one-frame velocity delta is itself smaller than
  `LINEAR_SLEEP_VELOCITY_THRESHOLD` would otherwise have that delta zeroed
  right back out every single frame by the still-asleep check, permanently
  stranding it. `input_is_active` treats an unrecovered analog channel
  (`None`) the same as a recovered-but-literally-neutral one (`Some(0.0)`)
  — both mean "no analog input this tick" — rather than the simpler
  `*input != ControllerInput::default()`, which would treat any `Some(0.0)`
  as active purely because it isn't `None`, keeping a car fed a real
  recorded input stream that always resolves every channel (even at rest)
  from ever sleeping at all.
  - **Non-goals (this requirement).** No persistent-island sleeping the way
    real Bullet's own architecture does it (a body wakes or sleeps
    independently of whatever else it's touching) — this crate's solver
    already treats each body independently rather than via Bullet's
    persistent islands (see `solver`'s own module doc comment), so
    per-body sleeping is the natural fit, not a simplification of a richer
    mechanism this crate lacks. No kinematic/deactivation-disabled body
    concept exists to exempt from sleeping, since this crate has no
    kinematic bodies at all. `LINEAR_SLEEP_VELOCITY_THRESHOLD`,
    `ANGULAR_SLEEP_VELOCITY_THRESHOLD`, and `SLEEP_TIME_THRESHOLD` are this
    project's own uncalibrated placeholders — no public reference states
    what threshold, if any, real Rocket League's own physics engine uses
    internally for this (a purely implementation-internal stabilization
    detail no replay or capture could ever directly reveal, even with real
    `RB-VERIFY-002` data); chosen only to sit clearly above this crate's own
    single-frame gravity/restitution velocity noise and clearly below any
    deliberate motion this crate models (see the constants' own doc
    comment in `body.rs` for the specific numbers behind that reasoning). A
    sleeping body waking only from a contact impulse or active input, never
    from a sustained-but-below-threshold external push (e.g. a very slow
    car nudging it), is a deliberate modeling choice matching real Bullet's
    own behavior, not a bug.
  - **Acceptance criteria.** A ball resting on the ground with nonzero
    restitution on both surfaces eventually has its velocity forced to
    exactly zero and `is_sleeping` set, instead of bouncing indefinitely.
    A car seeded already asleep wakes immediately (within the same step)
    the moment it receives genuinely active input, and actually
    accelerates that same step. A sleeping ball wakes when a moving car's
    contact impulse pushes its resultant velocity back above threshold,
    with no special-case contact-wake logic required beyond the ordinary
    `update_sleep_state` check. Every one of this crate's pre-existing
    tests passes unchanged, confirming sleeping doesn't alter any
    already-covered scenario's outcome within the timeframes those tests
    already used.
  - **Verification plan.** 5 new `body.rs` unit tests exercise
    `update_sleep_state`/`wake` directly and in isolation: under-threshold
    velocity doesn't sleep before the time threshold elapses; sustained
    under-threshold velocity does sleep and zeroes both velocities;
    velocity above either threshold alone never sleeps; a sleeping body
    regaining speed above threshold wakes immediately without `wake()`;
    and `wake()` itself clears both `is_sleeping` and the timer regardless
    of velocity (checked by confirming a single further sub-threshold `dt`
    right after waking isn't enough on its own to re-sleep). 3 new
    `world.rs` end-to-end tests prove the same claims through a live
    `PhysicsWorld`:
    `a_bouncy_resting_ball_actually_settles_once_asleep` (a nonzero-restitution
    ball/ground pair, run long enough to fall asleep, asserting both
    `is_sleeping` and exactly-zero velocity — the direct fix for the
    limitation `resting_ball_stays_at_rest`'s own comment, now corrected,
    used to document instead of demonstrate), `a_sleeping_car_wakes_up_the_instant_throttle_is_applied`
    (a car seeded already asleep via direct field assignment, not simulated
    settling, isolating the wake-on-input claim from how long settling
    itself takes), and `a_sleeping_ball_wakes_up_when_a_moving_car_hits_it`
    (a ball put to sleep before the car even exists in the scene, then a
    fast-moving car added and driven into it). 8 new tests, bringing the
    crate to 267 total (+8 over FR-036's 259).
- `RB-PHYSICS-001-FR-039` (wall-jump corner disambiguation, implemented):
  closes the "first wall in `self.walls`" simplification `RB-PHYSICS-001-FR-013`
  originally documented and `FR-019`'s new diagonal corner walls made
  reachable in the standard arena for the first time (see both entries'
  own Non-goals). `PhysicsWorld::step`'s per-car wall-normal computation
  (feeding `drive::apply_driven_forces`'s `wall_normal` parameter) now
  collects *every* wall a car is touching this step, sums their normals,
  and normalizes the result, instead of `Iterator::find`-ing the first
  match. A car touching exactly one wall gets that wall's own normal back
  unchanged — summing a single unit vector and normalizing it is a no-op —
  so the far more common single-wall case is bit-for-bit unaffected (every
  pre-existing wall-jump test, including
  `a_car_touching_a_wall_wall_jumps_outward_and_upward`, passes unchanged).
  A car touching two walls at once now pushes off diagonally, along the
  sum of both normals, rather than firing along only one of them depending
  on which wall happens to be earlier in `PhysicsWorld.walls`. No new
  collision code was needed: `resolve_plane_contact` already resolved a
  car touching two walls simultaneously correctly (each wall's own contact
  independently, every step) — only the wall-jump *push-off direction*
  picker was affected, since it's `drive::apply_driven_forces`'s own
  input, not part of physical contact resolution at all.
  - **Non-goals (this requirement).** No genuine multi-wall solid-angle
    weighting (e.g. giving a near-tangent wall's normal less influence than
    a near-perpendicular one) — an unweighted sum, same as summing
    unit-length contact normals in `collision`'s own multi-contact
    manifolds elsewhere in this crate. No change to physical contact
    resolution itself (`resolve_plane_contact` already handled simultaneous
    multi-wall contact correctly, see FR-013) — this closes only the
    wall-jump-direction gap, not a collision-detection one. The
    exactly-opposite-normals degenerate case (summing to the zero vector,
    geometrically impossible for a convex arena interior but handled
    defensively by falling back to the first touched wall's normal) is not
    exercised by any test, since no reachable scene in this crate's own
    arena or test helpers can produce it.
  - **Acceptance criteria.** A car touching exactly one wall wall-jumps
    identically to before this change (all pre-existing wall-jump tests
    pass unchanged). A car touching two walls at a right-angle corner
    wall-jumps with both horizontal velocity components positive and, for
    a symmetric (equal-angle) corner, roughly equal in magnitude — proof
    the push-off direction blends both walls rather than picking one.
  - **Verification plan.** New `world.rs` test
    `a_car_touching_two_walls_at_a_corner_wall_jumps_diagonally_outward`:
    two perpendicular walls (normals `(1,0,0)` and `(0,1,0)`), a car
    positioned to touch both simultaneously (zero gap on each, matching the
    existing single-wall test's own contact convention), jump pressed while
    airborne with gravity zeroed out to isolate the wall jump. Asserts both
    `vx > 0.0` and `vy > 0.0` (the old "first wall wins" picker would zero
    one of them) and `|vx - vy| < 1.0` (the two walls' normals are
    symmetric, so a true blend gives equal components), plus the same
    roughly-`JUMP_SPEED` vertical check every wall-jump test already makes.
    1 new test, bringing the crate to 268 total (+1 over FR-037's 267).
- `RB-PHYSICS-001-NFR-001` (implemented): The physics core doesn't force
  Bullet-specific data modeling into `rb_domain` — `rb_domain::state`
  stays a plain state DTO plus general-purpose vector/quaternion algebra;
  `rb_physics_bullet` owns all rigid-body/solver-specific types.

## Architecture and interfaces

`rb_physics_bullet` (new crate, depends only on `rb_domain`):
- `mat3`: `Mat3`, a general 3x3 matrix — needed because a box's inertia
  tensor is anisotropic (unlike a sphere's scalar/isotropic inertia).
- `body`: `RigidBody` (dynamic; a `Shape` enum — `Sphere` or `Box` —
  picks the collision geometry and local inertia formula), `StaticPlane`
  (immovable). One `RigidBody` type serves both shapes, matching Bullet's
  own architecture (`btRigidBody` + a polymorphic `btCollisionShape`)
  rather than a separate rigid-body type per shape. `StaticQuarterPipe`
  (also immovable, since `RB-PHYSICS-001-FR-020`) is a second static shape
  alongside `StaticPlane` — a partial-cylinder fillet, with its own
  `between_planes` constructor deriving its geometry from two flat planes.
  `StaticCornerFillet` (also immovable, since `RB-PHYSICS-001-FR-023`) is a
  third static shape — a sphere blending three flat planes at a single
  vertex, with its own `between_three_planes` constructor deriving its
  center and containment `bounds` from those three planes directly.
  `StaticGoalWall` (also immovable, since `RB-PHYSICS-001-FR-024`) is a
  fourth static shape — a `StaticPlane` plus a rectangular window in the
  plane's own local `u_axis`/`v_axis` frame, with `contains_in_window`
  testing a point's projection onto that frame directly.
  `StaticBoundedWall` (also immovable, since `RB-PHYSICS-001-FR-029`) is a
  fifth static shape — a `StaticPlane` plus a rectangular *bound* in the
  plane's own local `u_axis`/`v_axis` frame (`bound_center`/`half_u`/
  `half_v`), with `contains_in_bound` testing a point's projection onto
  that frame directly, the opposite gate convention from
  `StaticGoalWall`'s window (collides only *inside* the bound, not
  everywhere *except* inside a window).
- `integrate`: force accumulation, velocity integration, transform
  integration — pure functions over `RigidBody`, shape-agnostic.
- `collision`: `contacts_vs_plane` — analytic body-vs-static-plane contact
  generation (any plane, not just the ground — an arena wall is the exact
  same test with a different normal), dispatching to a sphere- or
  box-specific test and returning a manifold (`Vec<Contact>`, 0 to 4
  points); `contacts_vs_quarter_pipe` (since FR-020) — analytic
  sphere-vs-fillet contact generation for a sphere (always 0 or 1 points),
  dispatching to `box_vs_quarter_pipe`'s corner-testing (exact, not an
  approximation, per `RB-PHYSICS-001-FR-032`; 0-8 points) for a box since
  `RB-PHYSICS-001-FR-027` (a box always
  returned no contact through FR-026, see FR-020's original Non-goals);
  `contacts_vs_corner_fillet` (since
  FR-023) — the same analytic sphere-only contact generation against a
  `StaticCornerFillet` instead, using its spherical-triangle containment
  test in place of a `StaticQuarterPipe`'s 2-sided sector one, and the same
  `box_vs_corner_fillet` corner-testing dispatch for a box since FR-027;
  `contacts_vs_goal_wall` (since FR-024) — dispatches by shape instead of
  always analytic sphere-only: a sphere gets `sphere_vs_goal_wall`'s
  windowed treatment (no contact for a center inside the window), and,
  since `RB-PHYSICS-001-FR-028`, a box gets the equivalent per-corner
  windowed treatment via `box_vs_goal_wall` instead of falling straight
  through to an unwindowed `contacts_vs_plane` the way it did through
  FR-027 — a car can now actually drive into a goal through the same
  window the ball already could pass through;
  `contacts_vs_bounded_wall` (since `RB-PHYSICS-001-FR-029`) — dispatches by
  shape against a `StaticBoundedWall` instead: a sphere gets
  `sphere_vs_bounded_wall`'s bounded treatment (no contact for a center
  outside the bound), and a box gets `box_vs_bounded_wall`'s per-corner
  bounded treatment, the same "test every corner" technique
  FR-027/FR-028 established, but skipping a corner *outside* the bound
  instead of *inside* a window;
  `contacts_between` — dispatches to `sphere_vs_box` (0 or 1 points) or the
  separating-axis `box_vs_box` (0 to 4 points), covering every
  two-dynamic-body shape pairing this crate has.
- `solver`: `resolve_contacts` — sequential-impulse contact + friction
  resolution over an entire manifold against one static body, identified
  only by its `restitution`/`friction` (since FR-020, this serves a
  `StaticQuarterPipe` fillet exactly as it already served a `StaticPlane` —
  the static shape's actual geometry is irrelevant here, already baked into
  the caller's own `Contact` list); `resolve_contacts_between` — the same
  sequential-impulse math generalized to two dynamic bodies' shared contact
  manifold; `resolve_dynamic_manifolds` (since `RB-PHYSICS-001-FR-030`) —
  the scene-wide entry point `PhysicsWorld::step` actually calls: every
  ball-vs-car and car-vs-car manifold touching in a step shares one
  interleaved `SOLVER_ITERATIONS`-iteration budget instead of each pair
  getting its own fully independent `resolve_contacts_between`-style
  solve, via a new `delta_pair_mut` disjoint-borrow helper letting
  multiple manifolds that share a body index share that body's single
  `DeltaVelocity` accumulator too.
- `drive`: `apply_driven_forces` — couples a car's `ControllerInput` into
  ground throttle/steering forces and torques, a boost force/resource
  drain, a handbrake-driven temporary friction adjustment, a
  rising-edge-triggered ground jump impulse (with a continuous
  `JUMP_HOLD_ACCELERATION` hold-window bonus for variable height, driven by
  `jump_hold_time_remaining`), airborne pitch/yaw/roll torque, a second
  rising-edge-triggered airborne jump impulse (double
  jump, gated on and consuming a `double_jump_available` flag rather than
  ground contact — either a plain vertical `JUMP_SPEED` kick or, when
  `pitch`/`roll` exceed `DODGE_DEADZONE` at the moment of the press, a
  directional dodge: a horizontal `DODGE_SPEED` impulse plus an
  instantaneous `DODGE_ANGULAR_SPEED` spin written directly to
  `RigidBody.angular_velocity`, also arming a `dodge_flip_active` flag a
  further fresh press can spend to flip-cancel — zero the spin outright —
  before landing or a wall touch re-arms the double jump), and a third jump
  variant fired instead of the double-jump-or-dodge branch when a
  `wall_normal` (the outward normal of a touched wall, if any) is present —
  a plain outward-plus-upward impulse that restores rather than consumes
  `double_jump_available` below `DODGE_DEADZONE`, or — at or above it — a
  wall-jump dodge combining that same push-off with a `DODGE_SPEED`
  horizontal component and `DODGE_ANGULAR_SPEED` spin, which *does* consume
  `double_jump_available` and arms `dodge_flip_active` exactly like a
  ground dodge, and — whenever airborne with no active `pitch`/`roll` and
  no fresh jump press this step — a gentle continuous `LANDING_AUTO_UPRIGHT_TORQUE`-scaled restoring torque nudging the car's local up axis
  back toward world up (not a Bullet3 port — this project's own model of
  Rocket League's driving mechanics, since the real numbers aren't public;
  see the module's own doc comment for which constants are commonly-cited
  community estimates vs. uncalibrated placeholders).
- `world`: `PhysicsWorld::step`/`frame`, and `simulate()` — the
  composition root Bullet's `btDiscreteDynamicsWorld::stepSimulation`
  corresponds to, run in the same staged order (integrate every body's
  velocity — for cars, including `drive::apply_driven_forces` — then
  resolve every contact — ground, every wall, every curve, every
  corner fillet, every goal wall, and every bounded wall for every body,
  every ball-vs-car
  pair, then every
  car-vs-car pair — then integrate
  every body's transform). `PhysicsWorld` carries one ball (`RigidBody`,
  always present), `walls: Vec<StaticPlane>` (any number, via repeated
  `with_wall` calls, empty by default), `curves: Vec<StaticQuarterPipe>`
  (since FR-020; any number, via repeated `with_curve` calls, empty by
  default — deflected the ball only, a no-op for every car, through
  FR-026; since `RB-PHYSICS-001-FR-027`, a car is deflected too, via the
  8-corner testing approximation `collision::box_vs_quarter_pipe`
  provides), `corner_fillets: Vec<StaticCornerFillet>` (since FR-023; any
  number, via
  repeated `with_corner_fillet` calls, empty by default — same
  ball-and-since-FR-027-car deflection convention as `curves`, via
  `collision::box_vs_corner_fillet`), `goal_walls: Vec<StaticGoalWall>`
  (since FR-024; any number, via repeated `with_goal_wall` calls, empty by
  default — unlike `curves`/`corner_fillets`, resolved for *every* body
  from the start, and, since `RB-PHYSICS-001-FR-028`, a car passes through
  the window too, via `collision::box_vs_goal_wall`'s own per-corner
  window test), `bounded_walls: Vec<StaticBoundedWall>` (since
  `RB-PHYSICS-001-FR-029`; any number, via repeated `with_bounded_wall`
  calls, empty by default — like `goal_walls`, resolved for every body
  from the start, via `collision::contacts_vs_bounded_wall`), and
  `cars: Vec<RigidBody>` (any number, via repeated `with_car` calls) with a
  parallel
  `car_inputs: Vec<ControllerInput>` set via `set_car_input`, a parallel
  `car_boost: Vec<f32>` set via `set_car_boost`, a parallel
  `car_base_friction: Vec<f32>` snapshotted from each car's own friction by
  `with_car` (handbrake's restore target), a parallel
  `car_jump_held: Vec<bool>` (jump's rising-edge state, starting `false`),
  a parallel `car_double_jump_available: Vec<bool>` (starting `true`,
  restored on landing or wall contact, consumed by an airborne double
  jump), a parallel `car_jump_hold_time_remaining: Vec<f32>` (starting
  `0.0`, the ground jump's variable-height hold window), and a parallel
  `car_dodge_flip_active: Vec<bool>` (starting `false`, whether the most
  recent double-jump-or-dodge press left a cancelable flip active); each
  car's current wall contact (if any), like its ground contact, is computed
  fresh at the start of every `step` from its position at the time.
  `frame()` assigns each car's `player_id` as its index in `cars` and
  reports its current input and boost amount.
- `arena`: `standard_ground`/`standard_walls` — Rocket League's real
  standard-arena field dimensions and a 7-`StaticPlane` boundary (2 side
  walls, a ceiling, 4 diagonal corner walls — the back walls moved out as
  of FR-024, see below) built from `body::StaticPlane` alone (no new
  collision code); `standard_curves` (since FR-020, extended to all 9
  walls' floor/ceiling seams by FR-021, and to the 8 corner-wall vertical
  edges by FR-022) — 24 `StaticQuarterPipe` fillets total: 16 floor-side/
  ceiling-side fillets (one pair per wall, all 9 walls including the 4
  diagonal corner walls since FR-021, and still built from `back_wall_plane`
  directly even though the back walls themselves left `standard_walls`)
  plus 8 vertical-edge fillets (one per corner-wall endpoint, since
  FR-022), all built via `StaticQuarterPipe::between_planes` from those
  same flat planes — a corner wall's own floor/ceiling-seam
  `axis_direction` is computed via a cross product rather than
  hand-picked, since (unlike a cardinal wall's) it isn't a coordinate
  axis, while a vertical-edge fillet's own `axis_direction` is simply
  `(0, 0, 1)` (the edge itself is vertical); since FR-025, the 8 of those
  16 floor/ceiling-seam fillets that bridge a corner wall (rather than a
  cardinal one) to the floor or ceiling are built with the distinctly
  larger `CORNER_ARCH_RADIUS` in place of `FILLET_RADIUS`, while the other
  8 cardinal-wall seam fillets and all 8 vertical-edge fillets keep
  `FILLET_RADIUS` unchanged; `standard_corner_fillets`
  (since FR-023) — 16 `StaticCornerFillet`s, one per compound corner (4 per
  corner wall — floor+side, floor+back, ceiling+side, ceiling+back — times
  the 4 corner walls), all built via `StaticCornerFillet::between_three_planes`
  from those same three flat planes directly, not from the two
  `StaticQuarterPipe`s `standard_curves` builds at that vertex, and, since
  FR-025, all built with `CORNER_ARCH_RADIUS` rather than `FILLET_RADIUS`
  — every one of them touches one of the 8 now-larger arches, and
  `between_three_planes` needs one shared radius across the three planes it
  blends to still meet an arch exactly where their axes cross;
  `standard_goal_walls` (since FR-024) — 2 `StaticGoalWall`s, one per back
  wall, each wrapping the same `back_wall_plane` `standard_curves` already
  uses, windowed at `GOAL_HALF_WIDTH`/`GOAL_HEIGHT`;
  `standard_goal_cutout_fillets` (since FR-024) — 6 more
  `StaticQuarterPipe`s (two posts and a crossbar per goal), built via
  `StaticQuarterPipe::between_planes` from the real back-wall plane and a
  purely-geometric post/crossbar plane (`goal_post_plane`/
  `goal_crossbar_plane`, never themselves added as real collision walls);
  `standard_goal_corner_fillets` (since FR-026) — 4 more
  `StaticCornerFillet`s (one per goal post per goal), built via
  `StaticCornerFillet::between_three_planes` directly from those same
  back-wall/post/crossbar planes, rounding off the two compound corners per
  goal where a post's own edge fillet meets the crossbar's, added to the
  same `corner_fillets` list `standard_corner_fillets`'s 16 already
  populate (20 total) and reusing `FILLET_RADIUS` unchanged;
  `standard_goal_back_walls` (since `RB-PHYSICS-001-FR-029`) — 2 plain,
  unbounded `StaticPlane`s (one per goal), `GOAL_DEPTH` behind the real
  back wall, added to `PhysicsWorld.walls` (now 9 real entries, up from 7,
  once `standard_arena` is built);
  `standard_goal_side_walls` (since FR-029) — 4 `StaticBoundedWall`s total
  (2 per goal), each reusing `goal_post_plane` unchanged, bounded to the
  goal's own depth and height; `standard_goal_roofs` (since FR-029) — 2
  `StaticBoundedWall`s total (1 per goal), each reusing
  `goal_crossbar_plane` unchanged, bounded to the goal's own width and
  depth — all 6 of these `StaticBoundedWall`s go into a new
  `PhysicsWorld.bounded_walls` field via `with_bounded_wall`, not the
  `walls`/`curves`/`corner_fillets` lists any prior static shape used;
  `PhysicsWorld::standard_arena` (in `world`)
  wires all of these into a new `PhysicsWorld` in one call, an alternative to
  `PhysicsWorld::new` plus manual `with_wall`/`with_curve`/
  `with_corner_fillet`/`with_goal_wall`/`with_bounded_wall` calls for a
  caller that wants the real field rather than a custom test arena.

No `PhysicsStateSource`-style trait exists yet for "the physics engine"
specifically — `rb_verify_cli` calls `rb_physics_bullet::simulate`
directly. A trait is worth introducing once a second physics core
implementation actually exists to justify it (per the "no speculative
abstraction before two real call sites" convention this project follows
throughout) — not before.

## Data/state and invariants

World convention: +Z is up (matching Unreal Engine, which Rocket League
runs on). Sphere inertia is isotropic (`I = 2/5 m r^2`, same value on all
three axes); box inertia is anisotropic (`I = m/12 * (b^2 + c^2)` per
axis, from the box's full dimensions). Both are stored as
`RigidBody.inv_inertia_local` (a diagonal, in the body's own local frame)
and combined with the body's current orientation into
`inv_inertia_world` (a full `Mat3`) via `update_inertia_tensor` — called
once per step, after the transform integrates. A sphere's `inv_inertia_world`
is mathematically orientation-independent (`R * kI * R^T == kI` for any
rotation `R`), so this generalization doesn't change sphere behavior from
the previous scalar-only representation (see
`body::tests::sphere_inertia_tensor_is_orientation_independent`).

## Errors, failure, recovery, and observability

No fallible operations — `RigidBody::new` panics on non-physical input
(zero/negative mass, or a zero/negative radius or half-extent), matching
"trust internal callers, validate at real boundaries" (a physics body's
own constructor is such a boundary; a malformed body is a programming
error, not a recoverable runtime condition).

## Security, privacy, and compatibility

None beyond `THIRD_PARTY_NOTICES.md`'s zlib attribution obligations.

## Acceptance criteria

- Sphere (met): free-fall matches semi-implicit Euler kinematics before
  impact; an inelastic resting contact stays at rest; a dropped ball
  settles near the ground; restitution produces a bounce proportional to
  the combined coefficient; friction decelerates a sliding sphere and
  couples into spin.
- Box/FR-004 (met): free-fall matches the same kinematics as a sphere
  (shape-independent integration); box-vs-plane contact generation
  produces the correct point count for flat (4), edge-tilted (2), and
  embedded (4, positive penetration) cases; a box's inertia tensor changes
  with orientation while a sphere's doesn't; a box dropped flat settles on
  the ground without tipping onto an edge or corner (multi-contact
  resolution keeping symmetric contacts symmetric); a box resting flat
  with a small downward velocity settles to zero net rotation (no spurious
  torque from resolving 4 contacts one at a time). Sphere-vs-box contact
  generation is correct at the surface, under overlap, and for a sphere
  center embedded inside the box (pushed out via the nearest face); the
  two-body solver conserves linear momentum, produces no residual closing
  speed for an inelastic collision, and leaves a much heavier body
  (the car) barely moving from a much lighter body's (the ball's) impact;
  an end-to-end `PhysicsWorld::step` test confirms a ball shot at a
  stationary car actually bounces off it rather than tunnelling through.
- FR-006 (met): `box_vs_box` correctly reports no contact for far-apart
  boxes, a 4-point manifold with correct depth and normal for a symmetric
  flat overlap, a normal/depth pair antisymmetric in argument order
  (matching the sphere-vs-box case), and a partial (fewer-than-4-point)
  manifold for a non-flat rotated overlap; the generalized
  `resolve_contacts_between` settles two colliding boxes' face-to-face
  manifold without spurious net rotation, the same property already
  verified for the one-body ground-manifold case. `PhysicsWorld` builds a
  multi-car scene from repeated `with_car` calls, assigns each car a
  sequential `player_id`, and — the real end-to-end proof — two cars shot
  head-on at each other in a live `PhysicsWorld::step` loop actually
  bounce off each other instead of tunnelling through.
- FR-007 (met, ground throttle/steering): a neutral input applies no
  force or torque (so a car with no input set is unaffected); throttle
  accelerates a grounded car forward, has no effect while airborne, stops
  accelerating at `MAX_CAR_SPEED`, and reverse throttle accelerates
  backward; steering has no effect on a stationary car but yaws a moving
  one, in the opposite direction for opposite `steer` sign; an end-to-end
  `PhysicsWorld::step` loop with `set_car_input` set to full throttle
  drives a car forward across the ground, and `frame()` reports that same
  input back.
- FR-008 (met, boost): boost accelerates a car regardless of ground
  contact (an end-to-end `PhysicsWorld::step` loop with gravity zeroed and
  full boost input drives a car forward while airborne); the boost tank
  drains over time while held and clamps at zero; boost has no effect once
  the tank is empty; boost still drains the tank even once the car is at
  `MAX_CAR_SPEED` and the forward force stops applying; a new car starts
  with a full tank (`MAX_BOOST`), and `frame()` reports the live
  `boost_amount` instead of a hardcoded `0.0`.
- FR-009 (met, handbrake): handbrake reduces friction while grounded, has
  no effect on friction while airborne, and releasing it restores the
  car's own base friction (not a hardcoded default, verified with a
  car constructed with a non-default friction); an end-to-end
  `PhysicsWorld::step` loop confirms a car already sliding sideways
  retains more of that slide under handbrake's reduced friction than
  under normal grip.
- FR-010 (met, single ground jump): jump gives a grounded car upward
  velocity, has no effect while airborne, doesn't re-fire on a second call
  while still held, and fires again after a release-then-re-press; an
  end-to-end `PhysicsWorld::step` loop confirms a car with jump input
  actually leaves the ground, and — holding jump for the car's entire
  flight, never released — confirms it lands and settles instead of being
  relaunched every time it touches back down.
- FR-011 (met, air control): pitch/yaw/roll each produce angular velocity
  about the correct local axis for a stationary airborne car (no speed
  requirement, unlike ground steering); air control has no effect while
  grounded; a `None` analog value behaves like neutral input; opposite-sign
  yaw spins the opposite way. An end-to-end `PhysicsWorld::step` loop
  (gravity zeroed) confirms a car with yaw input actually reorients itself
  mid-air, and a regression test confirms a grounded car stays level
  despite stray pitch/yaw/roll input.
- FR-012 (met, double jump): a fresh airborne jump press gives upward
  velocity when the double jump is available, has no effect when it isn't,
  is consumed after firing once (a release-then-re-press while still
  airborne doesn't refire it), and touching the ground restores
  availability. An end-to-end `PhysicsWorld::step` loop (gravity zeroed)
  confirms a double jump fired after a ground jump adds a second
  `JUMP_SPEED` kick on top of the first, and a regression test confirms a
  spent double jump doesn't fire again mid-air no matter how many more
  times jump is released and re-pressed before landing.
- FR-013 (met, arena walls and wall jump): a fresh jump press while
  airborne and touching a wall pushes the car outward along the wall's
  normal and upward; has no effect while grounded even if `wall_normal` is
  `Some`; takes priority over the double jump without consuming it; and
  merely touching a wall (no jump press needed) restores the double jump.
  An end-to-end `PhysicsWorld::step` test confirms a car resting against a
  wall wall-jumps outward and upward on a fresh press; a second end-to-end
  test confirms a ball shot at a wall bounces off it instead of tunnelling
  through — the same physical proof `ball_bounces_off_a_stationary_car_instead_of_passing_through` already gives for cars, now for the generic
  plane-collision machinery walls reuse; a regression test confirms a car
  not actually touching an existing wall still gets a plain double jump,
  not a wall jump.
- FR-014 (met, dodge): a fresh double-jump press with `pitch` (forward) or
  `roll` (sideways) held gives horizontal velocity along the matching axis
  plus a visible spin, in the opposite direction for opposite stick sign; a
  deflection below `DODGE_DEADZONE` still gives a plain double jump;
  either way the press spends `double_jump_available`; a diagonal
  (`pitch`+`roll`) press combines both axes; dodge logic never fires while
  grounded (the ground jump owns that branch entirely) or while touching a
  wall (the wall jump fires its own fixed push-off regardless of stick
  input). An end-to-end `PhysicsWorld::step` test confirms a car dodges
  forward with a visible flip after a ground jump, and a regression test
  confirms a car touching a wall with directional stick input still gets
  the wall jump's own (smaller, purely horizontal-plus-vertical) push-off
  rather than the dodge's (larger, purely horizontal) one. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014 behavior covered
  by `rb_physics_bullet`'s unit tests (120 tests as of the 0.14.0 version).
- FR-015 (met, variable jump height): holding jump after a fresh ground
  jump adds more upward velocity than tapping it; releasing jump early
  stops the extra acceleration immediately, even with hold-window time
  left; the extra acceleration stops accruing once
  `JUMP_HOLD_MAX_DURATION` has elapsed, even if still held; a double jump
  fired after holding the ground jump through its whole window still adds
  exactly one more `JUMP_SPEED` kick, not an extra variable-height boost.
  An end-to-end `PhysicsWorld::step` test confirms a held ground jump
  reaches a greater peak height than a tapped one; a regression test
  confirms the same double-jump-unaffected property holds through a live
  `PhysicsWorld::step` loop, not just in `drive.rs` isolation; a second
  regression test (`holding_jump_does_not_repeatedly_relaunch_the_car`,
  extended for the longer flight time variable height now produces) still
  confirms holding jump for a car's entire flight lands and settles it
  instead of relaunching it every touchdown. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015 behavior
  covered by `rb_physics_bullet`'s unit tests (126 tests as of the 0.15.0
  version).
- FR-016 (met, flip-cancel): a dodge leaves the car spinning and sets a
  cancelable-flip flag; a further fresh jump press while airborne, not
  touching a wall, with the double jump already spent, zeroes the spin
  outright and spends the flag; flip-cancel touches neither the dodge's own
  linear velocity nor `double_jump_available`; a plain double jump (no
  stick input) explicitly clears any stale cancelable-flip flag left over
  from an earlier, already-landed-from dodge, so a later unrelated press
  can't spuriously flip-cancel nothing; a wall jump still takes priority
  over flip-cancel on a fresh press while touching a wall. An end-to-end
  `PhysicsWorld::step` test confirms a second jump press cancels a dodge's
  spin in a live scene; a regression test confirms landing and a later
  plain double jump clear a stale cancelable-flip flag there too, not just
  in `drive.rs` isolation — verified by confirming both the `drive.rs` and
  `world.rs` versions of that regression actually fail without the
  explicit-clear fix, not just that they pass with it. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016
  behavior covered by `rb_physics_bullet`'s unit tests (132 tests as of
  the 0.16.0 version).
- FR-017 (met, wall-jump dodge): a fresh wall-jump press with directional
  stick input at or above `DODGE_DEADZONE` fires a combined wall-push-plus-
  dodge impulse and a visible spin instead of the plain fixed push-off;
  below the deadzone the plain wall jump fires unchanged; the dodge variant
  consumes `double_jump_available` while the plain variant still doesn't;
  its spin can be flip-cancelled by a further press, exactly like a ground
  dodge's; opposite stick sign dodges the opposite direction; a diagonal
  (pitch+roll) wall-jump dodge combines both axes. An end-to-end
  `PhysicsWorld::step` test confirms the wall-jump dodge fires in a live
  scene; a second end-to-end test confirms its spin is flip-cancelable
  there too. Two pre-existing tests whose premise this requirement
  deliberately reverses (`drive::wall_jump_fires_instead_of_a_dodge_when_touching_a_wall`, `world::wall_jump_still_fires_instead_of_a_dodge_when_touching_a_wall`) were repurposed, not silently deleted, to assert the
  new behavior. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017
  behavior covered by `rb_physics_bullet`'s unit tests (138 tests as of
  the 0.17.0 version).
- FR-018 (met, landing auto-orientation assist): a tilted airborne car with
  no pitch/roll input gets a corrective torque; an already-upright airborne
  car gets none; the assist has no effect while grounded; and it doesn't
  fire while pitch or roll air control is actively held (checked via a
  tilt whose own correction axis is orthogonal to full pitch's own torque
  axis, so the two contributions can be cleanly told apart). An end-to-end
  `PhysicsWorld::step` test (gravity zeroed) confirms a car tilted 90
  degrees with no input trends back toward level over repeated steps rather
  than staying tilted or drifting further away. A pre-existing regression
  test (`landing_and_a_new_double_jump_clears_a_stale_dodge_flip_flag_in_a_live_world`) was loosened from exact equality to a small tolerance, since
  the assist now legitimately nudges angular velocity by a tiny amount on
  the test's intervening neutral-input step — still tight enough to catch a
  real regression (a spurious flip-cancel zeroing ~1.5 rad/s), which would
  dwarf the assist's own per-step contribution. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018
  behavior covered by `rb_physics_bullet`'s unit tests (143 tests as of
  the 0.18.0 version).
- FR-019 (met, modeled arena footprint): `standard_walls` returns exactly
  9 planes; the arena's center is on the playable side of every one of
  them; opposing side/back walls share one offset magnitude by
  construction; a point just past a side wall is no longer on the
  playable side; the ceiling bounds from above (playable below `CEILING_Z`,
  not above); a corner wall actually cuts off the true rectangular corner
  (that point is not on the playable side of its corresponding corner
  wall); all four corner walls share one offset magnitude. An end-to-end
  `PhysicsWorld::standard_arena` test confirms it carries exactly 9 walls
  and the standard ground; a second confirms a ball shot at the standard
  arena's side wall bounces off it rather than escaping (the same physical
  proof FR-013 already gave for an ad-hoc test wall, now for the real field
  dimension); a third confirms a ball fired straight at the true
  rectangular corner is stopped by the diagonal corner wall well before its
  x or y individually reaches either the side or back wall's own position —
  proof the corner cut is real physical geometry, not decoration. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019
  behavior covered by `rb_physics_bullet`'s unit tests (153 tests as of
  the 0.19.0 version).
- FR-020 (met, curved wall-to-floor/wall-to-ceiling transitions):
  `StaticQuarterPipe::between_planes` derives an axis sitting exactly
  `radius` units in from both bridged planes, with `sector_start`/
  `sector_end` pointing toward each plane's own tangent point (both unit
  vectors, perpendicular to each other), and those tangent points lying
  exactly on their respective planes. `sphere_vs_quarter_pipe`: a sphere
  deep inside the pipe has no contact; a sphere touching the pipe's own
  radius (from inside) has zero penetration; a sphere pushed past that
  radius has positive penetration with the correction pointing back toward
  the axis (not away from it, unlike a flat plane); a sphere outside the
  fillet's 90-degree sector has no contact regardless of absolute distance;
  a box always gets no contact (the documented deferred case). An
  end-to-end `PhysicsWorld` test confirms a ball resting at ordinary
  flat-floor height within a curve's footprint — already overlapping the
  fillet's own material — gets pushed up off that flat height instead of
  staying embedded, the real proof the curve is live physical geometry, not
  a detection hack; a regression test confirms a car (box) sitting in the
  exact same position is completely unaffected, staying at its ordinary
  flat-floor resting height. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019/FR-020
  behavior covered by `rb_physics_bullet`'s unit tests (168 tests as of
  the 0.20.0 version).
- FR-021 (met, curved corner-wall-to-floor/wall-to-ceiling transitions):
  `standard_curves` returns exactly 16 fillets instead of 8; every fillet's
  axis sits exactly `FILLET_RADIUS` in from some vertical wall — a side
  wall, a back wall, or a diagonal corner wall — not just a cardinal one;
  a corner wall's own derived fillet axis sits exactly `FILLET_RADIUS` in
  from both the corner wall and the floor, with the same perpendicular
  unit-vector sector properties FR-020 already proved for the cardinal-wall
  case; the cross product computing a corner wall's `axis_direction` is
  exactly unit length for every one of the 4 quadrants, confirming the
  production code's `.normalize()`-free assumption actually holds rather
  than merely compiling. An end-to-end `PhysicsWorld` test, built around a
  wall with a diagonal (non-axis-aligned) normal rather than going through
  `arena::standard_curves` directly, confirms `between_planes` genuinely
  generalizes to a non-cardinal wall: a ball resting at ordinary flat-floor
  height within that diagonal wall's fillet footprint gets pushed up off it,
  the same real physical-geometry proof FR-020 gave for a cardinal wall,
  now for one whose normal isn't a coordinate axis. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019/FR-020/FR-021
  behavior covered by `rb_physics_bullet`'s unit tests (172 tests as of
  the 0.21.0 version).
- FR-022 (met, curved corner-wall vertical-edge fillets): `standard_curves`
  returns exactly 24 fillets instead of 16; every fillet's axis (all 24,
  floor/ceiling-seam and vertical-edge alike) sits exactly `FILLET_RADIUS`
  in from some vertical wall; every vertical-edge fillet's own
  `axis_direction` runs purely along Z, unlike a floor/ceiling-seam
  fillet's horizontal one; a corner wall's own derived vertical-edge fillet
  axis sits exactly `FILLET_RADIUS` in from both the corner wall and its
  neighboring side wall, with a sector spanning exactly the 45-degree angle
  between their two normals (not the floor-seam fillets' 90 degrees).
  `between_planes`'s generalization is independently verified with a
  synthetic non-perpendicular fixture (a wall meeting a second wall at 45
  degrees, unrelated to the arena's own geometry): the derived axis still
  sits exactly `radius` in from both planes with tangent points exactly on
  each; the derived sector angle matches the angle between the two planes'
  normals exactly; the sharp corner the fillet replaces sits outside its
  own radius but within its sector (the real proof the generalized sector
  orientation actually faces the missing material, not away from it); and
  passing either of the two opposite directions as `axis_direction`
  produces the same correctly-oriented sector either way, confirming the
  self-correction is real and not an artifact of a particular input sign.
  An end-to-end `PhysicsWorld` test confirms a ball embedded past a
  vertical-edge fillet's own radius (deep in what would otherwise be the
  sharp corner sliver, at a wall-to-wall angle that isn't a right angle)
  gets pushed meaningfully back toward the axis — not a claim that it
  settles and stays at the exact resting distance, since (like every other
  fillet in this port) its contact stops firing once the overlap resolves,
  after which nothing cancels whatever residual velocity the correction
  left the ball with; `RB-PHYSICS-001-FR-020`'s and `FR-021`'s own
  equivalent tests make the same "moved meaningfully," not
  "settled-and-stayed," claim for exactly this reason. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019/FR-020/FR-021/FR-022
  behavior covered by `rb_physics_bullet`'s unit tests (181 tests as of
  the 0.22.0 version).
- FR-023 (met, compound-corner fillets): `StaticCornerFillet::between_three_planes` places its center exactly `radius` units in from
  all three bridged planes, with each plane's own tangent point lying
  exactly on that plane; its 3 `bounds` correctly include the direction
  toward the sharp corner it replaces and exclude the direction pointing
  straight away from that corner. `standard_corner_fillets` returns exactly
  16 fillets; every one of their centers sits exactly `FILLET_RADIUS` in
  from some floor-or-ceiling plane, some side-or-back wall, and some corner
  wall simultaneously — proof `between_three_planes` solved the real triple
  intersection this arena's geometry produces, not an arbitrary point.
  `sphere_vs_corner_fillet`: a sphere deep inside the fillet has no
  contact; a sphere touching the fillet's own radius (from inside) has zero
  penetration; a sphere pushed past that radius has positive penetration
  with the correction pointing back toward the center (not away from it,
  unlike a flat plane); a sphere outside the fillet's spherical-triangle
  bounds has no contact regardless of absolute distance; a box always gets
  no contact (the documented deferred case, same as every other fillet
  here). `PhysicsWorld::standard_arena` carries exactly 16 corner fillets.
  An end-to-end `PhysicsWorld` test confirms a ball embedded past a
  compound-corner fillet's own radius (deep in what would otherwise be the
  sharp, unrounded corner where three planes meet) gets pushed meaningfully
  back toward the center — the same "moved meaningfully," not
  "settled-and-stayed," claim `RB-PHYSICS-001-FR-020`'s/`FR-021`'s/`FR-022`'s
  own equivalent tests make, for exactly the same reason (a single-fire
  contact stops firing once the overlap resolves, and nothing cancels
  whatever residual velocity the correction left the ball with). All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019/FR-020/FR-021/FR-022/FR-023
  behavior covered by `rb_physics_bullet`'s unit tests (194 tests as of
  the 0.23.0 version).
- FR-024 (met, goal cutouts): `StaticGoalWall::contains_in_window`
  correctly reports a point at the window's own center, and points just
  inside/outside each of its four edges (checked independently of the
  point's own depth from the plane, proving the test really is
  distance-along-the-plane-independent as designed). `sphere_vs_goal_wall`:
  a sphere embedded in the goal window has no contact at all (passes
  through); a sphere outside the window behaves exactly like an ordinary
  plane contact, both embedded (positive penetration) and resting exactly
  at the surface (zero penetration). `contacts_vs_goal_wall`'s box path is
  bit-for-bit identical to plain `contacts_vs_plane` against the same
  wrapped plane, proving a car really does see the same wall as before,
  window or not. `standard_walls` returns exactly 7 planes (down from 9);
  `standard_goal_walls` returns exactly 2, sharing one offset magnitude,
  each window centered exactly on its own wall at half the goal's own
  height with the documented `GOAL_HALF_WIDTH`/`GOAL_HEIGHT` half-extents.
  `standard_goal_cutout_fillets` returns exactly 6 fillets, each sitting
  exactly `FILLET_RADIUS` in from both a real back wall and a post/crossbar
  plane. `PhysicsWorld::standard_arena` carries exactly 7 walls, 30 curves
  (24 edge/corner-wall fillets plus the 6 new goal-cutout ones), and 2 goal
  walls. Two end-to-end `PhysicsWorld` tests give the real live-physics
  proof: a ball fired straight through the center of a goal-mouth window
  keeps going past the back wall's own position instead of bouncing off
  it, while a car aimed at the exact same spot is still stopped by the
  wall, completely unaffected by the window; a third end-to-end test
  confirms a ball embedded past a goal-post fillet's own radius gets
  pushed meaningfully back toward the axis, the same "moved meaningfully,"
  not "settled-and-stayed," claim every other fillet's own equivalent test
  makes, for the same residual-velocity reason. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019/FR-020/FR-021/FR-022/FR-023/FR-024
  behavior covered by `rb_physics_bullet`'s unit tests (211 tests as of
  this version).
- FR-025 (met, corner-wall floor/ceiling arch radius): a compile-time
  `const _: () = assert!(CORNER_ARCH_RADIUS > FILLET_RADIUS);` right after
  `CORNER_ARCH_RADIUS`'s own definition proves the "distinctly larger"
  relationship at build time rather than at runtime. `standard_curves`
  still returns exactly 24 fillets, but `every_floor_or_ceiling_seam_curve_bridges_a_wall_to_the_floor_or_ceiling` now checks the first 8 of those
  24 entries (the cardinal-wall seams) against `FILLET_RADIUS` and the
  next 8 (the corner-wall seams) against `CORNER_ARCH_RADIUS` separately,
  instead of checking all 16 floor/ceiling entries against one shared
  radius as before; `every_standard_curve_sits_radius_in_from_a_vertical_wall` now accepts either radius, since a curve's distance from its own
  bridged vertical wall is `FILLET_RADIUS` for a cardinal-wall or
  vertical-edge fillet and `CORNER_ARCH_RADIUS` for a corner-wall
  floor/ceiling seam. `standard_corner_fillets` still returns exactly 16
  fillets, but `every_standard_corner_fillets_center_sits_radius_in_from_a_floor_or_ceiling_a_side_or_back_wall_and_a_corner_wall` now checks
  `CORNER_ARCH_RADIUS` instead of `FILLET_RADIUS`, since all 16 switched
  together. A new end-to-end `PhysicsWorld` test,
  `a_ball_embedded_in_a_corner_walls_floor_arch_footprint_is_pushed_toward_the_axis`, gives the real live-physics proof: a ball embedded past
  `arena::CORNER_ARCH_RADIUS` at a diagonal corner wall's own floor seam —
  deep enough that it would sit outside a plain `arena::FILLET_RADIUS`
  fillet's own footprint entirely — still gets pushed meaningfully back
  toward the axis, the same "moved meaningfully," not
  "settled-and-stayed," claim every other fillet's own equivalent test
  makes, for the same residual-velocity reason, and the test additionally
  asserts `CORNER_ARCH_RADIUS > FILLET_RADIUS` directly. While validating
  this change, the pre-existing `world.rs` end-to-end test
  `a_ball_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall`
  (FR-024) started failing: `StaticQuarterPipe` is documented as infinite
  along its own axis, not clipped to a corner wall's real, finite span, so
  a ball fired dead down the arena's own center line eventually re-enters
  *some* corner-wall floor-seam arch's resting shell far past the goal —
  already true before FR-025 with the smaller `FILLET_RADIUS` (verified
  directly against the pre-FR-025 code, where the ball drifts into this
  zone around y≈7650-7930 and gets a mild, harmless correction that still
  leaves it past the wall), but FR-025's bigger `CORNER_ARCH_RADIUS` moves
  that zone closer in (~y=6300-7700) and turns the same brush into a much
  sharper, solver-destabilizing correction (velocity spikes to tens of
  thousands of units/sec, throwing the ball back past the wall and failing
  the test's assertion). This is a discovered-and-fixed test-scoping
  issue, not a new capability or a new documented Non-goal —
  `StaticQuarterPipe`'s "infinite along its own axis" simplification was
  already documented in `body.rs` before this increment. The fix:
  shortened that one test's simulated flight duration from 3.0s to 1.8s,
  comfortably long enough to prove the ball clears the back wall (needs
  y > 5121, reaches y=5400 unobstructed by 1.8s) while stopping well short
  of re-entering the infinite-fillet zone, with a code comment in the test
  explaining why. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019/FR-020/FR-021/FR-022/FR-023/FR-024/FR-025
  behavior covered by `rb_physics_bullet`'s unit tests (212 tests as of
  this version — net +1 over 0.24.0's 211, since one new `arena.rs` test
  idea was implemented as the compile-time const-assert above instead of a
  runtime test, alongside the one new `world.rs` end-to-end test).
- FR-026 (met, goal post-crossbar corner fillets):
  `standard_goal_corner_fillets` returns exactly 4 fillets; every one of
  their centers sits exactly `FILLET_RADIUS` in from some back wall, some
  post plane, and the crossbar plane simultaneously — proof
  `between_three_planes` solved the real triple intersection this goal's
  own geometry produces, not an arbitrary point, the same "prove the real
  triple intersection, not an arbitrary point" style test FR-023's own
  arena-corner test already used. `PhysicsWorld::standard_arena` carries
  exactly 20 corner fillets (16 arena-corner plus 4 goal-corner). An
  end-to-end `PhysicsWorld` test,
  `a_ball_embedded_in_a_goal_corner_fillets_footprint_is_pushed_toward_the_center`,
  gives the real live-physics proof: a ball embedded past a goal
  corner fillet's own radius, at a synthetic back-wall/post/crossbar
  3-plane fixture (not the real arena's own numbers, matching this test
  file's established convention for fillet unit tests), gets pushed
  meaningfully back toward the center — the same "moved meaningfully," not
  "settled-and-stayed," claim every other fillet's own equivalent test
  makes, for the same residual-velocity reason. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019/FR-020/FR-021/FR-022/FR-023/FR-024/FR-025/FR-026
  behavior covered by `rb_physics_bullet`'s unit tests (215 tests as of
  this version — net +3 over 0.25.0's 212: 2 new `arena.rs` tests
  (`standard_goal_corner_fillets_has_four_fillets` and
  `every_goal_corner_fillets_center_sits_radius_in_from_a_back_wall_a_post_and_the_crossbar`)
  plus the 1 new `world.rs` end-to-end test above).
- FR-027 (met, car deflection by curved fillets): `collision.rs` gained
  two pairs of tests, one pair per curved shape, proving the corner-testing
  approximation is real physical geometry for a box, not a detection hack
  or a silent no-op: `box_embedded_in_the_quarter_pipes_footprint_has_contact`
  centers a car box directly on a quarter-pipe's own surface (along its
  sector bisector) and confirms at least one real contact comes back, with
  every contact's normal pointing back toward the axis (the radial vector
  from the axis to the contact point has a negative dot product with the
  contact normal); `box_far_from_the_quarter_pipe_has_no_contact` places a
  box deep on the opposite side of the fillet's own angular sector — not
  just far away radially, since a point far along the sector's own bisector
  direction is still angularly "inside" the wedge regardless of distance —
  and confirms no contact at all; `box_embedded_in_the_corner_fillets_footprint_has_contact`
  and `box_outside_the_corner_fillets_bounds_has_no_contact` give the same
  two proofs for the sphere-shaped compound-corner fillet
  (`StaticCornerFillet`) instead of the cylindrical quarter-pipe. `world.rs`
  gained two live end-to-end `PhysicsWorld` proofs:
  `a_car_resting_within_a_curved_transitions_footprint_is_pushed_up_off_the_flat_floor_height`
  replaces the old `a_car_is_not_deflected_by_a_curved_transition` regression
  test (whose entire premise this requirement reverses) — a car resting at
  flat-floor height, at the exact position the analogous ball test already
  uses (already overlapping a wall-to-floor curve's own material), gets
  pushed up off that height, mirroring the ball's own equivalent proof;
  `a_car_embedded_in_a_compound_corner_fillets_footprint_has_its_penetration_reduced`
  checks, for a car embedded in a compound-corner fillet, that the worst
  (maximum) corner penetration this fillet reports shrinks meaningfully
  after the solver runs — deliberately not that the box's own center of
  mass moves closer to the fillet's center, the way the equivalent ball
  test does, since a sphere is a single point but an oriented box has
  multiple corners at different depths simultaneously: resolving one
  corner's contact can rotate the box in a way that moves its center of
  mass *away* from the fillet's center even as every individual corner's
  own overlap shrinks, so "distance from center-of-mass to fillet center"
  isn't the right invariant for a box the way it is for a sphere. This was
  found empirically while writing the test — an earlier version asserting
  center-of-mass distance shrank actually failed (the car ended up farther
  from the fillet's center even though the fillet was correctly resolving
  penetration), which is what led to writing the assertion this more
  careful, per-corner way instead. All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019/FR-020/FR-021/FR-022/FR-023/FR-024/FR-025/FR-026/FR-027
  behavior covered by `rb_physics_bullet`'s unit tests (218 tests as of
  this version — net +3 over 0.26.0's 215: the 4 `collision.rs` tests above
  replace the 2 pre-existing "always empty for a box" regression tests
  FR-027's behavior change made obsolete, net +2, plus the 2 `world.rs`
  tests above replace the 1 pre-existing `a_car_is_not_deflected_by_a_curved_transition`
  regression test, net +1).
- FR-028 (met, car actually driving into a goal): `collision.rs` replaced
  the old `box_vs_goal_wall_ignores_the_window_entirely` regression test,
  whose entire premise this requirement reverses, with three new tests —
  `box_squarely_inside_the_goal_window_has_no_contact` (every corner
  inside the window gives an empty manifold, the box equivalent of the
  pre-existing `sphere_embedded_in_the_goal_window_has_no_contact`),
  `box_straddling_the_goal_window_edge_only_collides_on_the_corners_still_outside_it`
  (a car centered exactly on the window's own edge gets exactly 2
  contacts, only on the corners whose x-coordinate falls outside the
  window — the real proof of the partial-block behavior this
  requirement's own entry describes), and
  `box_entirely_outside_the_goal_window_behaves_like_an_ordinary_plane`
  (a car nowhere near the window collides identically to plain
  `contacts_vs_plane`, the box equivalent of the pre-existing
  `sphere_outside_the_goal_window_behaves_like_an_ordinary_plane`) — net
  +2 (3 new replacing 1 old). `world.rs` replaced
  `a_car_is_still_stopped_by_the_standard_arenas_back_wall_at_the_goal_mouth`,
  whose entire premise this requirement reverses, with
  `a_car_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall`
  — a live end-to-end `PhysicsWorld` proof: a car fired at the exact same
  goal-mouth-center position/velocity the pre-existing ball test
  `a_ball_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall`
  uses ends up past `BACK_WALL_Y` instead of being stopped, reusing that
  test's same 1.8s flight-duration bound for the same already-documented
  reason (see FR-025's own entry, unrelated to FR-028 itself) — and added
  a new regression guard,
  `a_car_aimed_away_from_the_goal_mouth_is_still_stopped_by_the_back_wall`
  (a car aimed well outside `GOAL_HALF_WIDTH`, at the solid part of the
  wall, is still stopped by it after 3.0s, proving this requirement only
  opens the window itself, not the rest of the wall) — net +1 (2 new
  replacing 1 old). All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019/FR-020/FR-021/FR-022/FR-023/FR-024/FR-025/FR-026/FR-027/FR-028
  behavior covered by `rb_physics_bullet`'s unit tests (221 tests as of
  this version — net +3 over 0.27.0's 218: 2 net in `collision.rs` plus 1
  net in `world.rs`).
- FR-029 (met, modeled goal interior): `body.rs` gained 4 new tests for
  `StaticBoundedWall::contains_in_bound`
  (`contains_in_bound_is_true_for_the_bounds_own_center`,
  `contains_in_bound_is_true_just_inside_each_edge`,
  `contains_in_bound_is_false_just_outside_each_edge`,
  `contains_in_bound_ignores_distance_from_the_plane_itself`) — mirroring
  the pre-existing `StaticGoalWall::contains_in_window` tests exactly, just
  with the boolean gate meaning inverted. `collision.rs` gained 5 new tests
  against a synthetic fixture
  (`sphere_inside_the_bound_behaves_like_an_ordinary_plane`,
  `sphere_outside_the_bound_has_no_contact`,
  `box_squarely_inside_the_bound_behaves_like_an_ordinary_plane`,
  `box_straddling_the_bounds_edge_only_collides_on_the_corners_still_inside_it`,
  `box_entirely_outside_the_bound_has_no_contact`), mirroring the
  `StaticGoalWall` collision tests with the gate inverted. `arena.rs`
  gained 8 new tests proving the new geometry functions place things
  correctly (`standard_goal_back_walls_has_two_walls`,
  `every_goal_back_wall_sits_goal_depth_behind_the_real_back_wall`,
  `standard_goal_side_walls_has_four_walls`,
  `every_goal_side_walls_plane_matches_some_goal_post_plane`,
  `every_goal_side_walls_bound_covers_the_real_goal_depth_and_height`,
  `standard_goal_roofs_has_two_roofs`,
  `every_goal_roofs_plane_is_the_goal_crossbar_plane`,
  `every_goal_roofs_bound_covers_the_real_goal_width`) — the same "prove
  the real geometry, not an arbitrary point" discipline this crate's other
  arena tests already use. `world.rs` gained 1 new wiring-count test
  (`standard_arena_has_six_bounded_walls`) and 3 new live end-to-end
  `PhysicsWorld` proofs —
  `a_ball_shot_into_the_goal_is_stopped_by_the_goal_back_wall`,
  `a_ball_shot_sideways_inside_the_goal_is_stopped_by_a_goal_side_wall`,
  `a_ball_shot_upward_inside_the_goal_is_stopped_by_the_goal_roof` —
  each deliberately isolated to a minimal `PhysicsWorld` built from just
  the specific new wall(s) under test rather than the full
  `PhysicsWorld::standard_arena` every other end-to-end goal test in this
  file uses (see Verification plan for the two real test-design findings —
  a sector-membership isolation issue and a wall-restitution-zeroing fix —
  this discovery led to); `PhysicsWorld.walls` growing from 7 to 9 real
  entries also renamed the pre-existing
  `standard_arena_has_seven_walls_and_the_standard_ground` to
  `standard_arena_has_nine_walls_and_the_standard_ground` (a test-count
  correction, not a new test). All
  FR-007/FR-008/FR-009/FR-010/FR-011/FR-012/FR-013/FR-014/FR-015/FR-016/FR-017/FR-018/FR-019/FR-020/FR-021/FR-022/FR-023/FR-024/FR-025/FR-026/FR-027/FR-028/FR-029
  behavior covered by `rb_physics_bullet`'s unit tests (242 tests as of
  this version — net +21 over 0.28.0's 221: 4 in `body.rs`, 5 in
  `collision.rs`, 8 in `arena.rs`, 4 in `world.rs` — the renamed
  `world.rs` wall-count test is not counted, since it's a rename, not a
  new test).
- FR-005 (open): acceptance criteria defined when that work starts.

## Verification plan

Unit tests (existing) for physical sanity; `RB-VERIFY-003` divergence
scoring against real replay/BakkesMod ball *and car* trajectories once
`RB-VERIFY-001`/`RB-VERIFY-002` exist — that comparison is what actually
validates (or invalidates) the placeholder constants and this port's
fidelity to Rocket League's real ball/car behavior, not the unit tests
alone. In particular, no real data has yet exercised the box/multi-contact,
ball-vs-car, or car-vs-car collision paths at all — the unit tests confirm
internal physical consistency (a level box stays level, an anisotropic
inertia tensor behaves correctly, a collision conserves momentum), not
fidelity to a real car's actual resting/tumbling/hitting behavior, or to
how many real cars are ever mutually touching at once — this port's
combined solve (`RB-PHYSICS-001-FR-030`) shares its iteration budget
across every dynamic-vs-dynamic manifold touching in a step rather than
solving each pair fully independently, but is itself only proven against
a synthetic symmetric-pinch scenario, not real recorded multi-car
contact data.
`drive::apply_driven_forces`'s constants are even further from validated:
`MAX_CAR_SPEED`, `MAX_BOOST`, `BOOST_ACCELERATION`, and `JUMP_SPEED` are
commonly-cited community numbers, but `THROTTLE_ACCELERATION`,
`BOOST_CONSUMPTION_RATE`, `STEER_TORQUE`, `HANDBRAKE_FRICTION_MULTIPLIER`,
`AIR_CONTROL_TORQUE`, `WALL_JUMP_HORIZONTAL_SPEED`, `DODGE_SPEED`,
`DODGE_ANGULAR_SPEED`, `JUMP_HOLD_MAX_DURATION`,
`JUMP_HOLD_ACCELERATION`, and `LANDING_AUTO_UPRIGHT_TORQUE` are this
project's own simplifications (or, for
`STEER_TORQUE`/`HANDBRAKE_FRICTION_MULTIPLIER`/`AIR_CONTROL_TORQUE`/
`WALL_JUMP_HORIZONTAL_SPEED`/`DODGE_SPEED`/`DODGE_ANGULAR_SPEED`/
`JUMP_HOLD_MAX_DURATION`/`JUMP_HOLD_ACCELERATION`/
`LANDING_AUTO_UPRIGHT_TORQUE`, uncalibrated
placeholders with no public reference at all) — the unit
tests confirm the *shape* of the response (accelerates, caps at max speed,
yaws when moving not when parked, boosts regardless of ground contact,
drains the tank at a constant rate even once the force itself stops
applying, slides more under reduced handbrake friction than under normal
grip, jumps once per fresh press, spins about the correct axis from a
standing start in the air, can spend exactly one extra airborne jump per
airborne period, pushes outward from a touched wall with no such limit,
dodges in the stick's direction with a visible flip when that jump is
spent with pitch or roll held (a wall jump included, when that press's
stick input exceeds the same deadzone), climbs higher the longer the
ground jump button stays held up to a cap, stops a dodge's spin
outright — a wall-jump dodge's included — on a further press before
landing or a wall touch, and gently nudges a tilted airborne car back
toward level when the player isn't otherwise steering it), not that a real
car's throttle/steer/boost/handbrake/jump/double-jump/wall-jump/
wall-jump-dodge/dodge/air-control/hold-height/flip-cancel/landing-assist
response actually matches these curves.
Flip-cancel itself introduces no new constant to calibrate — it's
a state-flag-gated zeroing action, not a magnitude, so it inherits no
validation burden beyond `DODGE_SPEED`/`DODGE_ANGULAR_SPEED`'s own (the
spin it cancels). The double jump reuses `JUMP_SPEED` rather than
introducing a second speed constant, so it inherits that constant's
validation status as-is; the wall jump reuses `JUMP_SPEED` for its
vertical component but introduces `WALL_JUMP_HORIZONTAL_SPEED` for its
horizontal one; the dodge introduces its own `DODGE_SPEED` (horizontal)
and `DODGE_ANGULAR_SPEED` (spin) rather than reusing either — this port
has no public reference for a double-jump-, wall-jump-, or dodge-specific
number to reuse instead of inventing its own — real Rocket League's actual
impulses for these may differ from the ground jump's and from each other,
which this port doesn't model. Variable jump height introduces its own
`JUMP_HOLD_MAX_DURATION` (the hold window's length) and
`JUMP_HOLD_ACCELERATION` (the continuous force applied within it) —
likewise this port's own invention, with no public reference for real
Rocket League's actual hold-window length or acceleration curve.
`AIR_CONTROL_TORQUE` is additionally a
per-axis simplification: real Rocket League's pitch/yaw/roll rates differ
from each other, and this port shares one constant across all three; the
dodge reuses those same axis/sign conventions for its own direction, but
not `AIR_CONTROL_TORQUE`'s magnitude. The wall-jump dodge introduces no new
constant either — it reuses `DODGE_SPEED`/`DODGE_ANGULAR_SPEED`/
`WALL_JUMP_HORIZONTAL_SPEED` outright, so it inherits exactly those
constants' existing (unvalidated) status; its one behavioral choice this
port made up rather than measured — that it consumes
`double_jump_available` while the plain wall jump doesn't — is a structural
simplification, not a magnitude, and is called out in FR-017 and the
`drive` module doc comment. The landing auto-orientation assist introduces
its own `LANDING_AUTO_UPRIGHT_TORQUE` — this port's own invention (chosen
only to read as visibly gentler than `AIR_CONTROL_TORQUE` in tests, one
full order of magnitude smaller), since this port has no public reference
for real Rocket League's actual landing-assist strength or trigger
condition either; unlike every other jump-family constant, this one also
has no ground-proximity signal behind its trigger at all (see FR-018 and
Open questions). The modeled arena footprint's `SIDE_WALL_X`/`BACK_WALL_Y`
are, like `MAX_CAR_SPEED`/`JUMP_SPEED`, commonly-cited community-measured
field dimensions; `CEILING_Z` was one too, until `RB-PHYSICS-001-FR-036`
independently confirmed it (correcting its value from an earlier `2044`
to `2048` in the process) by directly reading RocketSim's and RLUtilities'
own source and by reconstructing it from real extracted collision-mesh
geometry, rather than merely citing another community project's constant
— the same FR-036 pass cross-confirmed `SIDE_WALL_X`/`BACK_WALL_Y` too.
`CORNER_LENGTH` (the octagon corner-cut inset) is, since
`RB-PHYSICS-001-FR-036`, confirmed exact against real extracted
collision-mesh data — this project's earlier claim that it was an
uncalibrated invention with no public reference at all was itself
incorrect (see FR-036's own entry). The flat corner-wall plane
`CORNER_LENGTH` positions is exact; what remains unmodeled is only the
real field mesh's own curved blend from that flat plane into the
floor/ceiling ramps this port doesn't model either, a genuinely different
(curved) collision geometry question, not a "better number" one (see
Open questions). FR-020's `arena::FILLET_RADIUS` does NOT share
`CORNER_LENGTH`'s newly-confirmed status — it remains this port's own
invention, no public reference, and only governed the ball through FR-026
(see FR-020's own Non-goals). Since
FR-027, `FILLET_RADIUS` also governs a car's own corner-testing
approximation — no real recorded car-vs-curve data has exercised either
the ball's or the car's response to it, so the constant's validation
status is unchanged by FR-027, only its reach. FR-022's,
FR-024's, and FR-026's own fillets (the 8 vertical corner-wall edges, the 6
goal-cutout edges, and the 4 goal post-crossbar compound corners) reuse
this same `FILLET_RADIUS` constant rather than
introducing a separate one each — a documented simplification, since this
port has no reason to believe a vertical-edge fillet's own radius, or a
goal post's, should match a cardinal wall's floor/ceiling-seam radius —
FR-022's own edges are visibly shallower (45 degrees) than a floor/ceiling
seam's (90), yet shared the same radius regardless even before FR-025.
FR-021's and FR-023's own fillets (the 4 corner walls' floor/ceiling seams
and all 16 compound corners) instead reuse `CORNER_ARCH_RADIUS` as of
FR-025, not `FILLET_RADIUS` — see FR-025's own entry above and Open
questions for why. `CORNER_ARCH_RADIUS` has exactly the same unvalidated
status as `FILLET_RADIUS`/`CORNER_LENGTH`: this port's own invention, no
public reference, chosen only to read as visibly larger than
`FILLET_RADIUS` in tests (enforced at compile time, not calibrated), and
governs both the ball and, since FR-027, a car's own corner-testing
approximation too, same as every other fillet radius in this crate —
unvalidated for either.
FR-026's own 4 goal post-crossbar compound-corner fillets reuse
`FILLET_RADIUS` unchanged rather than `CORNER_ARCH_RADIUS` — unlike the
arena's own compound corners (FR-023/FR-025), both edge fillets meeting at
a goal's post-crossbar vertex already share `FILLET_RADIUS`, so there is no
mismatched-radius concern here requiring a dedicated, larger arch radius.
`StaticCornerFillet::between_three_planes`'s own general
three-plane-intersection center solve and spherical-triangle `bounds`
derivation were already independently verified against a synthetic
fixture by FR-023 — FR-026 doesn't re-derive that machinery, only applies
it to a new triple of real planes (a back wall, a post, and the crossbar),
proven here by
`arena.rs`'s `every_goal_corner_fillets_center_sits_radius_in_from_a_back_wall_a_post_and_the_crossbar`
(the same "prove the real
triple intersection, not an arbitrary point" discipline FR-023's own
arena-corner test used), plus a new end-to-end `world.rs` test giving the
same live-physics "pushed meaningfully back toward the center" proof every
other fillet in this port gets.
The unit
tests confirm the fillet's *shape* of response (pushes back toward the
axis once the sphere's surface crosses the fillet's own radius from
inside, respects its own sector — 90 degrees for a floor/ceiling seam, 45
for a corner wall's own vertical edge — and, since FR-027, deflects a box
the same way via its own 8-corner testing, confirmed mathematically exact
for this containment question by `RB-PHYSICS-001-FR-032`), not that a
real ball's or car's actual wall-to-floor/wall-to-ceiling or wall-to-wall
transition behavior matches this radius or this trigger condition, at a cardinal wall, a
corner wall's floor/ceiling seam, or a corner wall's own vertical edge
alike. `StaticQuarterPipe::between_planes`'s own generalization (FR-022) —
solving the axis point as a real 2x2 linear system, and testing sector
membership via signed cross products rather than the old
perpendicular-only two-dot-products shortcut — is itself unit-tested
directly against a synthetic non-perpendicular fixture, independent of the
arena's own geometry, so its correctness doesn't rest solely on the
corner-wall numbers happening to work out. `StaticCornerFillet::between_three_planes`'s three-plane-intersection center solve and its
spherical-triangle `bounds` derivation (FR-023) are likewise independently
unit-tested against a synthetic fixture (a perpendicular floor combined
with the same 45-degree non-perpendicular wall pair `between_planes`'s own
fixture uses), not just proven out against the arena's own 4 corner-wall
quadrants. `arena::GOAL_HALF_WIDTH`/`GOAL_HEIGHT` (FR-024) are
commonly-cited community numbers, not independently confirmed by this
project — unlike `SIDE_WALL_X`/`BACK_WALL_Y`/`CEILING_Z`, which
`RB-PHYSICS-001-FR-036` did independently confirm — and only govern the
ball, same as `FILLET_RADIUS` (a car can't drive into a goal in this port
at all — see FR-024's Non-goals).
`StaticGoalWall::contains_in_window`'s own containment logic is
independently unit-tested against a synthetic fixture (not the arena's
own goal dimensions), the same "prove the general mechanism, not just
that the specific arena numbers happen to work out" discipline every
other new shape's own constructor/containment logic in this port already
gets.
`collision::box_vs_quarter_pipe`/`box_vs_corner_fillet` (FR-027) are
independently unit-tested against a synthetic fixture, not just proven
out against the standard arena's own fillets: a box centered directly on a
quarter-pipe's own surface (along its sector bisector) reports at least
one contact with every normal pointing back toward the axis, a box placed
deep on the opposite side of the fillet's own angular sector (not merely
far away radially — a point far along the sector's own bisector direction
is still angularly "inside" the wedge regardless of distance) reports no
contact, and the same two proofs hold for a `StaticCornerFillet`. The unit
tests confirm the corner-testing's own *shape* of response
(a box embedded in a fillet's footprint gets at least one real contact, a
box outside the fillet's own bounds gets none). This port still has no
GJK/EPA support-mapping machinery, but `RB-PHYSICS-001-FR-032` subsequently
proved (both analytically and via a dedicated dense-sampling test,
`no_point_on_a_boxs_face_is_ever_farther_from_a_quarter_pipes_axis_than_its_own_corners`)
that none is needed here: the once-suspected "face resting flush against a
shallow curve, with every corner just clear of the fillet while the face's
middle already overlaps it" case cannot actually occur, since this
contact's containment question is a convex-maximum one always attained at
a corner. The two live end-to-end `world.rs`
proofs take deliberately different shapes for the same reason a sphere and
a box need different invariants:
`a_car_resting_within_a_curved_transitions_footprint_is_pushed_up_off_the_flat_floor_height`
mirrors the ball's own equivalent `StaticQuarterPipe` proof directly (a
car resting at ordinary flat-floor height, already overlapping the curve's
own material, gets pushed up off that height), since a resting height is
just as well-defined for a box as for a sphere; but
`a_car_embedded_in_a_compound_corner_fillets_footprint_has_its_penetration_reduced`
deliberately checks that the *worst (maximum) corner penetration* this
fillet reports shrinks meaningfully after the solver runs, not that the
box's own center of mass moves closer to the fillet's center the way the
equivalent ball test checks distance-to-center — a real, documented
test-design decision, not just an implementation detail: an oriented box
has multiple corners at different depths simultaneously, so resolving one
corner's contact can rotate the box in a way that moves its center of mass
*away* from the fillet's center even as every individual corner's own
overlap shrinks, making "distance from center-of-mass to fillet center"
the wrong invariant for a box the way it's the right one for a sphere (a
single point). This was found empirically while writing the test — an
earlier version asserting center-of-mass distance shrank actually failed,
with the car ending up farther from the fillet's center even though the
fillet was correctly resolving every corner's penetration — which is what
led to writing the assertion the more careful, per-corner way instead.

`collision::box_vs_goal_wall` (FR-028) is independently unit-tested
against the same synthetic `goal_wall()` fixture
`sphere_vs_goal_wall`'s own tests already use, not the standard arena's
real goal dimensions: a car with every corner inside the window gets no
contact at all, a car straddling the window's own edge gets contacts on
exactly the corners still outside it (the box equivalent of a sphere's
single all-or-nothing center-point test, but resolved per-corner
instead), and a car entirely outside the window collides bit-for-bit
identically to plain `contacts_vs_plane` against the same wrapped plane.
The unit tests confirm the corner-testing's own *shape* of
response for a goal wall (a car with every corner inside the window
passes clean through, a car straddling the edge gets a real partial
block, a car outside the window is unaffected), not that it matches a
real car's actual goal-line-crossing behavior; whether a car's face
resting flush against the window's own edge, with every corner still just
clear of it while the face's middle already overlaps it, can actually
occur is a distinct question from the curved-fillet containment question
`RB-PHYSICS-001-FR-032` investigated and resolved (the window's boundary
is a flat rectangle, not a curve) — this concern remains open and
unverified for the goal wall specifically, not covered by FR-032's
finding. The live end-to-end `world.rs` proof,
`a_car_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall`,
mirrors the pre-existing ball test's own live-physics proof directly (a
car fired at the same goal-mouth-center position/velocity ends up past
`BACK_WALL_Y` instead of being stopped), reusing that test's already-solved
1.8s flight-duration bound rather than re-deriving it, since the
underlying `StaticQuarterPipe`-infinite-axis timing concern (see FR-025's
own entry) is unrelated to FR-028 itself; a second end-to-end test,
`a_car_aimed_away_from_the_goal_mouth_is_still_stopped_by_the_back_wall`,
confirms a car aimed at the wall's still-solid portion (outside
`GOAL_HALF_WIDTH`) is unaffected by this requirement.

`body::StaticBoundedWall::contains_in_bound` (FR-029) is independently
unit-tested against a synthetic fixture, the same discipline every other
new shape's own containment logic in this port gets: true at the bound's
own center and just inside each of its four edges, false just outside
them, and unaffected by a point's distance from the plane — mirroring
`StaticGoalWall::contains_in_window`'s own tests exactly, with the
boolean gate's meaning inverted (inside the bound instead of outside the
window). `collision::sphere_vs_bounded_wall`/`box_vs_bounded_wall` are
likewise independently unit-tested against a synthetic fixture, not just
proven out against the standard arena's own goal geometry: a sphere
inside the bound behaves exactly like an ordinary plane contact, a sphere
outside the bound has no contact at all, a box squarely inside the bound
behaves like an ordinary plane contact, a box straddling the bound's own
edge collides only on the corners still inside it (the bounded-wall
mirror of `box_vs_goal_wall`'s own straddling-the-window test, with the
gate inverted), and a box entirely outside the bound has no contact. The
unit tests confirm the bounded-wall's own *shape* of response (collides
only within its own rectangular footprint, an ordinary plane contact
everywhere inside it), not that a real ball or car's actual behavior
against a real net's back/side/roof structure matches this exactly —
`arena::GOAL_DEPTH` in particular has no public reference at all (see
Non-goals and Open questions).

Two real test-design findings came out of writing this requirement's own
3 live end-to-end `world.rs` proofs
(`a_ball_shot_into_the_goal_is_stopped_by_the_goal_back_wall`,
`a_ball_shot_sideways_inside_the_goal_is_stopped_by_a_goal_side_wall`,
`a_ball_shot_upward_inside_the_goal_is_stopped_by_the_goal_roof`), both
worth keeping here rather than only in code comments, matching this
spec's established practice of recording "why the test looks like this"
stories (see FR-025's and FR-027's own entries above). First: an early
version of these tests built the scene from the full
`PhysicsWorld::standard_arena` (the way every other end-to-end goal test
in this file does), but a ball fired sideways or upward from deep inside
the goal box got flung to bizarre, wildly wrong positions (e.g. ending up
at x=-687 after being fired only in +x). Root cause: the standard arena's
own goal-cutout post/crossbar fillets (`arena::standard_goal_cutout_fillets`,
FR-024) sit right at the window's edge, close enough to a point deep
inside the goal box to spuriously trigger the pre-existing, already-documented `StaticQuarterPipe` limitation that a fillet's sector-membership
test only checks angular position around its own axis, not radial
distance — the same category of issue the FR-025 test-writing notes above
already describe for a different corner-wall fillet. The fix: isolate
each of these 3 tests to a minimal `PhysicsWorld` built from just the
specific new wall(s) under test (`PhysicsWorld::new` plus
`with_wall`/`with_bounded_wall`, not `PhysicsWorld::standard_arena`),
sidestepping the unrelated fillet interaction entirely — the correct fix,
not a bug in `StaticBoundedWall` or `goal_back_wall_plane` themselves.
Second: an earlier version of these same 3 tests set only the ball's own
`restitution = 0.0` and got nondeterministic results for the roof test
specifically (the ball ending up *below* its own starting height). Root
cause: the wall's own default `StaticPlane::new` restitution (0.5) still
applies in the solver's contact resolution regardless of the ball's own
value, so the ball bounced back down with enough remaining simulation
time left to travel well past its start. Fixed by also explicitly
zeroing the specific wall(s)' own `plane.restitution` in each of these 3
tests before adding them to the scene, so the ball damps out
deterministically instead of bouncing indefinitely.

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- Both genuine ambiguities `RB-PHYSICS-001-FR-031`'s constant-calibration
  audit surfaced but deliberately didn't act on for lack of confidence are
  now resolved by `RB-PHYSICS-001-FR-036`'s own dedicated follow-up (real
  source-level research, not a guess) — see its own Requirements entry for
  the full reasoning: (1) the ball radius question was itself reframed, not
  simply "92.75 is wrong, use 91.25" — real Rocket League splits an inertia
  radius (`91.25`) from a separately larger collision radius (`93.15`, the
  mesh's own collision margin), and since this port has no Bullet-style
  collision margin of its own, its single unified radius field's correct
  analog is the collision radius, so `92.75` became `93.15`, not `91.25`;
  (2) `arena::CEILING_Z` (`2044.0`) is now confirmed to share the same
  reference point as RocketSim's `ARENA_HEIGHT = 2048.f`, corrected to
  `2048.0`. Neither used real `RB-VERIFY-002` data (still blocked); both
  instead used direct RocketSim/RLUtilities source-code reading plus an
  independent real-mesh geometric reconstruction, which this project
  judged sufficient confidence to act on without waiting further.
- A combined multi-body solve for bodies simultaneously touching more than
  one other body is now implemented (see `RB-PHYSICS-001-FR-030`):
  `solver::resolve_dynamic_manifolds` shares one interleaved
  `SOLVER_ITERATIONS`-iteration budget across every dynamic-vs-dynamic
  manifold touching in a step, instead of resolving each pair with its
  own fully independent solve. What's still genuinely open: at this
  crate's existing `SOLVER_ITERATIONS = 10`, an extreme mass-ratio
  "sandwiched" configuration (measured directly by FR-030's own tests)
  still doesn't fully converge to the true simultaneous-solve answer,
  only measurably closer to it than the old independent-pairwise
  approach — real recorded multi-car contact data would be needed to
  know whether that residual error actually matters for fidelity in
  practice, or whether raising `SOLVER_ITERATIONS` (confirmed manually to
  converge much closer at 300 iterations, at obvious extra per-step cost)
  is worth it before such data exists; not started.
- Replicating real Rocket League's actual landing-assist trigger condition
  (proximity to the ground, via some raycast or distance query this port
  doesn't have) instead of the current continuous-whenever-airborne
  stand-in (see FR-018) — needs a concrete reason (e.g. real recorded
  landing-assist behavior to compare against) before it's worth adding a
  ground-distance query solely for this. (The double jump's own dodge — a
  directional flip off the ground/air, no wall involved — is now
  implemented as FR-014; variable jump height for the ground jump is now
  implemented as FR-015; canceling a dodge's rotation early — flip-cancel —
  is now implemented as FR-016; a dodge variant of the wall jump is now
  implemented as FR-017; a gentle landing auto-orientation assist is now
  implemented as FR-018.)
- ~~A car (box) actually being deflected by a curved fillet through genuine
  convex-vs-curved-surface narrow-phase collision machinery~~ —
  investigated and resolved, see `RB-PHYSICS-001-FR-032`: the specific
  concern motivating this (a face resting flush against a shallow curve
  under-detecting because none of its own corners individually register)
  is mathematically impossible for this contact's actual question
  (distance-from-an-axis/point is convex, so its maximum over a box is
  always at a corner — see FR-032's own entry for the full argument and
  the empirical test proving it). `box_vs_quarter_pipe`/
  `box_vs_corner_fillet`'s per-corner technique is exact for detecting
  whether *any* part of the box violates the fillet's radius, not an
  approximation of it; a genuine GJK/EPA convex-vs-convex narrow phase was
  actually built during this investigation and found to *regress* two
  real end-to-end tests, because it answered a different (nearest-point)
  question than the one this contact needs (farthest-point/containment).
  (Deflecting a car at
  all — previously this bullet's entire subject, when a car drove straight
  through every curve's or corner fillet's footprint completely unaffected
  — is now implemented via corner-testing, see FR-027.
  The compound corner where a vertical-edge fillet meets a
  floor-seam or ceiling-seam fillet, near a corner wall's own top/bottom
  endpoint, is now modeled with its own `StaticCornerFillet` sphere, as
  FR-023 — though it, the edge fillet, and the seam fillet it meets remain
  independent, additive contact sources, per `RB-PHYSICS-001`'s "single
  flat plane, single-radius edge fillet, or single-radius corner fillet
  per boundary segment" Non-goal — not a single continuously-blended
  surface across the whole octagon. The two compound corners per goal where
  a post's own edge fillet meets the crossbar's are likewise now modeled
  with a `StaticCornerFillet`, as FR-026 — built directly via
  `StaticCornerFillet::between_three_planes` on the back-wall/post/crossbar
  planes rather than derived from the two edge fillets meeting there,
  reusing `FILLET_RADIUS` unchanged since both edge fillets meeting at that
  vertex already share it, unlike the arena's own compound corners, which
  needed the distinctly larger `CORNER_ARCH_RADIUS` (see FR-025) — with the
  same independent-additive-fillets caveat as above, not a single
  continuously-blended surface. The goal's other two corners, where a post
  meets the floor, still need no such treatment: the window's own bottom
  edge sits exactly at floor level, so a post's own fillet there simply
  ends flush with the ground rather than leaving a sharp vertex to round
  off.)
- A car actually driving into a goal (now implemented, see
  `RB-PHYSICS-001-FR-028`) and a modeled bounded interior volume behind
  the goal window (now implemented too, see `RB-PHYSICS-001-FR-029`) — the
  goal-mouth window now opens onto a solid bounding box (2 back-of-net
  planes, 4 side walls, 2 roofs) a ball or car settles against instead of
  the open, unbounded space it opened onto through FR-028.
  `collision::box_vs_goal_wall`'s own per-corner approximation carries
  the same "exact per test point, an approximation of the whole shape"
  caveat the question above's fillet corner-testing already has — it
  isn't necessarily precise enough for a car actually clearing a
  goal-cutout edge fillet's own boundary right at the window's rim, only
  the flat window itself, which is what FR-028 actually tests; the same
  is true of FR-029's `box_vs_bounded_wall`, for its own bounded walls.
  ~~Still genuinely open: a **net mesh** — cloth/soft-body simulation,
  visual net sag, or a "ball tangles in netting" behavior — which FR-029
  deliberately didn't attempt; its interior is a solid bounding box, not
  a springy/catching net.~~ Implemented for the ball, see
  `RB-PHYSICS-001-FR-033`'s `net::NetMesh` (a real mass-spring grid, since
  the concrete motivation this bullet asked for turned out not to be
  needed to justify it — a "ball tangles in netting" behavior is itself
  the kind of qualitative, visually-checkable fidelity gap worth closing
  independent of a specific divergence-scoring signal). Still open: a
  car's own contact against a net (scoped out of FR-033, see its own
  Non-goals), and a full 3D "sock" shape/visual net sag/bending stiffness
  beyond FR-033's flat structural-plus-shear-spring panel.
- ~~Disambiguating or blending a car's simultaneous contact with two walls
  at a corner for wall-jump purposes (see FR-019's Non-goals) — physical
  collision resolution already handles this correctly regardless; only
  the wall-jump push-off direction picker (`PhysicsWorld::step`'s
  "first wall in `self.walls`" rule) isn't. FR-019's corner walls make this
  case reachable in the standard arena for the first time; still not
  exercised by any test here. Not started.~~ Implemented, see
  `RB-PHYSICS-001-FR-039`: the picker now sums every touched wall's normal
  and normalizes the result instead of picking whichever wall comes first.
- Sourcing or verifying `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS`
  against real field mesh data (see
  FR-020/FR-021/FR-022/FR-023/FR-024/FR-025/FR-026/FR-027) — this port has
  no reference for either at all. `arena::CORNER_LENGTH` no longer belongs
  in this bullet: `RB-PHYSICS-001-FR-036` confirmed it exact against real
  extracted collision-mesh data, the same sourcing status as `SIDE_WALL_X`/
  `BACK_WALL_Y`/`CEILING_Z` (see FR-036's own entry and FR-019's). Even a
  sourced value for `FILLET_RADIUS`/`CORNER_ARCH_RADIUS` would only
  approximate the real corner/transition, which isn't a single flat plane,
  single-radius edge fillet, or single-radius corner fillet in the actual
  game. `FILLET_RADIUS` governs the 4 cardinal walls'
  floor/ceiling-seam fillets (FR-020), all 8 vertical-edge fillets
  (FR-022), all 6 goal-cutout-edge fillets (FR-024), and all 4 goal
  post-crossbar compound-corner fillets (FR-026); since FR-025, the
  4 corner walls' own floor/ceiling-seam fillets (FR-021) and all 16
  compound-corner fillets (FR-023) instead reuse the distinctly larger
  `CORNER_ARCH_RADIUS`, chosen only to read as visibly bigger than
  `FILLET_RADIUS` (enforced at compile time), not measured against real
  field mesh data either — whether the real game even uses two uniform
  transition radii at all (as opposed to a genuinely different
  corner-specific curve, a third, differently-shaped curve at the
  vertical edges, a fourth at the compound corners, and a fifth at the
  goal posts/crossbar — real Rocket League's actual goal-post radius is
  visually quite different from a wall-to-floor transition's) is itself
  unconfirmed. `arena::GOAL_HALF_WIDTH`/`GOAL_HEIGHT` (FR-024) likewise
  have no independently-confirmed source, though they're commonly-cited
  community numbers like `SIDE_WALL_X`, not this port's own inventions
  like `CORNER_LENGTH`. `arena::GOAL_DEPTH` (FR-029) is a further step
  removed even from `GOAL_HALF_WIDTH`/`GOAL_HEIGHT`: this port has no
  commonly-cited community reference for the real net's depth at all, so
  it's this project's own uncalibrated invention, same status as
  `CORNER_LENGTH`, chosen only to be a visibly real interior volume
  comparable in scale to the goal mouth's own dimensions.
- Calibrating `drive`'s constants (`THROTTLE_ACCELERATION`, `STEER_TORQUE`,
  `BOOST_CONSUMPTION_RATE`, `HANDBRAKE_FRICTION_MULTIPLIER`,
  `AIR_CONTROL_TORQUE`, `WALL_JUMP_HORIZONTAL_SPEED`, `DODGE_DEADZONE`,
  `DODGE_SPEED`, `DODGE_ANGULAR_SPEED`, `JUMP_HOLD_MAX_DURATION`,
  `JUMP_HOLD_ACCELERATION`, `LANDING_AUTO_UPRIGHT_TORQUE`, and re-checking
  `MAX_CAR_SPEED`/`MAX_BOOST`/`BOOST_ACCELERATION`/`JUMP_SPEED`) against
  real recorded driving data — needs `RB-VERIFY-002` capture data; not
  started. `STEER_TORQUE`, `HANDBRAKE_FRICTION_MULTIPLIER`,
  `AIR_CONTROL_TORQUE`, `WALL_JUMP_HORIZONTAL_SPEED`, `DODGE_DEADZONE`,
  `DODGE_SPEED`, `DODGE_ANGULAR_SPEED`, `JUMP_HOLD_MAX_DURATION`,
  `JUMP_HOLD_ACCELERATION`, and `LANDING_AUTO_UPRIGHT_TORQUE` in particular
  have no public reference at all
  (unlike gravity, max speed, the boost constants, or `JUMP_SPEED`), so any
  of them may be off by a large factor, not just imprecise.
- Splitting `AIR_CONTROL_TORQUE` into distinct per-axis constants (pitch,
  yaw, roll) once real recorded air-control data exists to calibrate them
  separately — real Rocket League's three rates genuinely differ (roll
  fastest); sharing one constant is a documented simplification, not a
  claim they're actually equal.
- Handbrake's real mechanic (reduced rear-wheel grip enabling a
  steering-assisted drift) doesn't map cleanly onto this port's one-box,
  uniform-friction car model (see Non-goals) — worth revisiting whether a
  front/rear friction split, or a genuine slip-angle-driven lateral force,
  is warranted once real recorded drift behavior exists to compare
  against; the current uniform temporary friction reduction is a
  deliberately simple stand-in, not a claim of mechanistic fidelity.
- Real Rocket League doesn't share one speed ceiling between throttle and
  boost (a boosting car can exceed unboosted top speed); this port reuses
  `MAX_CAR_SPEED` as boost's cap too, a documented simplification — worth
  splitting into a separate boost speed cap once real recorded top-speed
  data exists to calibrate one.
- FR-005 above.
- Restitution/friction combine mode (`rb_physics_bullet::solver` currently
  averages; Bullet's actual default is `max` for both) — revisit once real
  data exists to calibrate against.
- Sleeping is no longer an open item — `RB-PHYSICS-001-FR-037` implemented
  it, and with it the actual fix for the *bouncy* (restitution > 0) resting
  contact that used to never settle (`RB-PHYSICS-001-FR-034`'s split
  impulse and `RB-PHYSICS-001-FR-035`'s warm-starting closed adjacent gaps
  but couldn't substitute for it, since restitution re-triggers off a fresh
  gravity-induced closing velocity every frame regardless of where the
  solver's iteration starts or how it got there). Warm-starting itself
  remains scoped to `resolve_dynamic_manifolds` only (static contacts and
  `resolve_contacts`/`resolve_contacts_between` stay un-warm-started, a
  deliberate scoping choice — see FR-035's own Non-goals); that's a
  genuinely separate, still-open item, tracked in this spec's own Non-goals
  rather than here now that it no longer shares a bullet with sleeping.
  `LINEAR_SLEEP_VELOCITY_THRESHOLD`/`ANGULAR_SLEEP_VELOCITY_THRESHOLD`/
  `SLEEP_TIME_THRESHOLD` (`body.rs`) are FR-037's own uncalibrated
  placeholders, worth revisiting once real recorded ball/car-hit behavior
  exists to compare against — see FR-037's own entry.
- `box_vs_box`'s edge-edge contact point uses the midpoint of the two
  closest points on the involved edges, and its face-contact clipping
  falls back to a single clamped-center point if clipping ever yields zero
  points (a defensive branch not exercised by real recorded data yet) —
  both are reasonable, tested choices, but neither has been validated
  against Bullet's own `dBoxBox` output or real car-vs-car contact
  behavior.

## Change history

- 0.38.0 (2026-08-31): FR-039 added and implemented (wall-jump corner
  disambiguation) — closes the "first wall in `self.walls`" simplification
  FR-013 originally documented and FR-019's new diagonal corner walls made
  reachable in the standard arena for the first time.
  `PhysicsWorld::step`'s per-car wall-normal computation now collects every
  wall a car is touching this step, sums their normals, and normalizes the
  result, instead of `Iterator::find`-ing the first match — a car touching
  exactly one wall gets that wall's own normal back unchanged (summing a
  single unit vector and normalizing it is a no-op), so the far more common
  single-wall case is bit-for-bit unaffected; a car touching two walls at
  once now pushes off diagonally along the sum of both, instead of firing
  along only one of them depending on iteration order. No new collision
  code was needed — `resolve_plane_contact` already resolved simultaneous
  multi-wall contact correctly; only the wall-jump push-off direction
  picker, `drive::apply_driven_forces`'s own input, was affected. 1 new
  `world.rs` test,
  `a_car_touching_two_walls_at_a_corner_wall_jumps_diagonally_outward`
  (two perpendicular walls, a car touching both at once, asserting both
  horizontal velocity components come out positive and roughly equal after
  a wall jump), bringing the crate to 268 total (+1 over FR-037's 267).
- 0.37.0 (2026-08-31): FR-037 added and implemented (sleeping) — closes
  the "no sleeping" half of `solver`'s own documented gap FR-035 left open,
  and with it the actual fix for a *bouncy* resting contact never
  settling, since restitution re-triggers off a fresh gravity-induced
  closing velocity every frame regardless of where the solver's iteration
  starts, so nothing about warm-starting or split impulse could ever stop
  the residual bounce. New `body::RigidBody` fields `is_sleeping: bool`
  (public) and `sleep_timer: f32` (private), plus
  `update_sleep_state(&mut self, dt: f32)` (accumulates `sleep_timer` while
  both `linear_velocity.length()`/`angular_velocity.length()` stay under
  new `LINEAR_SLEEP_VELOCITY_THRESHOLD`/`ANGULAR_SLEEP_VELOCITY_THRESHOLD`
  constants, setting `is_sleeping` and forcibly zeroing both velocities
  once `sleep_timer` reaches `SLEEP_TIME_THRESHOLD`, repeated every
  subsequent call while still under threshold; crossing either threshold
  resets the timer and clears `is_sleeping` immediately) and `wake(&mut
  self)` (the same reset, unconditionally, independent of velocity).
  `PhysicsWorld::step` calls `update_sleep_state` for the ball and every
  car right after every other contact this step resolves but before the
  transform integrates. `drive::apply_driven_forces` calls `car.wake()`
  unconditionally, before anything else in that call runs, whenever a new
  `input_is_active` helper finds the car's `ControllerInput` genuinely
  active — necessary because a resultant-velocity-only wake check isn't
  enough for a driven body: a car accelerating from rest under a small
  per-frame driving force whose own one-frame velocity delta is itself
  smaller than the sleep threshold would otherwise have that delta zeroed
  right back out every frame, permanently stranding it.
  `input_is_active` treats an unrecovered analog channel (`None`) the same
  as a recovered-but-literally-neutral one (`Some(0.0)`), rather than the
  simpler `!= ControllerInput::default()`, which would keep a car fed a
  real recorded input stream that always resolves every channel from ever
  sleeping at all. All three new threshold constants are this project's
  own uncalibrated placeholders — no public reference exists for what, if
  any, real Rocket League's own physics engine uses internally for this
  purely implementation-internal stabilization detail. 8 new tests (5 in
  `body.rs` exercising `update_sleep_state`/`wake` directly, 3 in
  `world.rs` proving the fix through a live `PhysicsWorld` — a
  nonzero-restitution ball/ground pair actually falling asleep at exactly
  zero velocity, a car seeded already asleep waking the instant throttle
  is applied, and a sleeping ball waking when a moving car hits it),
  bringing the crate to 267 total (+8 over FR-036's 259). All pre-existing
  tests pass unchanged, including `resting_ball_stays_at_rest` (whose own
  comment, describing the bouncy-resting-contact limitation this
  requirement fixes, was corrected to point at the new test that now
  demonstrates the fix instead of only documenting the gap) and
  `dropped_ball_eventually_settles_on_the_ground` (already used a bouncy
  restitution of 0.3, unaffected since it only ever checked landing height,
  not whether the ball kept bouncing forever).
- 0.36.0 (2026-08-31): FR-036 added and implemented (ball radius /
  `CEILING_Z` constant-ambiguity resolution) — a dedicated follow-up to
  FR-031's own audit, using real source-level research (RocketSim's and
  RLUtilities' own source, and the current RLBot wiki, read directly rather
  than guessed at) instead of leaving both ambiguities open indefinitely.
  Ball radius: FR-031 had framed this as "`92.75` vs. `91.25`", but the
  real games actually split the ball into a smaller inertia radius
  (`91.25`) and a distinctly larger collision radius (`93.15`, the mesh's
  own collision margin) — a split this port's single unified radius field
  can't represent, and since this port has no separate Bullet-style
  collision margin, the collision radius is the correct single-constant
  analog, not the inertia radius. Every `92.75` literal across
  `solver.rs`/`world.rs`/`net.rs`/`collision.rs` became `93.15`, not
  `91.25`. `arena::CEILING_Z`: confirmed, via both RocketSim's
  `ARENA_HEIGHT = 2048.f` and an independent reconstruction from real
  extracted collision-mesh geometry, to share the same reference point,
  so `2044.0` became `2048.0`. Two mis-documented claims were also
  corrected as a low-risk byproduct: `arena::CORNER_LENGTH` and
  `arena::GOAL_DEPTH` were wrongly described (by FR-019/FR-031 and FR-029
  respectively) as uncalibrated placeholders with no public reference —
  both are actually confirmed exact, so only their doc comments changed,
  not their values. `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS` remain
  untouched and still genuinely uncalibrated — no analytic reference exists
  for either, a separate, more involved mesh-ingestion follow-up
  deliberately left for later. Before the mechanical substitution, a
  targeted `grep -c "892\.75"` across the four non-`arena.rs` files
  confirmed zero matches in each, ruling out corruption of
  `arena::GOAL_HALF_WIDTH`'s unrelated `892.755` literal (edited by hand in
  `arena.rs` instead, which was excluded from the substitution). No new
  tests: a constant-only correction with no new behavior to characterize,
  the same precedent FR-031 established; `cargo test --workspace` re-run
  clean at 259 total (unchanged from FR-035), confirming the change is
  behavior-preserving everywhere the old values were exercised.
- 0.35.0 (2026-08-31): FR-035 added and implemented (warm-starting,
  `resolve_dynamic_manifolds` only) — a new `solver::ContactCache` carries
  a manifold's converged real-channel impulses (normal plus both friction
  rows) from one call to the next, matched by each contact's approximate
  world position (`CONTACT_MATCH_DISTANCE`, an uncalibrated placeholder).
  A new `warm_start_two_body_row` applies each row's cached impulse
  directly to the manifold's shared `DeltaVelocity` accumulators *before*
  iterating — critically not just setting `TwoBodyRow::applied_impulse`,
  which (with `GLOBAL_CFM` always `0.0`) would leave the first iteration's
  correction identical to a cold start; the seed has to be baked into the
  starting delta itself, mirroring Bullet's own warm-start applying the
  cached impulse to the solver body's temporary velocity at setup, before
  any iteration runs. `resolve_dynamic_manifolds` gained a new `caches: &mut
  HashMap<(usize, usize), ContactCache>` parameter (key normalized so
  either argument order finds the same entry); every call rebuilds
  `caches` from only that call's manifolds, so a pair no longer touching
  is dropped automatically, no separate eviction pass needed.
  `PhysicsWorld` gains one `dynamic_manifold_caches` field, persisted
  across steps. Deliberately scoped to this one call site:
  `resolve_contacts`/`resolve_contacts_between` (every static-geometry
  contact, for both the ball and every car) stay un-warm-started, since
  this port's fixed `SOLVER_ITERATIONS` already fully converges every
  one-body/two-body scenario this crate tests — warm-starting has no
  scenario to demonstrate value against there yet, while
  `resolve_dynamic_manifolds` already had one:
  `RB-PHYSICS-001-FR-030`'s own documented extreme-mass-ratio "sandwiched"
  case, which doesn't fully converge within one call's iteration budget. 1
  new `solver.rs` test,
  `warm_starting_a_sandwiched_ball_across_two_calls_converges_closer_than_a_repeated_cold_start`,
  reuses that exact scenario: call 1 (cold) partially converges and
  populates a cache; from that identical post-call-1 state, call 2 runs
  twice on independent copies — once warm (call 1's cache), once cold (a
  fresh map) — with identical positions, contacts, velocities, and
  iteration budget both times, isolating exactly what the warm seed
  contributes; the warm run lands measurably closer to the true
  zero-velocity equilibrium. All 14 of `solver.rs`'s pre-existing tests
  pass unchanged when given an empty cache, confirming this requirement is
  behavior-preserving for every case they already covered. This
  requirement does NOT fix this spec's own documented "bouncy resting
  contact never settles" limitation — that symptom comes from restitution
  re-triggering off a fresh gravity-induced closing velocity every frame,
  independent of where the solver's iteration starts; warm-starting
  converges the same wrong-looking bounce faster, it doesn't stop it from
  recurring. Sleeping (still unimplemented) is the actual fix for that,
  and remains the sole open item under the old combined
  "no-warm-starting-or-sleeping" bullet this requirement splits. Bringing
  the crate to 259 tests total (+1 over FR-034's 258).
- 0.34.0 (2026-08-31): FR-034 added and implemented (split impulse) —
  closes the "no split impulse" half of this spec's own documented
  solver-simplification gap (see Non-goals/Open questions), leaving only
  warm-starting/sleeping open. `ConstraintRow`/`TwoBodyRow` (`solver.rs`)
  each gained `rhs_penetration`/`applied_push_impulse` fields, splitting
  the normal row's combined `rhs = (positional_error + velocity_error) *
  jac_diag_ab_inv` into two independent terms (a friction row's
  `rhs_penetration` is always `0.0`). Two new resolve functions,
  `resolve_push_row`/`resolve_two_body_push_row`, run the same
  projected-Gauss-Seidel iteration as `resolve_row`/`resolve_two_body_row`
  but against `rhs_penetration`/`applied_push_impulse` and a separate
  `push_delta`/`push_delta_a`/`push_delta_b` accumulator, always clamped
  to `[0, UPPER_LIMIT]`. A new `apply_push_delta` applies that accumulated
  push delta directly to a body's position/orientation via the existing
  `integrate::integrate_transform` — mirroring Bullet's own
  `btSolverBody::writebackVelocity`'s second, independent
  `integrateTransform` call. `resolve_contacts`, `resolve_contacts_between`,
  and `resolve_dynamic_manifolds` (the last via a new per-body-index
  `push_deltas` vector, reusing `delta_pair_mut`) each
  gained the push-channel resolve/apply calls alongside their pre-existing
  real-channel ones; no other module or call site changed. 2 new
  `solver.rs` tests
  (`split_impulse_corrects_deep_penetration_via_position_not_velocity`/
  `..._between_two_bodies`) directly prove a deeply-penetrating, at-rest
  contact leaves near-zero real velocity while the body/bodies' positions
  measurably separate. This requirement also surfaced (via failing tests,
  not by design) that 4 pre-existing `world.rs` live end-to-end fillet
  tests
  (`a_ball_embedded_in_a_vertical_corner_edges_fillet_footprint_is_pushed_toward_the_axis`,
  `a_ball_embedded_in_a_compound_corner_fillets_footprint_is_pushed_toward_the_center`,
  `a_ball_embedded_in_a_goal_posts_fillet_footprint_is_pushed_toward_the_axis`,
  `a_ball_embedded_in_a_goal_corner_fillets_footprint_is_pushed_toward_the_center`)
  had encoded the *old*, pre-split-impulse behavior in their own
  assertions — each expected the ball to keep coasting past its resting
  distance under residual velocity the old combined `rhs` term left
  behind, and each now instead settles almost exactly at that resting
  distance with no such residual velocity to coast on. Their assertions
  and comments were updated to check settling at (not past) the resting
  distance, a strictly stronger and more physically correct proof than
  the "moved meaningfully" one they replace — a direct sign this
  requirement's fix is real and not just internally self-consistent.
  Bringing the crate to 258 tests total (+2 over FR-033's 256, plus the 4
  fillet-test assertion corrections).
- 0.33.0 (2026-08-31): FR-033 added and implemented (genuine net mesh,
  ball only) — closes the "genuine net mesh" Non-goal `FR-029`'s own doc
  comment left open. New module `net` (`net::NetMesh`): a rectangular
  mass-spring grid of point masses (each a real `RigidBody::sphere`, tiny
  and light), every perimeter point anchored (fixed, representing
  attachment to the rigid goal frame) and every interior point free,
  connected by structural (horizontal/vertical) and shear (diagonal)
  springs (Hooke's law plus velocity damping). `NetMesh::step` sub-steps
  its own internal physics for stability and resolves the ball's contact
  against every free point it overlaps via a new `collision::sphere_vs_sphere`
  (this crate's first real sphere-vs-sphere contact — previously an
  unimplemented, callerless placeholder) plus the *existing*
  `solver::resolve_contacts_between` two-body path — no new solver code,
  reusing the same machinery every other dynamic-vs-dynamic contact in
  this crate already uses. New `arena::standard_nets` builds one
  `net::NetMesh` per goal, `NET_DEPTH` behind the real back wall and well
  in front of `FR-029`'s own rigid back-of-net plane (unchanged, still a
  car's real backstop, since a car isn't tested against the net at all —
  a documented Non-goal, not an oversight). `PhysicsWorld` gains
  `nets`/`with_net`, resolved after every other contact each step. Every
  new constant (`net::NET_POINT_MASS`/`NET_POINT_RADIUS`/
  `NET_SPRING_CONSTANT`/`NET_SPRING_DAMPING`/`NET_LINEAR_DAMPING`/
  `NET_RESTITUTION`/`NET_FRICTION`, `arena::NET_DEPTH`) is an uncalibrated
  placeholder — real Rocket League net material properties have never
  been published. 10 net new tests: 5 in `net.rs` (perimeter anchoring,
  zero-stretch springs at rest, anchored points immovable under gravity,
  an undisturbed net settling instead of oscillating forever, and the
  real catching proof — a ball fired at the net's own center loses over
  half its speed within 1 second compared to free flight); `collision.rs`
  replaced the old `contacts_between_two_spheres_is_empty` regression test
  (whose entire premise this requirement reverses) with 2 tests proving
  `sphere_vs_sphere`'s own correctness (net +1); 2 in `arena.rs`
  (`standard_nets` returns exactly 2 nets, each sitting `NET_DEPTH` behind
  the real back wall and spanning exactly the goal mouth's own footprint);
  2 in `world.rs` (a wiring-count test, plus the real live end-to-end
  proof — a ball fired at a lone net panel in an isolated minimal scene
  loses at least half its speed compared to the identical shot with no
  net present). Bringing the crate to 256 tests total (+10 over FR-032's
  246). Still not modeled: a car's own contact against a net, a full 3D
  "sock" shape, bending stiffness, and everything else FR-024's own
  Non-goals already cover.
- 0.32.0 (2026-08-31): FR-032 added and resolved (genuine
  convex-vs-curved-surface narrow phase investigation — no change to the
  narrow phase itself). Set out to replace `box_vs_quarter_pipe`/
  `box_vs_corner_fillet`'s per-corner technique with a real GJK/EPA
  convex-vs-convex narrow phase, on the strength of a limitation FR-027's
  own doc comments claimed: a face resting flush against a shallow curve
  could have every corner still clear of the fillet while the face's
  middle already overlapped it, under-detecting that case. Built a
  from-scratch GJK closest-points implementation (`gjk::closest_points`)
  and wired it in — doing so broke two pre-existing, previously-passing
  end-to-end tests
  (`a_car_resting_within_a_curved_transitions_footprint_is_pushed_up_off_the_flat_floor_height`,
  `a_car_embedded_in_a_compound_corner_fillets_footprint_has_its_penetration_reduced`),
  because closest-point is the wrong question: a quarter-pipe/corner-fillet
  contact is a *containment* question (is the box's farthest point from the
  axis/center at or beyond radius), and distance-from-a-line/point is a
  convex function whose maximum over a convex polytope (the box) is always
  attained at a corner — so the original per-corner technique is
  mathematically exact for this question, not an approximation. Reverted
  `box_vs_quarter_pipe`/`box_vs_corner_fillet` to their original FR-027
  implementations, deleted the now-unused `gjk` module entirely (no
  remaining consumer), and corrected every doc comment across this crate
  and this spec that had inherited FR-027's unverified "approximation"/
  "under-detection" claim (`lib.rs`'s crate doc, this spec's own Purpose
  and scope, Non-goals, Requirements, and Verification plan sections, and
  the FR-027/FR-020 Open questions entry) to reflect the now-verified
  state. Added one new regression test,
  `no_point_on_a_boxs_face_is_ever_farther_from_a_quarter_pipes_axis_than_its_own_corners`,
  which densely samples (50×50 grid per face) all 6 faces of a car-sized
  box positioned exactly like the two broken end-to-end tests' scenario and
  confirms no sampled face-interior point ever exceeds the box's own 8
  corners' maximum distance from the axis. The goal wall's own analogous
  window-edge concern (`collision::box_vs_goal_wall`, FR-028) is a distinct
  question — the window boundary is a flat rectangle, not a curve — and
  remains open and unverified, not resolved by this investigation. 1 new
  test (246 total in `rb_physics_bullet`, +1 over FR-031's 245); no other
  test changes, since `box_vs_quarter_pipe`/`box_vs_corner_fillet`'s actual
  behavior is unchanged from FR-027.
- 0.31.0 (2026-08-31): FR-031 added and implemented (constant-calibration
  audit — does NOT close FR-005). Sourced every uncalibrated placeholder
  constant in `drive.rs`/`arena.rs`/`world.rs` against the RocketSim
  (`ZealanL/RocketSim`) and RLUtilities (`samuelpmish/RLUtilities`) source
  code plus the RLBot community wiki's "Useful Game Values" page — three
  independently-written reverse-engineering references, agreement across
  all three treated as high confidence. Corrected with code changes:
  `drive::JUMP_SPEED` (`292.0` → `875.0/3.0`, ≈291.667) and
  `drive::JUMP_HOLD_ACCELERATION` (`1400.0` → `4375.0/3.0`, ≈1458.33) to
  their precise real values; split `drive::MAX_CAR_SPEED` (2300, boost's
  cap, confirmed correct) from a new `drive::UNBOOSTED_MAX_CAR_SPEED`
  (1410, throttle's own cap) — a real behavioral fix, since throttle alone
  previously could reach the boosted top speed. Confirmed correct with no
  change: `drive::JUMP_HOLD_MAX_DURATION` (0.2), `drive::BOOST_ACCELERATION`
  (991.667), `drive::MAX_BOOST` (100), gravity (-650), `arena::GOAL_DEPTH`
  (880). Explicitly flagged as audited-but-still-uncalibrated (a real
  reference exists but doesn't safely port into this port's own unit
  system/mechanic shape, or no reference exists at all): `drive::DODGE_SPEED`,
  `drive::DODGE_ANGULAR_SPEED`, `drive::WALL_JUMP_HORIZONTAL_SPEED`,
  `drive::STEER_TORQUE`, `drive::AIR_CONTROL_TORQUE`,
  `drive::HANDBRAKE_FRICTION_MULTIPLIER`,
  `drive::LANDING_AUTO_UPRIGHT_TORQUE`, `arena::FILLET_RADIUS`,
  `arena::CORNER_ARCH_RADIUS`. Surfaced two open ambiguities without acting
  on them (ball radius 91.25 vs. this port's 92.75; `arena::CEILING_Z`
  2044 vs. RocketSim's cited 2048) — recorded in Open questions rather
  than guessed at. 1 new test (`drive::tests::throttle_alone_cannot_reach_the_boosted_top_speed`,
  245 total in `rb_physics_bullet`, +1 over FR-030's 244); the `JUMP_SPEED`/
  `JUMP_HOLD_ACCELERATION` precision refinements needed no new tests since
  every existing assertion already used a tolerance wide enough to absorb
  the small precision differences.
- 0.30.0 (2026-08-31): FR-030 added and implemented (combined multi-body
  solve) — closes the "combined multi-body solve" Non-goal/Open question
  this spec carried since FR-004: every ball-vs-car and car-vs-car
  contact manifold touching in the same step used to be resolved by its
  own independent call to `solver::resolve_contacts_between`, so a body
  touching two others at once (e.g. a car pinned between the ball and
  another car) never had both contacts reasoned about together — the
  second-resolved pair's setup used the first pair's already-finished
  result as its starting velocity, discarding almost all of the first
  contact's effect. New `solver::resolve_dynamic_manifolds(bodies,
  manifolds, dt)` shares one interleaved `SOLVER_ITERATIONS`-iteration
  budget across every manifold at once — every iteration processes every
  manifold once, each reading and updating the shared per-body-index
  `DeltaVelocity` accumulator a new `solver::delta_pair_mut` helper
  hands out (a `split_at_mut`-based disjoint-borrow generalization of the
  `b == a + 1` trick `PhysicsWorld::step`'s car-vs-car loop already used)
  — with velocities only applied once, after all iterations finish. The
  old `TwoBodyDelta` struct is removed in favor of two separate
  `DeltaVelocity` parameters on `resolve_two_body_row`, which is what
  makes sharing an accumulator across manifolds possible;
  `resolve_contacts`/`resolve_contacts_between`'s own public behavior is
  unchanged (the latter's internals were adjusted to match, a pure
  refactor). `PhysicsWorld::step` now builds one `Vec<RigidBody>` (ball
  plus every car), collects every non-empty ball-vs-car/car-vs-car
  manifold into one list, and resolves the whole scene's dynamic contacts
  with a single `resolve_dynamic_manifolds` call per step, replacing the
  two old independent loops and their now-deleted private
  `resolve_dynamic_contact` helper. Static contacts (ground, arena walls,
  curves, corner fillets, goal walls, bounded walls) are deliberately
  untouched — each depends on only one body, so independent resolution
  loses no cross-body information there. Measured via two new tests (1 in
  `solver.rs`, 1 in `world.rs`, bringing the crate to 244 total): a
  symmetric ball-between-two-closing-cars "pinch" whose true answer is
  every body near zero velocity. The old independent-pairwise approach
  left the ball at ~98-99% of a closing car's own speed (~98.9 units/s) —
  the first contact's effect almost entirely discarded; the new combined
  solve leaves it noticeably slower (~89.5 units/s at this crate's
  existing `SOLVER_ITERATIONS = 10`) — a real, measurable improvement,
  but not full convergence (confirmed, during test design only, to
  converge much closer at 300 iterations, while the old approach's result
  doesn't change at all regardless of iteration count — proving its error
  is structural, not an iteration-count shortfall). Both tests
  deliberately assert the direction/magnitude of the improvement, not
  exact convergence to zero, matching this spec's practice of not
  overclaiming what a test proves. Non-goals unaffected: split impulse,
  warm-starting/sleeping, and the average-not-max restitution/friction
  combine mode are exactly as documented before this requirement.
- 0.29.0 (2026-08-31): FR-029 added and implemented (modeled goal
  interior) — closes the last remaining goal-cutout Non-goal repeated
  across FR-024 through FR-028: the goal-mouth window opened onto
  completely open, unbounded space, so a ball or car passing through (the
  ball since FR-024, a car since FR-028) sailed forever with nothing
  behind the window to stop it. This closes that gap with a bounded
  interior volume behind each goal-mouth window — explicitly a solid
  bounding box, **not** a springy/catching net mesh (no cloth/soft-body
  simulation was added); a deliberate, honest scoping decision, not an
  oversight, with a genuine net mesh remaining a real, separate,
  not-yet-implemented Non-goal. Three pieces, all in
  `crates/rb_physics_bullet`: a new constant `arena::GOAL_DEPTH: f32 =
  880.0` — an uncalibrated placeholder, since this port has no verified
  reference for Rocket League's real net depth at all (unlike
  `GOAL_HALF_WIDTH`/`GOAL_HEIGHT`), chosen only to be a visibly real
  interior volume comparable in scale to the goal mouth's own dimensions;
  a new shape type, `body::StaticBoundedWall` — a flat `StaticPlane` that
  only collides *within* a rectangular bound in the plane's own local 2D
  frame (`bound_center`/`u_axis`/`v_axis`/`half_u`/`half_v`, plus a
  `contains_in_bound` method) — the opposite gate convention from
  `StaticGoalWall`'s window (collides everywhere *except* inside a
  rectangle), needed because the goal's own side walls and roof can't be
  plain unbounded `StaticPlane`s (an infinite plane at, say, `x =
  GOAL_HALF_WIDTH` would incorrectly wall off the entire main field at
  that x coordinate — the exact same problem `arena::goal_post_plane`'s
  own pre-existing doc comment already documented for a different
  purely-geometric plane used only to derive fillets), with new dispatch
  functions in `collision.rs` (`sphere_vs_bounded_wall`/
  `box_vs_bounded_wall`/`contacts_vs_bounded_wall` — `box_vs_bounded_wall`
  uses the same "test every corner" technique established by
  FR-027/FR-028, but a corner *outside* the bound is skipped, the opposite
  of `box_vs_goal_wall`'s per-corner window test); and new arena geometry
  functions in `arena.rs` — `goal_back_wall_plane`/
  `standard_goal_back_walls` (2 plain, unbounded `StaticPlane`s per goal,
  `GOAL_DEPTH` behind the real back wall, added to `PhysicsWorld.walls` via
  the existing `with_wall` builder, since nothing can ever reach this
  plane except by first passing through the goal-mouth window, so an
  unbounded plane here is exact, not an approximation), `goal_side_wall`/
  `standard_goal_side_walls` (4 `StaticBoundedWall`s total, each reusing
  `goal_post_plane` completely unchanged, bounded to the goal's own depth
  and height), and `goal_roof`/`standard_goal_roofs` (2
  `StaticBoundedWall`s total, each reusing `goal_crossbar_plane`
  completely unchanged, bounded to the goal's own width and depth).
  `PhysicsWorld` (in `world.rs`) gains a new field
  `bounded_walls: Vec<StaticBoundedWall>` and a `with_bounded_wall` builder
  (mirroring `with_goal_wall`), plus a new `resolve_bounded_wall_contact`
  (mirroring `resolve_goal_wall_contact`), resolved for the ball and every
  car in `PhysicsWorld::step`, exactly like `goal_walls`.
  `PhysicsWorld::standard_arena` wires in `standard_goal_back_walls` via
  `with_wall`, and `standard_goal_side_walls`/`standard_goal_roofs` via
  `with_bounded_wall`. No changes to the actual step-loop resolution
  pattern were needed beyond adding the new loop — unlike FR-027/FR-028,
  this isn't a "silent no-op that later got activated" story; it's
  straightforwardly new geometry wired in from the start.
  `PhysicsWorld.walls` grew from 7 to 9 real entries once `standard_arena`
  is built (the 2 new back-of-net planes), so the pre-existing `world.rs`
  test `standard_arena_has_seven_walls_and_the_standard_ground` was
  renamed `standard_arena_has_nine_walls_and_the_standard_ground` and its
  assertion updated — a test-count correction, not new capability. New
  tests: 4 in `body.rs` for `StaticBoundedWall::contains_in_bound`
  (`contains_in_bound_is_true_for_the_bounds_own_center`,
  `contains_in_bound_is_true_just_inside_each_edge`,
  `contains_in_bound_is_false_just_outside_each_edge`,
  `contains_in_bound_ignores_distance_from_the_plane_itself`), mirroring
  the pre-existing `StaticGoalWall::contains_in_window` tests exactly with
  the boolean gate meaning inverted; 5 in `collision.rs`
  (`sphere_inside_the_bound_behaves_like_an_ordinary_plane`,
  `sphere_outside_the_bound_has_no_contact`,
  `box_squarely_inside_the_bound_behaves_like_an_ordinary_plane`,
  `box_straddling_the_bounds_edge_only_collides_on_the_corners_still_inside_it`,
  `box_entirely_outside_the_bound_has_no_contact`) against a synthetic
  fixture, mirroring the `StaticGoalWall` collision tests with the gate
  inverted; 8 in `arena.rs` proving the geometry functions place things
  correctly (`standard_goal_back_walls_has_two_walls`,
  `every_goal_back_wall_sits_goal_depth_behind_the_real_back_wall`,
  `standard_goal_side_walls_has_four_walls`,
  `every_goal_side_walls_plane_matches_some_goal_post_plane`,
  `every_goal_side_walls_bound_covers_the_real_goal_depth_and_height`,
  `standard_goal_roofs_has_two_roofs`,
  `every_goal_roofs_plane_is_the_goal_crossbar_plane`,
  `every_goal_roofs_bound_covers_the_real_goal_width`); and 4 in
  `world.rs` — 1 wiring-count test (`standard_arena_has_six_bounded_walls`)
  and 3 new live end-to-end `PhysicsWorld` proofs
  (`a_ball_shot_into_the_goal_is_stopped_by_the_goal_back_wall`,
  `a_ball_shot_sideways_inside_the_goal_is_stopped_by_a_goal_side_wall`,
  `a_ball_shot_upward_inside_the_goal_is_stopped_by_the_goal_roof`). Net
  +21 tests, bringing the crate to 242 tests total. These 3 end-to-end
  tests are deliberately isolated to a minimal `PhysicsWorld` built from
  just the specific new wall(s) under test (`PhysicsWorld::new` plus
  `with_wall`/`with_bounded_wall`, not `PhysicsWorld::standard_arena`) —
  discovered empirically while writing them: using the full
  `standard_arena`, a ball fired sideways or upward from deep inside the
  goal box got flung to bizarre, wildly wrong positions (e.g. ending up
  at x=-687 after being fired only in +x), root-caused to the
  pre-existing, already-documented `StaticQuarterPipe` limitation that a
  fillet's sector-membership test only checks angular position around its
  own axis, not radial distance (the same category of issue as the
  earlier FR-025 test-writing discovery), which the standard arena's own
  goal-cutout post/crossbar fillets (sitting right at the window's edge)
  could spuriously trigger for a point deep inside the goal box; isolating
  the test scene to just the wall(s) actually under test sidesteps this
  entirely and is the correct fix, not a bug in the new
  `StaticBoundedWall`/`goal_back_wall_plane` code itself. Additionally, an
  early version of these 3 tests set only the ball's own `restitution =
  0.0` and got nondeterministic results for the roof test specifically
  (the ball ending up below its own starting height), root-caused to the
  wall's own default `StaticPlane::new` restitution (0.5) still applying
  in the solver's contact resolution regardless of the ball's own value,
  causing it to bounce back down with enough remaining simulation time to
  travel well past its start; fixed by also explicitly zeroing the
  specific wall(s)' own `plane.restitution` in each of these 3 tests
  before adding them to the scene, so the ball damps out deterministically.
  Still not modeled: a genuine net *mesh* — no cloth/soft-body simulation,
  no visual net sag, no "ball tangles in netting" behavior; this is a
  solid bounding volume standing in for the net's functional role
  (stopping the ball/car), nothing more.
- 0.28.0 (2026-08-31): FR-028 added and implemented (car actually driving
  into a goal) — closes the last remaining half of the goal-cutout
  Non-goal FR-024 through FR-027 kept repeating:
  `collision::contacts_vs_goal_wall` dispatched a `Shape::Box` straight through to
  plain `contacts_vs_plane` against the wrapped `StaticPlane`, completely
  ignoring the goal-mouth window `StaticGoalWall` carries, even though a
  sphere (the ball) already passed through the window via
  `sphere_vs_goal_wall`. New `collision::box_vs_goal_wall(position,
  orientation, half_extents, wall) -> Vec<Contact>` iterates the box's 8
  corners exactly like `box_vs_plane` does, but for each corner first
  checks `wall.contains_in_window(&world_corner)` — a corner inside the
  window contributes no contact at all (`continue`), regardless of how
  deep it might be penetrating along the plane's normal, exactly
  mirroring `sphere_vs_goal_wall`'s own distance-along-normal-independent
  window test (`contains_in_window` needed no changes); a corner outside
  the window falls through to an ordinary `box_vs_plane`-style corner
  test. This is the exact same "test every corner" approximation
  technique `RB-PHYSICS-001-FR-027` established for curved fillets,
  applied here to a flat windowed plane instead. One real behavioral
  consequence worth documenting: because each corner is tested
  independently, a car only partially lined up with the window gets a
  *partial* block — contacts register on whichever corners are still
  outside it — rather than the all-or-nothing result a single-point
  sphere test necessarily produces. `contacts_vs_goal_wall`'s dispatch
  changed from `Shape::Box { .. } => contacts_vs_plane(body,
  &wall.plane)` to `Shape::Box { half_extents } =>
  box_vs_goal_wall(body.position, body.orientation, half_extents,
  wall)`. No `world.rs` step-loop changes were needed — exactly like
  FR-027's own discovery: `PhysicsWorld::step`'s
  `resolve_goal_wall_contact` was already being called for every car in
  the scene (it always needed the wall's plain-plane collision even
  before this fix), so this is a pure dispatch-function change; only
  doc-comment updates were needed (`goal_walls`' field doc comment,
  `with_goal_wall`'s doc comment, and `resolve_goal_wall_contact`'s doc
  comment in `world.rs`; `body.rs`'s `StaticGoalWall` doc comment;
  `arena.rs`'s module-level and `standard_goal_walls`'s own doc
  comments; and `lib.rs`'s crate-level module doc comment), removing
  "unwindowed"/"falls straight through to an ordinary plane contact,
  ignoring the window" language now that a car passes through too. New
  tests: `collision.rs` replaced the old
  `box_vs_goal_wall_ignores_the_window_entirely` regression test, whose
  entire premise this requirement reverses, with three new tests —
  `box_squarely_inside_the_goal_window_has_no_contact` (every corner
  inside the window gives an empty manifold, the box equivalent of
  `sphere_embedded_in_the_goal_window_has_no_contact`),
  `box_straddling_the_goal_window_edge_only_collides_on_the_corners_still_outside_it`
  (a car centered on the window's own edge gets exactly 2 contacts, only
  on the corners outside it — the real proof of the partial-block
  behavior above), and
  `box_entirely_outside_the_goal_window_behaves_like_an_ordinary_plane`
  (a car nowhere near the window collides identically to plain
  `contacts_vs_plane`, the box equivalent of
  `sphere_outside_the_goal_window_behaves_like_an_ordinary_plane`) — net
  +2. `world.rs` replaced
  `a_car_is_still_stopped_by_the_standard_arenas_back_wall_at_the_goal_mouth`,
  whose entire premise this requirement reverses, with
  `a_car_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall`
  (a live end-to-end `PhysicsWorld` proof: a car fired at the exact same
  goal-mouth-center position/velocity the pre-existing ball test uses
  ends up past `BACK_WALL_Y` instead of being stopped, reusing that
  test's already-solved 1.8s flight-duration bound) and added a new
  regression guard,
  `a_car_aimed_away_from_the_goal_mouth_is_still_stopped_by_the_back_wall`
  (a car aimed well outside `GOAL_HALF_WIDTH`, at the solid part of the
  wall, is still stopped by it after 3.0s) — net +1. Net +3 tests,
  bringing the crate to 221 tests total (net +2 in `collision.rs`, net +1
  in `world.rs`). Still not modeled: a modeled goal interior/net beyond
  the cutout itself, a genuine convex-vs-curved-surface narrow phase for
  a car (support-mapping/SAT-style, e.g. GJK/EPA) in place of this
  port's corner-testing approximations, and everything else FR-024's own
  Non-goals already cover.
- 0.27.0 (2026-08-31): FR-027 added and implemented (car deflection by
  curved fillets) — closes the long-repeated Non-goal, documented across
  FR-020 through FR-026, that a car (box) drove straight through every
  curved fillet in this port unaffected while the ball was properly
  deflected. New `collision::box_vs_quarter_pipe(position, orientation,
  half_extents, pipe)` tests each of the box's 8 corners as a zero-radius
  sphere via the existing `sphere_vs_quarter_pipe(world_corner, 0.0, pipe)`
  call and collects every corner that reports a contact — exactly the same
  "test every corner" technique `box_vs_plane` already uses for a box
  against a flat plane (already in this codebase, generating up to 4
  contacts for a box resting on a plane), generalized here to a curved
  surface instead of a flat one; new `box_vs_corner_fillet` does the
  identical thing against `sphere_vs_corner_fillet` for the sphere-shaped
  compound-corner fillets. `contacts_vs_quarter_pipe`/
  `contacts_vs_corner_fillet` now dispatch a `Shape::Box` to these new
  functions instead of returning `Vec::new()`. Each surviving contact's
  `point` field is overwritten to the corner's own world position rather
  than the fillet-surface point `sphere_vs_quarter_pipe`/
  `sphere_vs_corner_fillet` itself computes, for the same rel_pos/
  torque-accuracy reason `box_vs_plane`'s own doc comment already
  documents (a tilted box's corner isn't generally "below" the body center
  along the surface normal, so the solver needs the true contact-to-center
  offset). Both new functions' own doc comments document this explicitly
  as an approximation, not a full convex-vs-curved-surface narrow phase —
  no GJK/EPA support-mapping machinery was added, and this port still
  doesn't have any: a face of the box resting flush against a shallow
  curve (a radius large relative to the box) could have every one of its
  own corners still just clear of the fillet while the face's middle
  already overlaps it, under-detecting that case — the same category of
  "exact per test point, an approximation of the whole shape" caveat this
  crate has always carried for curved geometry. No new shape or
  fundamentally new collision primitive was needed — this is a
  generalization of existing dispatch, not new physics/math machinery.
  `PhysicsWorld::step` already called `resolve_curve_contact`/
  `resolve_corner_fillet_contact` for every car in the scene (wired in
  since FR-023), just always a no-op before now, since
  `contacts_vs_quarter_pipe`/`contacts_vs_corner_fillet` always returned
  empty for a box — so no `world.rs` step-loop changes were needed, only
  doc-comment updates reflecting the new real behavior (the "no-op for a
  box" language in `resolve_curve_contact`/`resolve_corner_fillet_contact`'s
  own doc comments, the `curves`/`corner_fillets` field doc comments, the
  `with_curve`/`with_corner_fillet` builder doc comments, and `arena.rs`'s
  own module-level "Still not modeled" list). Explicitly unaffected:
  `StaticGoalWall`/`contacts_vs_goal_wall` — a car still sees the exact
  same solid, full-width back wall it always has, deliberately ignoring
  the goal-mouth window, since a goal wall isn't a curved fillet at all; a
  car actually driving into a goal remains a real, separate,
  not-yet-implemented capability. New tests: `collision.rs` renamed/
  replaced the 2 pre-existing "always empty for a box" regression tests
  (`box_vs_quarter_pipe_is_always_empty`, `box_vs_corner_fillet_is_always_empty`)
  with 4 new tests proving the corner-testing approximation actually works
  both ways — `box_embedded_in_the_quarter_pipes_footprint_has_contact` and
  `box_far_from_the_quarter_pipe_has_no_contact` for the cylindrical
  quarter-pipe, `box_embedded_in_the_corner_fillets_footprint_has_contact`
  and `box_outside_the_corner_fillets_bounds_has_no_contact` for the
  sphere-shaped compound-corner fillet (net +2); `world.rs` replaced the
  pre-existing `a_car_is_not_deflected_by_a_curved_transition` regression
  test, whose entire premise this requirement reverses, with
  `a_car_resting_within_a_curved_transitions_footprint_is_pushed_up_off_the_flat_floor_height`
  (mirroring the ball's own equivalent live-physics proof) and added
  `a_car_embedded_in_a_compound_corner_fillets_footprint_has_its_penetration_reduced`
  (net +1) — the latter deliberately asserts that the fillet's own worst
  (maximum) corner penetration shrinks after the solver runs, not that the
  box's center of mass moves closer to the fillet's center the way the
  equivalent ball test does, since an earlier version asserting
  center-of-mass distance actually failed empirically (a box's multiple
  simultaneously-penetrating corners can rotate the box away from the
  fillet's center even as every individual corner's own overlap shrinks —
  see the spec's own Verification plan for the full reasoning). Net +3
  tests, bringing the crate to 218 tests total (2 in `collision.rs` net
  above the 2 replaced, plus 1 in `world.rs` net above the 1 replaced).
  Still not modeled: a car actually driving into a goal, a modeled goal
  interior/net beyond the cutout itself, a genuine convex-vs-curved-surface
  narrow phase for a car (support-mapping/SAT-style, e.g. GJK/EPA) in place
  of this increment's corner-testing approximation, and everything else
  FR-024's own Non-goals already cover.
- 0.26.0 (2026-08-31): FR-026 added and implemented (goal post-crossbar
  corner fillets) — rounds off the two remaining sharp compound corners per
  goal (4 total, 2 per goal), where a post's own vertical-edge fillet
  (FR-024) meets the crossbar's own horizontal-edge fillet, an explicitly
  documented gap FR-024's own doc comment left open. New
  `arena::standard_goal_corner_fillets` builds all 4 by calling
  `StaticCornerFillet::between_three_planes` directly on the three real
  flat planes that meet at each vertex (the back wall, that post's own
  plane, and the crossbar) — the same direct-from-real-planes approach
  FR-023 used for the arena's own 16 compound corners, rather than deriving
  from the two edge fillets already built at that vertex, since a corner
  fillet's center is already exactly those two edge fillets' own common
  axis intersection. No new shape or collision code needed —
  `StaticCornerFillet`/`sphere_vs_corner_fillet` (FR-023) are already fully
  generic to any three non-parallel planes. Unlike FR-025, this increment
  reuses `FILLET_RADIUS` unchanged for all 4 new fillets rather than
  introducing a new radius constant, since both edge fillets meeting at a
  goal's post-crossbar corner already share `FILLET_RADIUS` — no
  mismatched-radius concern exists here. `PhysicsWorld::standard_arena`
  wires these 4 in via the same generic `with_corner_fillet` builder
  `standard_corner_fillets`'s 16 already use, so `PhysicsWorld.corner_fillets`
  now holds 20 total; the pre-existing test
  `standard_arena_has_sixteen_compound_corner_fillets` was renamed
  `standard_arena_has_twenty_compound_corner_fillets` and its assertion
  updated to match. New tests: `arena.rs` gained
  `standard_goal_corner_fillets_has_four_fillets` and
  `every_goal_corner_fillets_center_sits_radius_in_from_a_back_wall_a_post_and_the_crossbar`
  (the same "prove the real triple intersection, not an arbitrary point"
  style test FR-023's own arena-corner test used); `world.rs` gained
  `a_ball_embedded_in_a_goal_corner_fillets_footprint_is_pushed_toward_the_center`
  (the same live-physics "ball embedded past the fillet's own radius gets
  pushed back toward the center" proof already given for every other
  fillet type in this port, using a synthetic back-wall/post/crossbar
  3-plane fixture rather than the real arena's own numbers, matching this
  test file's established convention for fillet unit tests). Net +3 tests,
  bringing the crate to 215 tests total (2 in `arena.rs`, 1 in `world.rs`).
  Explicitly still out of scope: the goal's other two corners, where a post
  meets the floor — the window's own bottom edge sits exactly at floor
  level (`z = 0`), so a post's own fillet there simply ends flush with the
  ground, with no sharp, unrounded vertex requiring a blend, unlike the top
  post-crossbar corner this increment addresses. Still not modeled: a car
  actually being deflected by any fillet or driving into a goal, a modeled
  goal interior/net beyond the cutout itself, and everything else FR-024's
  own Non-goals already cover.
- 0.25.0 (2026-08-30): FR-025 added and implemented (corner-wall
  floor/ceiling arch radius) — gives a diagonal corner wall's own
  floor-seam and ceiling-seam fillets a distinctly larger, dedicated
  radius instead of reusing the cardinal walls' own `arena::FILLET_RADIUS`
  (292.0), matching real Rocket League's corner-boost area reading as a
  noticeably bigger, more swept curve than a cardinal wall's small
  rounding, not just a scaled-down copy of the same shape. New constant
  `arena::CORNER_ARCH_RADIUS` (750.0), documented as an uncalibrated
  placeholder same as every other arena dimension in this module, governs
  8 of `standard_curves`' 24 entries — the ones bridging one of the 4
  corner walls to the floor or ceiling; a compile-time
  `const _: () = assert!(CORNER_ARCH_RADIUS > FILLET_RADIUS);` enforces
  the "distinctly larger" relationship at build time. Because
  `StaticCornerFillet::between_three_planes` needs one shared radius
  across all three planes it blends — a mismatched radius wouldn't
  produce a geometrically valid single sphere satisfying all three
  radius-in conditions, and would also break FR-023's established
  "meets its adjoining edge fillets exactly where their axes cross"
  no-gap property — all 16 of `standard_corner_fillets`'s
  compound-corner fillets switch to `CORNER_ARCH_RADIUS` too, since every
  one of them touches one of the 8 now-larger arches. Unaffected, still
  using `FILLET_RADIUS` exactly as before: the 8 cardinal-wall
  floor/ceiling-seam fillets, the 8 vertical corner-edge fillets
  (`RB-PHYSICS-001-FR-022`), and the 6 goal-cutout edge fillets
  (`RB-PHYSICS-001-FR-024`) — independent, additive contact sources next
  to the bigger arches, not blended with them, per this port's established
  "no blended 3D corner" convention. New end-to-end `world.rs` test
  `a_ball_embedded_in_a_corner_walls_floor_arch_footprint_is_pushed_toward_the_axis` gives the live-physics proof for the new radius, the same
  "moved meaningfully" claim every other fillet test in this port makes,
  and additionally asserts `CORNER_ARCH_RADIUS > FILLET_RADIUS`.
  `arena.rs`'s existing tests were updated to match: `every_floor_or_ceiling_seam_curve_bridges_a_wall_to_the_floor_or_ceiling` now checks
  the first 8 of `standard_curves()`'s 24 entries against `FILLET_RADIUS`
  and the next 8 against `CORNER_ARCH_RADIUS` separately (previously all
  16 floor/ceiling entries were checked against one radius);
  `every_standard_curve_sits_radius_in_from_a_vertical_wall` now accepts
  either radius; `every_standard_corner_fillets_center_sits_radius_in_from_a_floor_or_ceiling_a_side_or_back_wall_and_a_corner_wall` now
  checks `CORNER_ARCH_RADIUS` instead of `FILLET_RADIUS`. Net +1 test (one
  new `arena.rs` test idea was implemented as the compile-time
  const-assert above instead of a runtime test, alongside the one new
  `world.rs` end-to-end test), bringing the crate to 212 tests total.
  While validating this change, the pre-existing `world.rs` test
  `a_ball_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall`
  (FR-024) started failing — a discovered-and-fixed test regression, not a
  new feature or Non-goal. Root cause: `StaticQuarterPipe` is documented
  (in `body.rs`, unchanged by this increment) as infinite along its own
  axis, not clipped to a corner wall's real, finite span, so a ball fired
  dead down the arena's own center line eventually re-enters *some*
  corner-wall floor-seam arch's resting shell far past the goal — already
  true before FR-025 with the smaller `FILLET_RADIUS` (verified against
  the pre-FR-025 code, where the ball drifts into this zone around
  y≈7650-7930 and gets a mild, harmless correction that still leaves it
  past the wall), but FR-025's bigger `CORNER_ARCH_RADIUS` moves that zone
  closer in (~y=6300-7700) and turns the same brush into a much sharper,
  solver-destabilizing correction (velocity spikes to tens of thousands of
  units/sec, throwing the ball back past the wall and failing the test's
  assertion). Fixed by shortening that one test's simulated flight
  duration from 3.0s to 1.8s — comfortably long enough to prove the ball
  clears the back wall (needs y > 5121, reaches y=5400 unobstructed by
  1.8s) while stopping well short of re-entering the infinite-fillet zone,
  with a code comment in the test explaining why; `StaticQuarterPipe`'s
  own documented scope is otherwise unchanged. Still not modeled: a car
  actually being deflected by any fillet, and everything else FR-024's own
  Non-goals already cover.
- 0.24.0 (2026-08-30): FR-024 added and implemented (goal cutouts) —
  opens an actual goal-mouth window in each back wall, rounded at its own
  rim, where every prior increment had a single solid, flat plane spanning
  the full width. New static shape `body::StaticGoalWall` — a `StaticPlane`
  plus a rectangular window in the plane's own local `u_axis`/`v_axis`
  frame (`window_center`, `half_width`, `half_height`), with
  `contains_in_window` testing a point's projection onto that frame
  directly, independent of the point's own depth from the plane. New
  `collision::sphere_vs_goal_wall`/`contacts_vs_goal_wall`: a sphere (the
  ball) gets no contact at all when its center falls inside the window,
  letting it pass through; a box (car) falls straight through to the
  ordinary `contacts_vs_plane` against the wrapped plane, deliberately
  ignoring the window — a zero-regression choice, since a car now sees
  literally the same contact-generation call it always did. `arena::standard_walls` drops its 2 back-wall `StaticPlane`s (now 7 planes
  instead of 9); new `arena::standard_goal_walls` returns them instead as
  2 `StaticGoalWall`s, windowed at new commonly-cited constants
  `GOAL_HALF_WIDTH`/`GOAL_HEIGHT`. New `arena::standard_goal_cutout_fillets`
  rounds each window's 3 edges (two posts, one crossbar, per goal — 6
  `StaticQuarterPipe`s, added to the same `curves` list `standard_curves`'s
  24 already populate), each derived via the existing
  `StaticQuarterPipe::between_planes` from the real back-wall plane and a
  second, purely-geometric plane (`goal_post_plane`/`goal_crossbar_plane`)
  representing the post's or crossbar's own inward-/downward-facing
  surface — positioned at exactly the window's own edge, so the fillet's
  tangent point lands exactly on the window boundary with no gap or
  overlap. Unlike `standard_walls`' real walls, these post/crossbar planes
  are never themselves added as collision geometry — an infinite plane
  facing straight along X (or capping Z) would incorrectly wall off the
  entire rest of the field at that coordinate, unlike a diagonal corner
  wall's own orientation, which stays non-binding everywhere except right
  at the true corner. `PhysicsWorld` gains a parallel
  `goal_walls: Vec<StaticGoalWall>` field and `with_goal_wall` builder,
  resolved for the ball *and* every car (unlike `curves`/`corner_fillets`'s
  ball-only resolution) — safe precisely because the box path is a no-op
  change from the prior plain-`StaticPlane` behavior; `PhysicsWorld::standard_arena` wires in both the goal walls and the goal-cutout fillets
  automatically. 17 new unit tests (4 in `body.rs` for
  `StaticGoalWall::contains_in_window`'s correctness against a synthetic
  fixture, 4 in `collision.rs` for `sphere_vs_goal_wall`'s contact
  generation and `contacts_vs_goal_wall`'s box-path equivalence to plain
  `contacts_vs_plane`, 5 in `arena.rs` for `standard_walls`'s reduced
  count and the new `standard_goal_walls`/`standard_goal_cutout_fillets`
  functions' counts and geometric correctness, and 4 in `world.rs` — two
  end-to-end tests proving a ball passes through the goal mouth while a
  car aimed at the same spot is still stopped by the wall, one proving a
  ball embedded past a goal-post fillet's own radius gets pushed
  meaningfully back toward the axis, and one confirming `standard_arena`
  carries exactly 2 goal walls) bring the crate to 211 tests total. Still
  not modeled: a car actually being deflected by any fillet or driving
  into a goal, a modeled goal interior/net beyond the cutout itself, and
  the goal's own two compound top corners where a post's fillet meets the
  crossbar's (see Non-goals).
- 0.23.0 (2026-08-30): FR-023 added and implemented (compound-corner
  fillets) — rounds off the last 16 sharp vertices in the standard arena's
  vertical boundary: the compound corners where a corner wall's own
  vertical-edge fillet (FR-022) meets a floor- or ceiling-seam fillet
  (FR-020/FR-021), near that corner wall's own top or bottom endpoint. A
  compound corner is where three planes meet at once, which no existing
  cylindrical `StaticQuarterPipe` can blend, so this version introduces a
  new static shape, `body::StaticCornerFillet` — an immovable sphere riding
  the concave inside of the vertex. Its `between_three_planes` constructor
  exploits the same "radius-in from every bridged plane" invariant
  `StaticQuarterPipe::between_planes` already relies on: the fillet's
  center must sit exactly `radius` in from all three planes, so it's also
  exactly `radius` in from each pair — meaning it already lies on all
  three of that vertex's own pairwise `between_planes` axis lines
  simultaneously, so the center is just those three lines' common
  intersection, solved directly via the classic three-plane-intersection
  cross-product form of Cramer's rule rather than from scratch.
  Containment (new `sphere_vs_corner_fillet` in `collision.rs`)
  generalizes a `StaticQuarterPipe`'s 2-sided sector test to a "spherical
  triangle": inside iff a direction's dot product with each of 3 `bounds`
  is non-negative, each bound the raw (non-normalized — only its sign is
  used) cross product of a pair of normals, sign-corrected against the
  third plane's own normal to always point toward the sharp corner —
  provably correct since that dot product is exactly the derivative of
  the third plane's signed distance along a candidate direction. No
  `.normalize()`/`.unwrap()` is needed or used anywhere in this new
  production code, consistent with `between_planes`'s own FR-022
  precedent. `arena::standard_corner_fillets` builds all 16 (4 per corner
  wall — floor+side, floor+back, ceiling+side, ceiling+back — times the 4
  corner walls) directly from the same three flat planes `standard_walls`
  already builds, reusing `FILLET_RADIUS` once again. `PhysicsWorld` gains
  a parallel `corner_fillets: Vec<StaticCornerFillet>` field and
  `with_corner_fillet` builder, resolved for the ball and every car
  exactly like `curves` (a no-op for a car, same deferred case as every
  other fillet); `PhysicsWorld::standard_arena` wires in all 16
  automatically. 13 new unit tests (4 in `body.rs` for
  `between_three_planes`'s center/tangent-point/containment correctness
  against a synthetic fixture, 5 in `collision.rs` for
  `sphere_vs_corner_fillet`'s contact generation, 2 in `arena.rs` for
  `standard_corner_fillets`' count and per-fillet radius-in property, and
  2 in `world.rs` — a `standard_arena` fillet-count check and an
  end-to-end test proving a ball embedded past a compound-corner fillet's
  own radius gets pushed meaningfully back toward the center, the same
  "moved meaningfully" — not "settled-and-stayed" — claim FR-020/FR-021/
  FR-022's own equivalent tests make, for the same residual-velocity
  reason) bring the crate to 194 tests total. Still not modeled: a car
  actually being deflected by any fillet, and goal cutouts (see
  Non-goals).
- 0.22.0 (2026-08-30): FR-022 added and implemented (curved corner-wall
  vertical-edge fillets) — rounds off the arena's last remaining sharp
  edges: the 8 vertical edges where each of the 4 diagonal corner walls
  meets its neighboring side or back wall. `arena::standard_curves` now
  builds 24 `StaticQuarterPipe`s (the 16 floor/ceiling-seam fillets
  FR-020/FR-021 already built, plus 8 vertical-edge fillets). Unlike every
  prior fillet, the two planes a vertical-edge fillet bridges aren't
  perpendicular — a corner wall meets its neighbor at 135 degrees (given
  `standard_walls`' 45-degree corner cut), not 90 — which exposed a real
  gap in `StaticQuarterPipe::between_planes`: it previously only computed
  the correct axis point for perpendicular planes, via a shortcut (summing
  the two scaled normals) that silently gives the wrong point for any other
  angle. `between_planes` is now fully general: it solves the axis point as
  an actual 2x2 linear system in the (possibly non-orthogonal) basis the
  two normals form, and its own sector angle comes out to exactly
  `arccos(dot(plane_a.normal, plane_b.normal))` — a right angle for
  perpendicular planes as before, or (for this requirement's own geometry)
  a shallow 45 degrees, the supplement of the walls' 135-degree dihedral
  angle. `sphere_vs_quarter_pipe`'s sector-membership test is likewise
  generalized from the old two-dot-products check (only correct for a
  90-degree sector, since its two edges happen to be perpendicular) to a
  signed-cross-product test against `axis_direction`, exact for any sector
  up to 180 degrees. Since the general test depends on `axis_direction`'s
  own sign/handedness — unlike the old test, which never used
  `axis_direction` at all — `between_planes` now self-corrects a
  "backwards" `axis_direction` internally, so a caller can pass either of
  the two opposite directions along the shared edge line without reasoning
  about which is correct. The vertical-edge fillets' own `axis_direction`
  is simply `(0, 0, 1)` (the edge itself is vertical) — no cross product
  needed, unlike the corner-wall floor/ceiling-seam case. `FILLET_RADIUS` is
  reused as-is once again rather than a separate, smaller radius for these
  visibly shallower edges. Still not modeled: a car actually being
  deflected by any fillet, the compound corner where a vertical-edge
  fillet meets a floor- or ceiling-seam fillet, and goal cutouts (see
  Non-goals). 9 new unit tests across `body.rs`/`arena.rs`/`world.rs` in
  `rb_physics_bullet` (181 total): 5 in `body.rs`, using a synthetic
  non-perpendicular fixture independent of the arena's own geometry — the
  axis still sits exactly `radius` in from both planes with tangent points
  exactly on each; the derived sector angle matches the angle between the
  two planes' normals (45 degrees for this fixture); the sharp corner the
  fillet replaces sits outside its own radius but within its sector (the
  real proof the generalized sector orientation actually faces the missing
  material); and passing either of the two opposite `axis_direction`
  choices produces the same correctly-oriented sector; 3 in `arena.rs` —
  `standard_curves` returns exactly 24 fillets, every vertical-edge
  fillet's `axis_direction` runs purely along Z, and a corner wall's own
  vertical-edge fillet sits radius-in from both the corner wall and its
  neighboring side wall with a 45-degree sector; 1 in `world.rs` — the real
  end-to-end proof, a ball embedded past a vertical-edge fillet's own
  radius (at a wall-to-wall angle that isn't a right angle) gets pushed
  meaningfully back toward the axis (not a claim that it settles and stays
  at the exact resting distance — like every other fillet here, its
  contact stops firing once the overlap resolves, so nothing cancels
  whatever residual velocity the correction left the ball with, the same
  reason FR-020's and FR-021's own equivalent tests make the same weaker,
  "moved meaningfully" claim rather than an exact-settling one).
- 0.21.0 (2026-08-30): FR-021 added and implemented (curved
  corner-wall-to-floor/wall-to-ceiling transitions) — extends FR-020's
  fillet treatment to the 4 diagonal corner walls `FR-019` introduced.
  `arena::standard_curves` now builds 16 `StaticQuarterPipe`s (still one
  floor-side and one ceiling-side fillet per wall, now for all 9 walls)
  instead of 8. `StaticQuarterPipe::between_planes` itself needed no code
  changes: its real correctness requirement was never "axis-aligned
  planes" (as FR-020's own doc comment had incorrectly claimed) but only
  that the two bridged planes' normals, plus `axis_direction`, form an
  orthonormal basis — which only needs the two planes to be mutually
  *perpendicular*, true for a corner wall meeting the floor or ceiling
  regardless of the corner wall's own horizontal rotation (a vertical
  wall's normal always has zero Z component, and the floor/ceiling's is
  always purely Z). The only new work is in `arena.rs`'s
  `standard_curves`: a cardinal wall's fillet axis direction was always
  hand-picked as a coordinate axis, but a corner wall's own "along the
  wall" direction isn't one, so it's instead computed via a cross product
  (`floor.normal.cross(&wall.normal)`, and the ceiling equivalent) —
  already exactly unit length by construction (the two operands are always
  exactly perpendicular unit vectors), so no `.normalize()`/`.unwrap()` is
  needed, avoiding a `clippy::unwrap_used` violation the workspace's lint
  config promotes to a hard CI error in production code. A new
  `corner_wall_plane(sx, sy)` helper in `arena.rs` factors out the existing
  (behavior-unchanged) corner-wall plane construction `standard_walls`
  already did inline, so `standard_curves` can reuse it rather than
  duplicating the math. `PhysicsWorld::standard_arena` picks up the extra 8
  curves automatically, since it already loops over every curve
  `arena::standard_curves()` returns — no changes needed there.
  `FILLET_RADIUS` is reused as-is for the corner-wall fillets rather than
  introducing a second, independently chosen radius (see Verification
  plan). Still not modeled: a car actually being deflected by any fillet
  (unchanged from FR-020), a fillet at a corner wall's own *vertical* edges
  — where it meets its neighboring side/back wall at other than 90 degrees,
  a materially different problem `between_planes` doesn't address, since it
  only handles two perpendicular planes — and goal cutouts (see Non-goals
  and Open questions). 8 new unit tests across `arena.rs`/`world.rs` in
  `rb_physics_bullet` (172 total): `standard_curves` returns exactly 16
  fillets; every fillet's axis sits exactly `FILLET_RADIUS` in from some
  vertical wall, cardinal or corner; a corner wall's own derived fillet
  axis sits exactly `FILLET_RADIUS` in from both the corner wall and the
  floor, with correctly perpendicular unit sector vectors; the cross
  product computing each of the 4 corner walls' `axis_direction` is exactly
  unit length, confirming the production code's `.normalize()`-free
  assumption actually holds; plus — the real end-to-end proof — a new
  `PhysicsWorld` test built around a wall with a diagonal (non-axis-aligned)
  normal, rather than going through `arena::standard_curves` directly,
  confirms a ball resting at ordinary flat-floor height within that
  diagonal wall's fillet footprint gets pushed up off it, the same physical
  proof FR-020 gave for a cardinal wall, now for one whose normal isn't a
  coordinate axis.
- 0.20.0 (2026-08-30): FR-020 added and implemented (curved
  wall-to-floor/wall-to-ceiling transitions) — a new `body::StaticQuarterPipe`
  shape (an immovable partial-cylinder fillet, infinite along its own axis
  like `StaticPlane`) and `collision::contacts_vs_quarter_pipe` (sphere-only
  — a box always returns no contact, deliberately deferred). The playable
  side is the *inside* of the fillet's concave face (the geometry a
  skateboard quarter-pipe is named after and ridden on the inside of): a
  point is governed by a fillet only within the 90-degree sector from
  `sector_start` to `sector_end`, and contact fires as the sphere's surface
  approaches or crosses the fillet's own radius from the inside, pushing
  the sphere back toward the axis — the opposite direction convention from
  `sphere_vs_plane`'s always-away-from-the-plane push.
  `StaticQuarterPipe::between_planes(plane_a, plane_b, radius,
  axis_direction)` derives a fillet's axis/sector automatically from the
  two flat planes it bridges, exact only when both planes' normals and
  `axis_direction` form an orthonormal basis (true for every cardinal
  arena wall's own floor/ceiling seam, not for a diagonal corner wall's).
  `PhysicsWorld` gains `curves: Vec<StaticQuarterPipe>` and a `with_curve`
  builder (mirroring `walls`/`with_wall`), resolved via a new
  `resolve_curve_contact` for the ball and every car (a no-op for cars).
  `solver::resolve_contacts`'s second parameter changed from `&StaticPlane`
  to plain `restitution: f32, friction: f32` — the only two fields it ever
  used — so the same solver path serves a `StaticQuarterPipe` fillet
  exactly as it already served a `StaticPlane`, with no new solver code.
  `arena::standard_curves` builds the 8 fillets (floor-side and
  ceiling-side, for each of the 4 cardinal walls) the standard arena needs,
  using a new uncalibrated placeholder `FILLET_RADIUS` (this port has no
  verified reference for the real transition radius, same status as
  `arena::CORNER_LENGTH`); `PhysicsWorld::standard_arena` now adds these 8
  curves alongside its existing 9 walls. Still not modeled: a car actually
  being deflected by a fillet, fillets at the 4 diagonal corner walls
  (their non-axis-aligned normals don't satisfy `between_planes`'
  orthonormal-basis assumption), and goal cutouts. 15 new unit tests across
  `body.rs`/`collision.rs`/`arena.rs`/`world.rs` in `rb_physics_bullet`
  (168 total): the derived fillet geometry sits exactly `radius` in from
  both bridged planes with correctly-directed, perpendicular unit sector
  vectors and tangent points exactly on each plane; a sphere deep inside a
  fillet has no contact, touching it has zero penetration, pushed past it
  has positive penetration pushing back toward the axis, and outside the
  90-degree sector has no contact regardless of absolute distance; a box
  against a fillet always returns no contact; `standard_curves` returns
  exactly 8 fillets, each sitting radius-in from the floor/ceiling and a
  cardinal wall; `PhysicsWorld::standard_arena` carries exactly 8 curves,
  plus — the real end-to-end proof — a ball resting at ordinary flat-floor
  height within a curve's footprint (already overlapping the fillet's own
  material) gets pushed up off that flat height instead of staying
  embedded, while a car in the exact same position stays completely
  unaffected at its ordinary flat-floor resting height.
- 0.19.0 (2026-08-30): FR-019 added and implemented (modeled arena
  footprint) — a new `arena` module builds Rocket League's real
  standard-arena boundary entirely from FR-013's existing generic
  `StaticPlane`/`with_wall` machinery: no new collision code, since a
  ceiling and a corner-cut wall are each just another flat plane.
  `arena::standard_ground` is the flat floor at `z = 0` (identical to the
  `flat_ground()` test helper this crate has used since v0);
  `arena::standard_walls` returns 9 `StaticPlane`s — 2 side walls
  (`x = ±SIDE_WALL_X`), 2 back walls (`y = ±BACK_WALL_Y`), a ceiling
  (`z = CEILING_Z`), and 4 diagonal corner walls (one per quadrant) cutting
  off the true rectangular corner where a side wall would otherwise meet a
  back wall at 90 degrees, giving the field its real octagonal footprint.
  `SIDE_WALL_X` (4096), `BACK_WALL_Y` (5120), and `CEILING_Z` (2044) are
  commonly-cited community-measured field dimensions, matching the sourcing
  convention `drive::MAX_CAR_SPEED`/`JUMP_SPEED` already established; the
  corner walls' inset distance (`CORNER_LENGTH`, equal along both axes,
  giving a 45-degree cut) is this project's own uncalibrated
  placeholder — this port has no verified reference for the real arena's
  actual corner-wall geometry, which isn't even a single flat plane in the
  real field mesh (it's curved, and blends into ramps this port doesn't
  model either). New `PhysicsWorld::standard_arena` convenience
  constructor wires both into a `PhysicsWorld` in one call — offered
  alongside, not replacing, `PhysicsWorld::new`/`with_wall`'s existing
  ad-hoc-wall capability, which this crate's own tests keep using for
  non-standard scenes. Still not modeled: curved wall-to-floor/
  wall-to-ceiling transitions, goal cutouts in the back walls, and
  disambiguating or blending a car's simultaneous contact with two walls
  at a corner for wall-jump purposes — physical collision resolution
  already handles a car touching two walls at once correctly regardless
  (each wall is resolved independently every step), only the wall-jump
  push-off direction picker still isn't, and FR-019's corner walls make
  that case reachable in the standard arena for the first time (still
  untested here). 10 new unit tests across `arena.rs`/`world.rs` in
  `rb_physics_bullet` (153 total): `standard_walls` returns exactly 9
  planes; the arena's center is on the playable side of every one of them;
  opposing side/back walls share one offset magnitude by construction; a
  point just past a side wall is no longer on the playable side; the
  ceiling bounds from above; a corner wall actually cuts off the true
  rectangular corner; all four corner walls share one offset magnitude,
  plus — the real end-to-end proof — `PhysicsWorld::standard_arena` carries
  exactly 9 walls and the standard ground, a ball shot at the standard
  arena's side wall bounces off it rather than escaping, and a ball fired
  straight at the true rectangular corner is stopped by the diagonal
  corner wall well before its x or y individually reaches either the side
  or back wall's own position.
- 0.18.0 (2026-08-30): FR-018 added and implemented (landing
  auto-orientation assist) — `drive::apply_driven_forces` gains a gentle
  continuous restoring torque, applied while airborne, nudging the car's
  local up axis back toward world up. Real Rocket League triggers this on
  approach to the ground; this port has no raycast or distance query to
  replicate that condition, so the assist instead applies continuously
  whenever airborne, gated on two conditions so it never fights the player:
  no active `pitch`/`roll` air-control input this step, and no fresh
  `ControllerInput.jump` press this step (avoiding a same-step conflict
  between this torque's accumulation into `total_torque` and a
  dodge's/wall-jump-dodge's/double-jump's/flip-cancel's own direct
  `angular_velocity` mutation, both resolved by the same
  `integrate_velocities` call). The correction is
  `up_axis(car).cross(&world_up) * LANDING_AUTO_UPRIGHT_TORQUE`: since both
  vectors are unit length, the cross product's magnitude is already
  proportional to the sine of the car's tilt off level, so a level car
  earns no correction and a heavily tilted one earns a proportionally
  stronger nudge, with no separate angle computation needed. New constant
  `LANDING_AUTO_UPRIGHT_TORQUE` is an uncalibrated placeholder, deliberately
  one full order of magnitude smaller than `AIR_CONTROL_TORQUE` so the
  assist reads as gentle assistance, not full control; this port has no
  public reference for the real assist's actual strength or trigger
  condition either. Known, accepted, unaddressed limitation: a car resting
  exactly upside-down gives an exactly antiparallel `up_axis`/`world_up`
  pair, whose cross product is also zero, so no correction is computed in
  that unlikely exact singularity. No new `PhysicsWorld` state — the assist
  is a pure function of the car's current orientation, input, and ground
  contact, all already in scope. Drive.rs's own test-helper chain never
  calls `integrate::integrate_transform`, so a car's `orientation` never
  actually changes step-to-step there; the new `drive.rs` tests instead set
  a known tilted orientation directly (a new `tilted_car()` helper, calling
  `RigidBody::update_inertia_tensor` afterward for consistency) and check a
  single step's resulting torque, a pattern reusable for any future
  orientation-dependent test there. A pre-existing regression test
  (`world::tests::landing_and_a_new_double_jump_clears_a_stale_dodge_flip_flag_in_a_live_world`) was loosened from an exact `assert_eq!` to a small
  tolerance, since the assist now legitimately nudges angular velocity by a
  tiny amount on the test's intervening neutral-input step — the tolerance
  stays far tighter than a real spurious flip-cancel (which zeroes ~1.5
  rad/s) would need to slip through undetected. 5 new unit tests across
  `drive.rs`/`world.rs` in `rb_physics_bullet` (143 total): a tilted
  airborne car with no input gets a corrective torque; an already-upright
  airborne car gets none; the assist has no effect while grounded; it
  doesn't fire while pitch air control is actively held (isolated via a
  tilt whose own correction axis is orthogonal to full pitch's own torque
  axis); and — the real end-to-end proof — a car tilted 90 degrees with no
  input trends back toward level over 120 steps of a live
  `PhysicsWorld::step` loop (gravity zeroed) rather than staying tilted or
  drifting further away. This closes out the last item that had been
  tracked in `drive.rs`'s own module doc "Not implemented" list since the
  dodge (FR-014) increment — that list is now empty.
- 0.17.0 (2026-08-30): FR-017 added and implemented (wall-jump dodge) —
  the wall jump's own fresh press (FR-013) now checks
  `ControllerInput.pitch`/`roll` against `DODGE_DEADZONE`, the same check
  the ground double jump's press already uses (FR-014): at or above it on
  either axis, a **wall-jump dodge** fires instead of the plain fixed
  push-off — the same outward-plus-upward impulse combined with a
  horizontal `DODGE_SPEED` component and `DODGE_ANGULAR_SPEED` spin
  (identical axis/sign conventions to the ground dodge), also arming
  `dodge_flip_active` so its spin is flip-cancelable (FR-016) exactly like
  a ground dodge's. Below the deadzone, the plain wall jump fires exactly
  as before, still never touching `double_jump_available`. Unlike the
  plain wall jump, the dodge variant *does* consume `double_jump_available`
  — the same resource a ground dodge spends. This is a deliberate
  simplification: since touching a wall unconditionally restores
  `double_jump_available` before this check ever runs, gating the dodge
  variant on it would be vacuous (always true there); having the dodge
  variant spend it instead keeps the existing invariant
  "`dodge_flip_active` is only ever true while `double_jump_available` is
  false" intact with zero changes to flip-cancel's own branch ordering or
  any new landing/wall-touch-clearing logic — this port has no way to
  separately account for "a wall touch refilled the double jump, then the
  wall-jump dodge spent it" versus a genuinely independent wall-dash
  resource, and real Rocket League's precise accounting here isn't public
  to the precision this project would need to model that distinction. No
  new physics constants — reuses `DODGE_SPEED`/`DODGE_ANGULAR_SPEED`/
  `WALL_JUMP_HORIZONTAL_SPEED`/`JUMP_SPEED` outright. Two pre-existing
  tests whose entire premise this requirement deliberately reverses
  (`drive::wall_jump_fires_instead_of_a_dodge_when_touching_a_wall`,
  `world::wall_jump_still_fires_instead_of_a_dodge_when_touching_a_wall`,
  both of which asserted "wall jump always ignores stick input") were
  repurposed in place — not silently deleted — to assert the new
  wall-jump-dodge behavior instead, keeping the same scenario (touching a
  wall with directional stick input) but updating the expected outcome. 6
  new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet` (138
  total): a wall-jump dodge consumes the double jump unlike a plain wall
  jump; its spin can be flip-cancelled; a below-deadzone stick deflection
  still gives a plain wall jump; opposite stick sign dodges the opposite
  direction; a diagonal (pitch+roll) wall-jump dodge combines both axes,
  plus — the real end-to-end proof — a wall-jump dodge firing in a live
  `PhysicsWorld::step` loop, and a second end-to-end test confirming its
  spin is flip-cancelable there too.
- 0.16.0 (2026-08-30): FR-016 added and implemented (flip-cancel) — a
  dodge's spin (FR-014) can now be canceled early: a further fresh
  `ControllerInput.jump` press while airborne, not touching a wall, with
  `double_jump_available` already spent by that dodge, zeroes
  `RigidBody.angular_velocity` outright instead of leaving the flip to spin
  indefinitely. A new per-car `dodge_flip_active: bool`
  (`PhysicsWorld`'s parallel `car_dodge_flip_active: Vec<bool>`, starting
  `false`) tracks this: the directional-dodge branch sets it `true`; the
  plain-double-jump branch explicitly sets it `false` rather than leaving
  it alone, closing off a real staleness bug this port's own regression
  tests were written to catch and did catch (verified by temporarily
  removing the fix and confirming both the `drive.rs` and `world.rs`
  regression tests fail without it) — without that explicit clear, a
  much-later, completely unrelated plain double jump (after landing from
  the dodge and taking off again) would leave the flag `true`, letting a
  further press spuriously flip-cancel a flip that no longer exists.
  Flip-cancel touches neither the dodge's own linear velocity nor
  `double_jump_available` (already spent by the dodge that set the flag).
  Wall jump keeps its existing priority — checked first in the airborne
  branch, unchanged. This port has no timed flip animation to interrupt (a
  dodge is one instantaneous angular-velocity kick, not a sustained torque
  over a fixed duration), so "mid-flip" here means "any time before
  landing or a wall touch re-arms the double jump," a documented
  simplification of real Rocket League's actual flip-duration window. No
  new physics constants — this is a state-flag-gated zeroing action, not a
  magnitude to calibrate. 6 new unit tests across `drive.rs`/`world.rs` in
  `rb_physics_bullet` (132 total): a second jump press cancels a dodge's
  spin outright and spends the flag; flip-cancel leaves the dodge's own
  translation and `double_jump_available` untouched; a plain double jump
  clears a stale `dodge_flip_active` left over from an earlier dodge so a
  later press can't spuriously cancel nothing; a wall jump still takes
  priority over flip-cancel when touching a wall; an end-to-end test
  confirms a second jump press cancels a dodge's spin in a live
  `PhysicsWorld::step` loop; a second end-to-end regression test confirms
  landing and a later plain double jump clear a stale flag there too, not
  just in `drive.rs` isolation. Deliberately excludes a dodge variant of
  the wall jump and landing auto-orientation assistance — see Non-goals.
- 0.15.0 (2026-08-30): FR-015 added and implemented (variable jump
  height) — the ground jump (FR-010) gains a hold window: continuing to
  hold `ControllerInput.jump` after the fresh press that fires it adds a
  continuous `JUMP_HOLD_ACCELERATION` upward force, for up to
  `JUMP_HOLD_MAX_DURATION` seconds, on top of the press's own fixed
  `JUMP_SPEED` impulse. A new per-car `jump_hold_time_remaining: f32`
  (`PhysicsWorld`'s parallel `car_jump_hold_time_remaining: Vec<f32>`,
  starting `0.0`, threaded into `apply_driven_forces` and
  `drive_and_integrate_velocities` alongside `jump_held`/
  `double_jump_available`) is checked and decremented against the
  *previous* call's value at the very top of `apply_driven_forces`, before
  that same call's own `on_ground`/`jump_pressed` handling can re-arm it to
  `JUMP_HOLD_MAX_DURATION` — so a fresh ground-jump press's own step always
  fires only the plain impulse, and only continued holding into later
  calls earns the extra height. Releasing `jump` zeroes the window
  immediately, even with time left, stopping the extra acceleration right
  away. Scoped to the ground jump alone: firing the double jump, a dodge,
  or the wall jump all require releasing jump first (a fresh press), which
  itself unconditionally zeroes the ground jump's hold window before that
  press's own branch ever runs, so none of the three can be boosted by a
  leftover hold window. `JUMP_HOLD_MAX_DURATION` and
  `JUMP_HOLD_ACCELERATION` are both uncalibrated placeholders — this port
  has no public reference for real Rocket League's actual hold-window
  length or acceleration the way `JUMP_SPEED` does. The pre-existing
  `holding_jump_does_not_repeatedly_relaunch_the_car` regression test's run
  duration was extended (1.5s → 3.0s) since a continuously held jump now
  also earns the variable-height bonus, climbing higher and taking longer
  to land than a bare `JUMP_SPEED` impulse alone. 6 new unit tests across
  `drive.rs` and `world.rs` in `rb_physics_bullet` (126 total): holding
  jump after a ground jump adds more upward velocity than tapping it,
  releasing jump early stops the extra acceleration immediately, the extra
  acceleration stops accruing once the hold window has expired even if
  still held, and a double jump fired after holding the ground jump
  through its whole window still adds exactly one more `JUMP_SPEED` kick
  rather than an extra variable-height boost; an end-to-end test confirms
  a held ground jump reaches a greater peak height than a tapped one in a
  live `PhysicsWorld::step` loop, and a second end-to-end regression test
  confirms the double-jump-unaffected property holds there too, not just
  in `drive.rs` isolation.
- 0.14.0 (2026-08-30): FR-014 added and implemented (dodge) — the double
  jump's own fresh press (see FR-012) now checks `ControllerInput.pitch`/
  `roll` at the moment it fires: at or above a new `DODGE_DEADZONE` on
  either axis, it fires a directional dodge instead of the plain vertical
  double jump — a purely horizontal `DODGE_SPEED` impulse (along
  `forward_axis` for `pitch`, `right_axis` for `roll`) plus an
  instantaneous `DODGE_ANGULAR_SPEED` spin added directly to
  `RigidBody.angular_velocity` (mirroring how `apply_impulse` already
  directly changes `linear_velocity`, rather than `apply_torque`'s
  continuous accumulation, since `RigidBody` has no "angular impulse"
  helper and none was warranted for this one call site) about the
  perpendicular axis (`right_axis` for `pitch`, `forward_axis` for
  `roll`) — reusing air control's own pitch/roll axis and sign
  conventions for direction, though not its `AIR_CONTROL_TORQUE`
  magnitude. Both axes can contribute at once (a diagonal dodge), simply
  summed rather than normalized — a documented simplification, since real
  Rocket League normalizes the stick direction so a diagonal dodge isn't
  faster than an axis-aligned one. Below `DODGE_DEADZONE` on both axes, the
  plain vertical double jump fires exactly as before this requirement;
  either way the press still spends the shared `double_jump_available`
  resource. Wall jump is untouched — it never checks `pitch`/`roll` at
  all, so touching a wall always gets the fixed wall-jump push-off, never
  a dodge. `DODGE_SPEED` and `WALL_JUMP_HORIZONTAL_SPEED` are now `pub`
  (mirroring `JUMP_SPEED`) so `world.rs`'s end-to-end tests can assert
  against, and distinguish between, all three jump variants' distinct
  magnitudes. Deliberately excludes a dodge variant of the wall jump,
  canceling a dodge's rotation early (flip-cancel), landing
  auto-orientation assistance, and variable jump height — see Non-goals.
  10 new unit tests across `drive.rs` and `world.rs` in `rb_physics_bullet`
  (120 total): a forward (pitch) dodge and a lateral (roll) dodge each
  give the expected horizontal velocity and spin, a below-deadzone
  deflection still gives a plain double jump, a dodge spends
  `double_jump_available` the same as a plain double jump, opposite pitch
  dodges the opposite direction, a diagonal (pitch+roll) dodge combines
  both axes, dodge logic has no effect while grounded, and a wall jump
  still fires its own (smaller) push-off instead of a dodge when touching
  a wall; an end-to-end test confirms a car dodges forward with a visible
  flip after a ground jump in a live `PhysicsWorld::step` loop, and a
  second end-to-end test confirms a car touching a wall with directional
  stick input still gets the wall jump, not a dodge.
- 0.13.0 (2026-08-30): FR-013 added and implemented (arena walls and wall
  jump) — `PhysicsWorld` gains `walls: Vec<StaticPlane>` and a `with_wall`
  builder (mirroring `with_car`); `resolve_ground_contact` is renamed
  `resolve_plane_contact` (no behavior change — it already had no
  ground-specific logic, only a ground-specific name) and is now called
  once per wall in addition to the ground, for both the ball and every
  car, so arena walls are real physical geometry every body collides with,
  not just an input-detection hack. On top of that, `drive::apply_driven_forces` gains a `wall_normal: Option<Vec3>` parameter (a per-step fact
  computed by `PhysicsWorld` the same way `on_ground` is, not `&mut` state)
  and a wall jump: a fresh airborne `jump_pressed` press while touching a
  wall fires an impulse combining a new `WALL_JUMP_HORIZONTAL_SPEED`
  (uncalibrated placeholder) outward along the wall's normal with
  `JUMP_SPEED` upward, checked before the double jump so it takes priority
  on that press. Wall contact — independent of whether jump is pressed —
  unconditionally restores `double_jump_available`, the same "any surface
  contact refills your second jump" rule landing already uses, so wall
  jump doesn't cost a player their double jump and has no
  once-per-airborne-period limit of its own (unlike the double jump).
  Deliberately excludes the directional "dodge" a real wall jump can pair
  with, variable jump height, and any modeled arena footprint beyond
  generic flat walls (no octagonal shape, curved transitions, ceiling, or
  multi-wall-corner disambiguation) — see Non-goals. 7 new unit tests
  across `drive.rs` and `world.rs` in `rb_physics_bullet` (110 total):
  wall jump gives outward-and-upward velocity when available, has no
  effect while grounded, takes priority over the double jump without
  consuming it, and mere wall contact restores double-jump availability;
  an end-to-end test confirms a car resting against a wall wall-jumps
  outward and upward in a live `PhysicsWorld::step` loop; a second
  end-to-end test confirms a ball shot at a wall bounces off it instead of
  tunnelling through — the same physical proof already given for
  ball-vs-car, now for the generic plane-collision machinery walls reuse;
  and a regression test confirms a car near, but not touching, an existing
  wall still gets a plain double jump instead of a wall jump.
- 0.12.0 (2026-08-29): FR-012 added and implemented (double jump) —
  `drive::apply_driven_forces` fires one more, identical `JUMP_SPEED`
  instantaneous upward velocity change on a fresh airborne press of
  `ControllerInput.jump`, reusing the ground jump's own rising-edge
  detection (`jump_pressed`) and the `JUMP_SPEED` constant itself rather
  than introducing a second edge-detector or a separately-calibrated
  speed. Gated on a new per-car `double_jump_available` flag: landing
  unconditionally restores it, and a fresh airborne press that spends it
  sets it back to `false` until the next landing, so it fires at most once
  per airborne period regardless of how many more times jump is released
  and re-pressed before then. `PhysicsWorld` gains a parallel
  `car_double_jump_available: Vec<bool>` (starting `true`, kept in
  lockstep with `cars` by `with_car`), threaded through
  `drive_and_integrate_velocities` and `step`'s per-car loop alongside
  `jump_held`. Deliberately excludes the directional "dodge" impulse/torque
  a real double jump pairs with, variable jump height, and wall jump — see
  Non-goals. `JUMP_SPEED` is now `pub` so `world.rs`'s end-to-end tests can
  assert against it directly. 6 new unit tests across `drive.rs` and
  `world.rs` in `rb_physics_bullet`, minus one pre-existing `drive.rs`
  test — `jump_has_no_effect_while_airborne` — removed because this
  feature deliberately supersedes its premise (a fresh airborne jump press
  can now have an effect); net +5, 103 total, including an end-to-end
  test confirming a double jump fired after a ground jump adds a second
  `JUMP_SPEED` kick on top of the first in a live `PhysicsWorld::step` loop
  (gravity zeroed), and a regression test confirming a spent double jump
  doesn't refire mid-air no matter how many more times jump is released and
  re-pressed before landing.
- 0.11.0 (2026-08-29): FR-011 added and implemented (air control) —
  `drive::apply_driven_forces` applies torque about the car's local
  right/up/forward axes, scaled by `ControllerInput.pitch`/`yaw`/`roll`
  (each an `Option<f32>`, `None` treated as zero) times one shared
  `AIR_CONTROL_TORQUE` constant, gated on the car *not* touching the
  ground — the mirror image of throttle/steering/handbrake/jump's
  ground-only gating. Unlike ground steering, not speed-scaled: a car can
  spin from a standing start in the air. New `right_axis` helper completes
  the local (forward, right, up) basis `forward_axis`/`up_axis` already
  provided. `AIR_CONTROL_TORQUE` is a shared, uncalibrated placeholder
  across all three axes — a documented simplification, since real Rocket
  League's pitch/yaw/roll rates differ from each other. Double jump/dodge,
  variable jump height, and wall jump remain explicitly not implemented —
  see Non-goals. 6 new unit tests across `drive.rs` and `world.rs` in
  `rb_physics_bullet` (98 total), including an end-to-end test confirming
  a car with yaw input actually reorients itself mid-air (gravity zeroed)
  in a live `PhysicsWorld::step` loop, and a regression test confirming a
  grounded car stays level despite stray pitch/yaw/roll input.
- 0.10.0 (2026-08-29): FR-010 added and implemented (single ground jump) —
  `drive::apply_driven_forces` applies a fixed `JUMP_SPEED` instantaneous
  upward velocity change (via `RigidBody::apply_impulse`) on the rising
  edge of `ControllerInput.jump` while the car is grounded — a fresh
  press, not merely held; a continued press through the resulting
  airborne period doesn't re-fire it, and releasing then re-pressing while
  still airborne doesn't fire it either (no double jump in this scope).
  `PhysicsWorld` gains a parallel `car_jump_held: Vec<bool>` (starting
  `false`, kept in lockstep with `cars` by `with_car`) carrying the
  rising-edge state across steps, the same pattern `boost_amount` already
  uses. Double jump/dodge, variable jump height (holding for a higher
  jump), wall jump, and air control remain explicitly not implemented —
  see Non-goals. 6 new unit tests across `drive.rs` and `world.rs` in
  `rb_physics_bullet` (92 total), including an end-to-end test confirming
  a car with jump input actually leaves the ground in a live
  `PhysicsWorld::step` loop, and a regression test confirming that holding
  jump for a car's entire flight (never released) lets it land and settle
  instead of being relaunched on touchdown.
- 0.9.0 (2026-08-29): FR-009 added and implemented (handbrake) —
  `drive::apply_driven_forces` temporarily multiplies the car's
  `RigidBody.friction` by a new `HANDBRAKE_FRICTION_MULTIPLIER`
  (uncalibrated placeholder) while `ControllerInput.handbrake` is held and
  the car is grounded, restoring it otherwise — modeling handbrake as a
  temporary grip reduction that lets existing momentum carry the car into
  a slide, reusing the ground-contact solver's existing friction machinery
  rather than a new lateral-slip system (this port has no per-wheel tire
  model to build a real rear-grip-loss mechanic on). `PhysicsWorld` gains
  a parallel `car_base_friction: Vec<f32>`, snapshotted from each car's own
  constructed `friction` by `with_car`, so handbrake restores the car's
  own value rather than a hardcoded default. Jump and air control remain
  explicitly not implemented — see Non-goals. 5 new unit tests across
  `drive.rs` and `world.rs` in `rb_physics_bullet` (86 total), including
  an end-to-end test confirming a car already sliding sideways retains
  more of that slide under handbrake's reduced friction than under normal
  grip in a live `PhysicsWorld::step` loop, and a regression test
  confirming handbrake restores a car's own non-default base friction, not
  a crate-wide constant.
- 0.8.0 (2026-08-29): FR-008 added and implemented (boost) —
  `drive::apply_driven_forces` gains a boost force: a flat forward force
  (`BOOST_ACCELERATION * mass`, not speed-tapered like throttle) along the
  car's local forward axis, applied whenever `ControllerInput.boost` is set
  and the car has boost remaining, capped at `MAX_CAR_SPEED`. Unlike
  throttle and steering, boost is *not* gated on ground contact — it works
  identically airborne, matching real Rocket League's rocket-based (not
  wheel-based) boost. `PhysicsWorld` gains a parallel `car_boost: Vec<f32>`
  (kept in lockstep with `cars` by `with_car`, initialized to a full tank —
  `drive::MAX_BOOST`) and `set_car_boost` to set it directly; holding boost
  drains the tank at `BOOST_CONSUMPTION_RATE` per second whenever held,
  even once the forward force itself stops applying at `MAX_CAR_SPEED`
  (matching real Rocket League's "holding boost drains fuel regardless");
  the tank clamps at zero. `frame()` now reports each car's live
  `boost_amount` instead of a hardcoded `0.0`. Jump, air control, and
  handbrake remain explicitly not implemented — see Non-goals. 6 new unit
  tests across `drive.rs` and `world.rs` in `rb_physics_bullet` (81 total),
  including an end-to-end test confirming a car with boost input actually
  drives forward while airborne (gravity zeroed) in a live
  `PhysicsWorld::step` loop, and a regression test confirming a new car
  starts with a full boost tank.
- 0.7.0 (2026-08-29): FR-007 added and implemented (ground throttle and
  steering only) — new `drive` module, `apply_driven_forces` couples
  `rb_domain::ControllerInput` into a throttle force (along the car's
  local forward axis, capped at `MAX_CAR_SPEED`) and a steering torque
  (about the car's local up axis, scaled by current speed), both gated on
  ground contact. `PhysicsWorld` gains `car_inputs: Vec<ControllerInput>`
  (kept in lockstep with `cars` by `with_car`, defaulting to neutral) and
  `set_car_input` to update a car's persistent input; `step` computes each
  car's ground-contact state up front and applies its driven forces
  alongside gravity, before integrating velocities; `frame()` now reports
  each car's actual input instead of always `None`. Boost, jump, air
  control, and handbrake remain explicitly not implemented — see
  Non-goals. 10 new unit tests in `rb_physics_bullet` (75 total),
  including an end-to-end test confirming a car with throttle input
  actually drives forward across the ground in a live `PhysicsWorld::step`
  loop, and a regression test confirming a car with no input set behaves
  exactly as before this requirement existed.
- 0.6.0 (2026-08-29): Multi-car `PhysicsWorld` support — `car: Option<RigidBody>` is replaced by `cars: Vec<RigidBody>` (a breaking
  field rename); `with_car` now appends, so it's callable any number of
  times to build a scene with any number of cars. `PhysicsWorld::step`
  resolves every car's ground contact, every ball-vs-car pair, and every
  car-vs-car pair (via `collision::box_vs_box`, now running for real in a
  live scene instead of only under a unit test) each step, one pair at a
  time; `frame()` assigns each car's `player_id` as its index in `cars`.
  This completes `RB-PHYSICS-001-FR-006` — car-vs-car collision detection
  (0.5.0) is now actually wired up, not just unit-tested in isolation. 3
  new unit tests in `rb_physics_bullet` (65 total), including an
  end-to-end test confirming two cars shot head-on at each other in a live
  `PhysicsWorld` actually bounce off instead of tunnelling through.
- 0.5.0 (2026-08-29): FR-006 added and implemented (detection only) —
  `collision::box_vs_box`, a 15-axis separating-axis test (3+3 face axes,
  9 edge-pair axes) between two oriented boxes, producing a clipped face
  manifold (`face_contact`, 0-4 points) or a single edge-edge point
  (`edge_contact`, via a standard closest-point-between-segments
  construction). `collision::contact_between` is generalized to
  `contacts_between` (returning `Vec<Contact>` uniformly, since box-vs-box
  can now return a manifold where sphere-vs-box always returned at most
  one point), and `solver::resolve_contact_between` is generalized to
  `resolve_contacts_between` (a manifold, mirroring `resolve_contacts`'
  existing multi-contact structure) to match. `box_vs_box` has no live
  caller through `PhysicsWorld` yet — this scope still has exactly one
  car, so multi-car wiring is deliberate, tracked follow-up work, not this
  change's scope (see Non-goals). 4 new unit tests in `rb_physics_bullet`
  (62 total).
- 0.4.0 (2026-08-28): FR-004 completed — sphere-vs-box (ball-vs-car)
  contact generation (`collision::sphere_vs_box`/`contact_between`,
  handling both the ordinary exterior case and a deep-penetration interior
  case) and a two-dynamic-body sequential-impulse solver path
  (`solver::resolve_contact_between`, generalizing the existing
  body-vs-static-plane rows to carry both bodies' mass/inertia
  contributions). `PhysicsWorld::step` was restructured into Bullet's
  actual staged pipeline (integrate every body's velocity, then resolve
  every contact, then integrate every body's transform) so ball-vs-car
  resolution sees the same pre-integration state ground contacts do.
  `rb_domain::Quat` gains `conjugate` (needed to transform a world point
  into the box's local frame). Box-vs-box collision remains explicitly not
  implemented — see Non-goals. 11 new unit tests in `rb_physics_bullet`
  (58 total), 1 in `rb_domain`.
- 0.3.0 (2026-08-28): FR-004 substantially implemented — box-shaped
  bodies via a unified `RigidBody`/`Shape` design (matching Bullet's own
  rigid-body-plus-collision-shape architecture), a general 3x3 inverse
  inertia tensor (`Mat3`, shared by sphere and box), analytic box-vs-plane
  contact generation (1-4 points), and multi-contact manifold resolution
  in the solver. `PhysicsWorld` gains an optional car body stepped
  alongside the ball. Box-vs-sphere collision remains explicitly not
  implemented — see Non-goals. 21 new unit tests (47 total).
- 0.2.0 (2026-08-28): v0 implemented — sphere-vs-static-plane rigid body
  integration and sequential-impulse contact solver, ported from Bullet3
  per ADR-0004. Resolves the "build-vs-integrate" framing from this spec's
  0.1.0 open questions in favor of a direct source port.
- 0.1.0 (2026-08-28): Placeholder created at bootstrap; full spec deferred
  to Phase 1 start.
