# RB-PHYSICS-001 — Physics Core Port

- Version: 0.22.0
- Status: In Progress (sphere-vs-plane, box-vs-plane, sphere-vs-box
  (ball-vs-car), box-vs-box (car-vs-car), body-vs-arena-wall, and
  ball-vs-curved-fillet collision all implemented, tested, and wired into a
  real N-body `PhysicsWorld` scene; ground-driving car input (throttle,
  steering), boost, handbrake, a variable-height ground jump, a double jump
  (plain or a directional dodge, itself flip-cancelable), a wall jump
  (itself dodgeable), air control (pitch/yaw/roll), a gentle landing
  auto-orientation assist, a modeled arena footprint
  (`PhysicsWorld::standard_arena`'s octagonal boundary plus a ceiling), and
  curved fillets throughout the arena's vertical boundary — wall-to-floor/
  wall-to-ceiling seams for all 9 walls (the 4 cardinal walls and, since
  FR-021, the 4 diagonal corner walls too) plus, since FR-022, all 8 of the
  corner walls' own vertical edges where they meet their neighboring
  side/back walls — implemented; a car (box) actually being deflected by a
  curve, goal cutouts, split impulse, warm-starting, a combined multi-body
  solve, and constant calibration are open follow-up work)
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
`PhysicsWorld` carries `cars: Vec<RigidBody>` and resolves every
ball-vs-car and car-vs-car pair each step, so `box_vs_box` now runs for
real in a live scene, not just in isolation under a unit test. Each car
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
too), plus, since FR-022, all 8 of the corner walls' own vertical edges
where they meet their neighboring side/back walls — a `StaticQuarterPipe`
fillet each deflecting the ball (not yet a car) away from the sharp edge a
flat wall and the floor/ceiling, or two flat walls, would otherwise meet at
— see FR-020/FR-021/FR-022.

## Non-goals (this increment)

- **A combined multi-body solve.** Each ball-vs-car and car-vs-car pair is
  resolved independently, one full `SOLVER_ITERATIONS` pass at a time, not
  as a single simultaneous solve across every contact touching in the same
  step. This is a real approximation once 3+ bodies are mutually touching
  at once (e.g. a car pinned between the ball and another car) — Bullet's
  actual solver interleaves constraints across an entire "island" of
  touching bodies together. See `world::step`'s doc comment and Open
  questions.
- **Team structure, car limits, or any Rocket-League-specific scene
  policy.** `with_car` can be called any number of times; this crate
  itself imposes no cap (Rocket League's real max is 8, but that's a
  gameplay/matchmaking rule, not a physics-core one) and has no concept of
  teams — a caller (eventually `rb_verify_cli`, once real multi-car
  recorded data exists) owns that policy.
- **A car (box) actually being deflected by a curved fillet, goal cutouts,
  and any geometry finer than a flat plane or single-radius fillet per
  boundary segment.** `arena::standard_curves` builds 24 `StaticQuarterPipe`
  fillets — 16 floor/ceiling-seam fillets (one floor-side and one
  ceiling-side per wall, for all 9 walls including the 4 diagonal corner
  walls since FR-021) plus, since FR-022, 8 vertical-edge fillets (one per
  corner wall endpoint, where it meets its neighboring side/back wall) —
  deflecting only the ball; `collision::contacts_vs_quarter_pipe` returns no
  contact at all for a box, so a car drives straight through any curve's
  footprint completely unaffected, exactly as if the curve weren't there
  (see FR-020's own Non-goals note). The back walls have no goal-shaped
  cutout. `FR-019`'s corner-cut inset distance (`arena::CORNER_LENGTH`) and
  `FR-020`'s fillet radius (`arena::FILLET_RADIUS`, also reused by FR-021
  and FR-022) are both this project's own uncalibrated
  placeholders, not measured against real field mesh data — only
  `SIDE_WALL_X`/`BACK_WALL_Y`/`CEILING_Z` are commonly-cited, sourced
  dimensions.
- **Disambiguating or blending a car's simultaneous contact with two walls
  at a corner, for wall-jump purposes.** Physical collision resolution
  already handles a car touching two walls at once correctly — `step`
  resolves every wall independently, so both contacts are resolved on the
  same step regardless of the arena's shape (see FR-013). What's still not
  disambiguated is *which* wall's normal `drive::apply_driven_forces` uses
  to decide a wall jump's push-off direction when a car is touching more
  than one wall at a corner at once: it picks whichever wall comes first in
  `PhysicsWorld.walls`, not a blend of the two normals — a documented
  simplification for a case this port's test scenes don't exercise (FR-019's
  new corner walls make this case reachable in the standard arena for the
  first time, but still untested here).
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
- **Warm-starting and sleeping.** Every contact's impulses are re-derived
  from zero each frame. Documented consequence: a bouncy (restitution > 0)
  resting contact never truly settles under v0's solver — see
  `rb_physics_bullet::solver`'s module doc and
  `world::tests::resting_ball_stays_at_rest`.
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
  pre-existing tests (`drive::wall_jump_fires_instead_of_a_dodge_when_
  touching_a_wall`, `world::wall_jump_still_fires_instead_of_a_dodge_when_
  touching_a_wall`) asserted the *old* "wall jump always ignores stick
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
  (4096), `BACK_WALL_Y` (5120), and `CEILING_Z` (2044) are commonly-cited
  community-measured field dimensions (the same sourcing convention as
  `drive::MAX_CAR_SPEED`/`JUMP_SPEED`); the corner walls' inset distance
  (`CORNER_LENGTH`, equal along both axes, giving a 45-degree cut) is this
  project's own uncalibrated placeholder — this port has no verified
  reference for the real arena's actual corner-wall geometry, which isn't
  even a single flat plane in the real field mesh (it's curved, and blends
  into ramps this port doesn't model either). `PhysicsWorld::standard_arena`
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
  along its own axis like `StaticPlane` — and `collision::
  contacts_vs_quarter_pipe`, a sphere-only narrow-phase test (a box/car
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
  `sphere_vs_plane`'s always-away-from-the-plane push. `StaticQuarterPipe::
  between_planes(plane_a, plane_b, radius, axis_direction)` derives a
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
  every car (a no-op for cars, since the box arm of `contacts_vs_quarter_pipe`
  is always empty). `solver::resolve_contacts`'s second parameter changed
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
  fillet, fillets at the 4 diagonal corner walls (now implemented, see
  FR-021), and goal cutouts (see Non-goals).
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
  it, rather than duplicating the corner-wall plane math. `PhysicsWorld::
  standard_arena` picks up the extra 8 curves automatically, since it
  already loops over every curve `arena::standard_curves()` returns. Still
  not modeled at the time this requirement shipped: a car actually being
  deflected by any fillet (unchanged from FR-020), a fillet at a corner
  wall's own *vertical* edges (now implemented, see FR-022), and goal
  cutouts (see Non-goals).
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
  deflected by any fillet, and goal cutouts (see Non-goals) — the
  compound corner where a vertical-edge fillet meets a floor- or
  ceiling-seam fillet, near a wall's own endpoint, also remains unaddressed
  (this port doesn't attempt a blended 3D corner fillet, only independent,
  additive per-edge ones — see Non-goals).
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
- `integrate`: force accumulation, velocity integration, transform
  integration — pure functions over `RigidBody`, shape-agnostic.
- `collision`: `contacts_vs_plane` — analytic body-vs-static-plane contact
  generation (any plane, not just the ground — an arena wall is the exact
  same test with a different normal), dispatching to a sphere- or
  box-specific test and returning a manifold (`Vec<Contact>`, 0 to 4
  points); `contacts_vs_quarter_pipe` (since FR-020) — analytic
  sphere-vs-fillet contact generation (always 0 or 1 points; a box always
  returns none, see FR-020's Non-goals); `contacts_between` — dispatches to
  `sphere_vs_box` (0 or 1 points) or the separating-axis `box_vs_box` (0 to
  4 points), covering every two-dynamic-body shape pairing this crate has.
- `solver`: `resolve_contacts` — sequential-impulse contact + friction
  resolution over an entire manifold against one static body, identified
  only by its `restitution`/`friction` (since FR-020, this serves a
  `StaticQuarterPipe` fillet exactly as it already served a `StaticPlane` —
  the static shape's actual geometry is irrelevant here, already baked into
  the caller's own `Contact` list); `resolve_contacts_between` — the same
  sequential-impulse math generalized to two dynamic bodies' shared contact
  manifold.
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
  no fresh jump press this step — a gentle continuous `LANDING_AUTO_
  UPRIGHT_TORQUE`-scaled restoring torque nudging the car's local up axis
  back toward world up (not a Bullet3 port — this project's own model of
  Rocket League's driving mechanics, since the real numbers aren't public;
  see the module's own doc comment for which constants are commonly-cited
  community estimates vs. uncalibrated placeholders).
- `world`: `PhysicsWorld::step`/`frame`, and `simulate()` — the
  composition root Bullet's `btDiscreteDynamicsWorld::stepSimulation`
  corresponds to, run in the same staged order (integrate every body's
  velocity — for cars, including `drive::apply_driven_forces` — then
  resolve every contact — ground, every wall, and every curve for every
  body, every ball-vs-car pair, then every car-vs-car pair — then integrate
  every body's transform). `PhysicsWorld` carries one ball (`RigidBody`,
  always present), `walls: Vec<StaticPlane>` (any number, via repeated
  `with_wall` calls, empty by default), `curves: Vec<StaticQuarterPipe>`
  (since FR-020; any number, via repeated `with_curve` calls, empty by
  default — only ever deflects the ball, a no-op for every car), and
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
  standard-arena field dimensions and a 9-`StaticPlane` octagonal boundary
  plus ceiling, built from `body::StaticPlane` alone (no new collision
  code); `standard_curves` (since FR-020, extended to all 9 walls'
  floor/ceiling seams by FR-021, and to the 8 corner-wall vertical edges by
  FR-022) — 24 `StaticQuarterPipe` fillets total: 16 floor-side/
  ceiling-side fillets (one pair per wall, all 9 walls including the 4
  diagonal corner walls since FR-021) plus 8 vertical-edge fillets (one per
  corner-wall endpoint, since FR-022), all built via
  `StaticQuarterPipe::between_planes` from those same flat planes — a
  corner wall's own floor/ceiling-seam `axis_direction` is computed via a
  cross product rather than hand-picked, since (unlike a cardinal wall's)
  it isn't a coordinate axis, while a vertical-edge fillet's own
  `axis_direction` is simply `(0, 0, 1)` (the edge itself is vertical);
  `PhysicsWorld::standard_arena` (in `world`)
  wires all three into a new `PhysicsWorld` in one call, an alternative to
  `PhysicsWorld::new` plus manual `with_wall`/`with_curve` calls for a
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
  through — the same physical proof `ball_bounces_off_a_stationary_car_
  instead_of_passing_through` already gives for cars, now for the generic
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
  deliberately reverses (`drive::wall_jump_fires_instead_of_a_dodge_when_
  touching_a_wall`, `world::wall_jump_still_fires_instead_of_a_dodge_when_
  touching_a_wall`) were repurposed, not silently deleted, to assert the
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
  test (`landing_and_a_new_double_jump_clears_a_stale_dodge_flip_flag_in_a_
  live_world`) was loosened from exact equality to a small tolerance, since
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
  this version).
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
how many real cars are ever mutually touching at once (this port's
one-pair-at-a-time solve, see Non-goals, is untested against that).
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
Open questions). The modeled arena footprint's `SIDE_WALL_X`/`BACK_WALL_Y`/
`CEILING_Z` are, like `MAX_CAR_SPEED`/`JUMP_SPEED`, commonly-cited
community-measured field dimensions this project hasn't independently
confirmed; `CORNER_LENGTH` (the octagon corner-cut inset) is this port's
own uncalibrated invention with no public reference at all, and unlike
every other constant in this crate, the real quantity it approximates
(the field mesh's actual corner geometry) isn't even a single flat plane
to begin with — so this one constant can't converge toward a "correct"
value through calibration alone the way a scalar speed or torque could;
matching the real corner shape would need genuinely different (curved)
collision geometry, not just a better number (see FR-019 and Open
questions). FR-020's `arena::FILLET_RADIUS` has exactly the same status as
`CORNER_LENGTH` — this port's own invention, no public reference, and only
governs the ball (see FR-020's own Non-goals: a car isn't deflected by a
curve at all yet, so there's nothing to validate there either). FR-021's
and FR-022's own fillets reuse this same `FILLET_RADIUS` constant rather
than introducing a second one each — a documented simplification, since
this port has no reason to believe the real game's corner-wall transition
radius (if it even uses one uniform radius, which the actual field mesh's
curved corners likely don't) matches its cardinal-wall radius, and even
less reason to believe a vertical-edge fillet's own radius should match
either — FR-022's own edges are visibly shallower (45 degrees) than a
floor/ceiling seam's (90), yet share the same radius regardless. The unit
tests confirm the fillet's *shape* of response (pushes back toward the
axis once the sphere's surface crosses the fillet's own radius from
inside, respects its own sector — 90 degrees for a floor/ceiling seam, 45
for a corner wall's own vertical edge — leaves a box untouched), not that a
real ball's actual wall-to-floor/wall-to-ceiling or wall-to-wall transition
behavior matches this radius or trigger condition, at a cardinal wall, a
corner wall's floor/ceiling seam, or a corner wall's own vertical edge
alike. `StaticQuarterPipe::between_planes`'s own generalization (FR-022) —
solving the axis point as a real 2x2 linear system, and testing sector
membership via signed cross products rather than the old
perpendicular-only two-dot-products shortcut — is itself unit-tested
directly against a synthetic non-perpendicular fixture, independent of the
arena's own geometry, so its correctness doesn't rest solely on the
corner-wall numbers happening to work out.

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- A combined multi-body solve for bodies simultaneously touching more than
  one other body (see Non-goals) — needs real recorded multi-car contact
  data to know whether the current one-pair-at-a-time approximation
  actually matters for fidelity, or is fine in practice; not started.
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
- A car (box) actually being deflected by a curved fillet (see FR-020's
  Non-goals) — needs real support-mapping/SAT-style collision machinery
  against curved geometry this port doesn't have; a car currently drives
  straight through a curve's footprint unaffected. Not started.
- The compound corner where a vertical-edge fillet (FR-022) meets a
  floor-seam or ceiling-seam fillet (FR-020/FR-021), near a corner wall's
  own top/bottom endpoint — this port models each fillet as an independent,
  additive contact source (per `RB-PHYSICS-001`'s "single flat plane or
  single-radius fillet per boundary segment" Non-goal), with no blended 3D
  corner-sphere treatment where three surfaces would ideally meet smoothly.
  A concrete reason to model this (e.g. real recorded behavior specifically
  at these compound corners that diverges from the independent-fillets
  approximation) would justify the added complexity. Not started.
- Goal cutouts in the back walls (see FR-019's Non-goals) — the back walls
  are solid, flat planes spanning the full width; a concrete reason to
  model the cutout (e.g. real recorded goal-area behavior that diverges
  specifically because of it) would justify the added complexity. Not
  started.
- Disambiguating or blending a car's simultaneous contact with two walls
  at a corner for wall-jump purposes (see FR-019's Non-goals) — physical
  collision resolution already handles this correctly regardless; only
  the wall-jump push-off direction picker (`PhysicsWorld::step`'s
  "first wall in `self.walls`" rule) isn't. FR-019's corner walls make this
  case reachable in the standard arena for the first time; still not
  exercised by any test here. Not started.
- Sourcing or verifying `arena::CORNER_LENGTH`/`FILLET_RADIUS` against real
  field mesh data (see FR-019/FR-020/FR-021/FR-022) — this port has no
  reference for either at all, unlike `SIDE_WALL_X`/`BACK_WALL_Y`/
  `CEILING_Z`; even a sourced value would only approximate the real
  corner/transition, which isn't a single flat plane or single-radius
  fillet in the actual game. `FILLET_RADIUS` now also governs the 4 corner
  walls' floor/ceiling-seam fillets (FR-021) and all 8 vertical-edge
  fillets (FR-022), reusing the same cardinal-wall value rather than a
  separately chosen one for each — whether the real game even uses one
  uniform transition radius at all (as opposed to a genuinely different
  corner-specific curve, and a third, differently-shaped curve again at the
  vertical edges) is itself unconfirmed.
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
- No-split-impulse and no-warm-starting/sleeping are documented, deliberate
  gaps (see Non-goals). Now that ball-vs-car and car-vs-car collision are
  both real and actually wired into `PhysicsWorld` (not just ground
  contact), these matter more than they did before — worth revisiting once
  real recorded ball/car-hit behavior exists to compare against, rather
  than only the unit tests' internal-consistency checks (momentum
  conservation, no residual closing speed).
- `box_vs_box`'s edge-edge contact point uses the midpoint of the two
  closest points on the involved edges, and its face-contact clipping
  falls back to a single clamped-center point if clipping ever yields zero
  points (a defensive branch not exercised by real recorded data yet) —
  both are reasonable, tested choices, but neither has been validated
  against Bullet's own `dBoxBox` output or real car-vs-car contact
  behavior.

## Change history

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
  (`world::tests::landing_and_a_new_double_jump_clears_a_stale_dodge_flip_
  flag_in_a_live_world`) was loosened from an exact `assert_eq!` to a small
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
  not just an input-detection hack. On top of that, `drive::apply_driven_
  forces` gains a `wall_normal: Option<Vec3>` parameter (a per-step fact
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
- 0.6.0 (2026-08-29): Multi-car `PhysicsWorld` support — `car:
  Option<RigidBody>` is replaced by `cars: Vec<RigidBody>` (a breaking
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
