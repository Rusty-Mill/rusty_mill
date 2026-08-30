# Project Status

- Last verified main commit: `b748b86` (merge of [#35](https://github.com/baileyrd/rusty_bullet/pull/35))
- Verified at: 2026-08-30
- Current milestone: `PHASE-1-PHYSICS-CORE` (box-shaped car bodies, general 3x3 inertia, multi-contact resolution, ball-vs-car collision, car-vs-car collision, body-vs-arena-wall collision, ground-driving car input (throttle/steering), boost, handbrake, a ground jump, air control, a double jump (plain or a directional dodge), and a wall jump all implemented in `rb_physics_bullet` and wired into a real multi-car `PhysicsWorld`; a dodge variant of the wall jump, flip-cancel, landing auto-orientation assistance, variable jump height, a modeled arena footprint, and constant calibration still open) — In Progress
- Health: green — workspace builds, `fmt`/`clippy`/`test` all pass on `main`

## Completed

- `PHASE-0-BOOTSTRAP` — charter, system architecture, spec tree, spec
  registry, ADRs, research backlog, roadmap, traceability, AGENTS.md/
  WORKFLOW.md, governance file set, CI workflow, and a minimal buildable
  Cargo workspace with the divergence-scoring algorithm implemented and
  unit-tested. Merged via [PR #1](https://github.com/baileyrd/rusty_bullet/pull/1).
- `PHASE-1-PHYSICS-CORE-V0` — `rb_physics_bullet`, a from-scratch Rust port
  of Bullet3's rigid-body integration and sequential-impulse contact solver
  (zlib-licensed, see `THIRD_PARTY_NOTICES.md`), scoped to a dynamic sphere
  (ball) vs. static plane (ground) — per ADR-0004. Merged via
  [PR #1](https://github.com/baileyrd/rusty_bullet/pull/1).
- Status/workflow sync — merged via [PR #2](https://github.com/baileyrd/rusty_bullet/pull/2).
- `RB-VERIFY-001-FR-001/002/003` — `rb_replay_ingest` now really parses
  `.replay` files: `boxcars` parses the replay/network stream,
  `subtr-actor` resolves it into frame-indexed ball/car `RigidBody` state
  (avoiding a hand-rolled actor-graph resolver — see the crate's
  `Cargo.toml` dependency comment), and `convert.rs` maps that to
  `rb_domain::PhysicsFrame`. Verified end-to-end against a real vendored
  replay fixture (12,029 frames, ~428s match, ball position sane on every
  frame). Merged via [PR #3](https://github.com/baileyrd/rusty_bullet/pull/3).
- `RB-VERIFY-001-NFR-003` — a local, gitignored corpus health-check bin
  (`corpus_check`), run once against 40 of the owner's own real match
  replays (`baileyrd/replays`): 40/40 parsed cleanly, sane ball-position
  bounds on every file. Closes the "runs correctly on real owner data at
  scale" half of `RB-VERIFY-001`'s owner-data acceptance criterion; the
  manual single-timestamp cross-check remains open (see Blocked). Marks
  `PHASE-0-REPLAY-INGEST` Done.
- `ADR-0005` — decided the capture file format (JSON Lines) and a shared
  `rb_domain::ControllerInput` schema, resolving `RB-RESEARCH-O003` and the
  domain-schema question `RB-VERIFY-001-FR-004` had deferred.
  `RB-VERIFY-001-FR-004` — `rb_replay_ingest` now attaches recovered input
  (throttle/steer/jump/boost/handbrake) to `CarState.input`; `pitch`/`yaw`/
  `roll` stay `None` for replay-sourced input (never recoverable from a
  replay, see ADR-0005).
- `RB-VERIFY-002-FR-002`/`NFR-001` — `rb_capture_ingest` parses the
  JSON-Lines capture format into `PhysicsFrame`s with `CarState.input`
  always populated, tested against a synthetic hand-authored fixture (no
  real BakkesMod capture exists — see Blocked).
- `rb_verify_cli` divergence-scoring CLI wiring — `score_replay_against_capture`
  (new `lib.rs`, `main.rs` is now a thin argument/output wrapper over it)
  ingests a replay + a capture and runs `rb_domain::divergence::score`.
  Manually run against the vendored replay fixture + synthetic capture
  fixture: `frames compared: 5, mean ball distance: 0.25 uu, max ball
  distance: 0.25 uu`. Proves the pipeline runs end-to-end without erroring
  — not yet a fidelity measurement (see Blocked/Next).
- `RB-VERIFY-003-FR-002` (car-state scoring) — `DivergenceScore` gains a
  `cars: CarDivergence` field (mean/max position, rotation, velocity
  distance, pairs compared), matching recorded-to-candidate cars by
  `player_id` within each frame pair; a car present on only one side is
  skipped, not an error. New `Quat::angle_to` computes rotation distance
  (radians), using an `atan2`-based half-angle form rather than `acos`
  since `acos` is numerically unstable exactly where it matters most here
  (near-identical rotations). 8 new unit tests. Manually re-run: `car
  pairs compared: 5, mean car position/rotation/velocity distance: 2823.85
  uu / 2.36 rad / 1369.44 uu/s` (large numbers expected — unrelated
  matches, not a fidelity signal).
- `RB-VERIFY-003-FR-003` (timestamp-tolerant alignment) — `score` now
  aligns frames by nearest `timestamp_secs` (an `O(n+m)` merge, not a
  binary search per frame) instead of list index, with a required
  `max_timestamp_delta_secs` parameter so a match outside tolerance is
  skipped rather than force-matched. Implementing this surfaced a real
  bug: `rb_capture_ingest`'s synthetic fixture had timestamps starting at
  `0.0`, but the vendored replay fixture's ball doesn't spawn (produce a
  frame) until ~11.78s in — the old index-pairwise comparison never
  noticed, silently comparing temporally unrelated frames. Fixed the
  fixture's timestamps to actually overlap. `rb_verify_cli` gains
  `DEFAULT_MAX_TIMESTAMP_DELTA_SECS` (0.02s) and an optional third CLI
  argument to override it. 2 new unit tests. Manually re-run: `frames
  compared: 6, mean/max ball distance: 0.25 uu, car pairs compared: 6,
  mean car position/rotation/velocity distance: 2816.42 uu / 2.36 rad /
  1307.87 uu/s`. `RB-VERIFY-003` now has all three functional
  requirements implemented.
- `RB-PHYSICS-001-FR-004` (box-shaped car bodies) — `rb_physics_bullet`
  gains a unified `RigidBody`/`Shape` design (sphere or box, matching
  Bullet's own rigid-body-plus-collision-shape architecture) and a general
  3x3 inverse inertia tensor (`Mat3`, recomputed from orientation each
  step, shared by both shapes — a sphere's is mathematically
  orientation-independent, so this doesn't change ball behavior).
  Box-vs-plane contact generation tests all 8 corners against the plane
  (exact, not an approximation), producing 1-4 contacts depending on
  orientation; the solver now resolves an entire manifold together
  (multi-contact resolution) instead of one contact at a time.
  `PhysicsWorld` gains an optional car body (`with_car`), stepped and
  collided against the ground independently from the ball. **Not**
  implemented: box-vs-sphere (car-vs-ball) collision — the two bodies
  never collide with each other yet (needs a real convex narrow-phase
  algorithm, SAT or GJK/EPA); driven car input (a car here is a free
  rigid box, nothing couples throttle/steer/boost into it). 21 new unit
  tests (47 total in `rb_physics_bullet`), including a dropped-box
  settling test confirming multi-contact resolution keeps a symmetric
  box level instead of spuriously tipping it.
- `RB-PHYSICS-001-FR-004` (ball-vs-car collision, completing FR-004) —
  `rb_physics_bullet` gains analytic sphere-vs-box contact generation
  (`collision::sphere_vs_box`, a closed-form closest-point-on-box query
  handling both the ordinary case and a sphere-center-embedded-in-box
  deep-penetration case) and a two-dynamic-body sequential-impulse solver
  path (`solver::resolve_contact_between`) generalizing the existing
  body-vs-static-plane rows to carry both bodies' mass/inertia
  contributions, rather than assuming one side is static. `PhysicsWorld::step`
  was restructured into Bullet's actual staged pipeline (integrate every
  body's velocity → resolve every contact, ground and ball-vs-car → integrate
  every body's transform) so ball-vs-car resolution sees the same
  pre-integration state ground contacts do. `rb_domain::Quat` gains
  `conjugate` (needed to transform a world point into the box's local
  frame). **Not** implemented: box-vs-box collision (doesn't block this
  scope — there's only one car) and driven car input. 11 new unit tests in
  `rb_physics_bullet` (58 total) plus 1 in `rb_domain` (23 total),
  including an end-to-end `PhysicsWorld::step` test confirming a ball shot
  at a stationary car actually bounces off it instead of tunnelling
  through.
- `RB-PHYSICS-001-FR-006` (car-vs-car collision *detection*) —
  `rb_physics_bullet` gains `collision::box_vs_box`, a 15-axis
  separating-axis test between two oriented boxes (3+3 face axes, 9
  edge-pair axes), producing a clipped face manifold (0-4 points) or a
  single edge-edge point (via a standard closest-point-between-segments
  construction). `collision::contact_between` is generalized to
  `contacts_between` (returning `Vec<Contact>` uniformly) and
  `solver::resolve_contact_between` to `resolve_contacts_between` (a
  manifold, mirroring the existing ground-contact solver's structure) so
  box-vs-box's up-to-4-point case fits the same two-body solver path
  ball-vs-car already uses. **Not** wired up (at the time): `PhysicsWorld`
  still modeled exactly one car. 4 new unit tests in `rb_physics_bullet`
  (62 total).
- `RB-PHYSICS-001-FR-006` (multi-car `PhysicsWorld` support, completing
  FR-006) — `PhysicsWorld.car: Option<RigidBody>` is replaced by
  `cars: Vec<RigidBody>` (a breaking field rename); `with_car` now
  appends, so calling it repeatedly builds a scene with any number of
  cars. `PhysicsWorld::step` resolves every car's ground contact, every
  ball-vs-car pair, and every car-vs-car pair (via `collision::box_vs_box`,
  now running for real in a live scene instead of only under a unit test)
  each step, one pair at a time; `frame()` assigns each car's `player_id`
  as its index in `cars`. **Not** implemented (at the time): a combined
  multi-body solve for 3+ simultaneously-touching bodies and driven car
  input. 3 new unit tests in `rb_physics_bullet` (65 total), including an
  end-to-end test confirming two cars shot head-on at each other in a live
  `PhysicsWorld` actually bounce off instead of tunnelling through.
- `RB-PHYSICS-001-FR-007` (driven car input — ground throttle and steering
  only) — new `drive` module: `apply_driven_forces` couples
  `rb_domain::ControllerInput` into a throttle force (along the car's
  local forward axis, capped at `MAX_CAR_SPEED`, a commonly-cited
  community number) and a steering torque (about the car's local up axis,
  scaled by current speed so a stationary car can't turn in place), both
  gated on the car actually touching the ground. `THROTTLE_ACCELERATION`
  is a simplified constant (real Rocket League throttle tapers
  nonlinearly with speed); `STEER_TORQUE` is an uncalibrated placeholder
  with no public reference at all. `PhysicsWorld` gains
  `set_car_input` (persists a car's current input across steps) and
  `frame()` now reports each car's actual input instead of always `None`.
  A car with no input set behaves exactly as before this requirement
  existed. **Not** (at the time) implemented: boost, jump, air control
  (pitch/yaw/roll torque while airborne), and handbrake/drift — each a
  distinct real mechanic, tracked as separate follow-up. 10 new unit tests
  in `rb_physics_bullet` (75 total), including an end-to-end test
  confirming a car with throttle input actually drives forward across the
  ground in a live `PhysicsWorld::step` loop, and a regression test
  confirming a car with no input set is unaffected.
- `RB-PHYSICS-001-FR-008` (boost) — `drive::apply_driven_forces` gains a
  flat forward boost force (`BOOST_ACCELERATION * mass`, not
  speed-tapered like throttle, capped at the same `MAX_CAR_SPEED`),
  applied whenever `ControllerInput.boost` is set and the car has boost
  remaining. Unlike throttle/steering, boost is **not** gated on ground
  contact — it's a rocket, not an engine, so it works identically
  airborne. `MAX_CAR_SPEED`, `MAX_BOOST`, and `BOOST_ACCELERATION` are
  commonly-cited community numbers; `BOOST_CONSUMPTION_RATE` is a
  simplified constant approximating "a full tank lasts ~3 seconds".
  `PhysicsWorld` gains a parallel `car_boost: Vec<f32>` (kept in lockstep
  with `cars` by `with_car`, starting full) and `set_car_boost`; holding
  boost drains the tank at `BOOST_CONSUMPTION_RATE` per second whenever
  held, even once the force itself stops applying at `MAX_CAR_SPEED`
  (matching real Rocket League's "holding boost drains fuel regardless"),
  clamping at zero. `frame()` now reports each car's live `boost_amount`
  instead of a hardcoded `0.0`. **Not** (at the time) implemented: jump,
  air control, and handbrake/drift — each a distinct real mechanic,
  tracked as separate follow-up. 6 new unit tests across
  `drive.rs`/`world.rs` in `rb_physics_bullet` (81 total), including an
  end-to-end test confirming a car with boost input actually drives
  forward while airborne (gravity zeroed) in a live `PhysicsWorld::step`
  loop, and a regression test confirming a new car starts with a full
  boost tank.
- `RB-PHYSICS-001-FR-009` (handbrake) — `drive::apply_driven_forces`
  temporarily multiplies the car's `RigidBody.friction` by a new
  `HANDBRAKE_FRICTION_MULTIPLIER` (uncalibrated placeholder, no public
  reference at all) whenever `ControllerInput.handbrake` is held and the
  car is grounded, restoring it otherwise — gated on ground contact like
  throttle/steering. This models handbrake as a temporary grip reduction,
  letting the car's existing momentum carry it into a slide, reusing the
  ground-contact solver's existing Coulomb-friction machinery rather than
  inventing a separate lateral-slip system (this port has no per-wheel
  tire model to build a real rear-grip-loss mechanic on). `PhysicsWorld`
  gains a parallel `car_base_friction: Vec<f32>`, snapshotted from each
  car's own constructed `friction` by `with_car`, so handbrake restores
  the car's own value on release rather than a hardcoded default. **Not**
  (at the time) implemented: jump and air control — each a distinct real
  mechanic, tracked as separate follow-up. 5 new unit tests across
  `drive.rs`/`world.rs` in `rb_physics_bullet` (86 total), including an
  end-to-end test confirming a car already sliding sideways retains more
  of that slide under handbrake's reduced friction than under normal grip
  in a live `PhysicsWorld::step` loop, and a regression test confirming
  handbrake restores a car's own non-default base friction, not a
  crate-wide constant.
- `RB-PHYSICS-001-FR-010` (single ground jump) —
  `drive::apply_driven_forces` applies a fixed `JUMP_SPEED` instantaneous
  upward velocity change (via `RigidBody::apply_impulse`, not a continuous
  force) on the *rising edge* of `ControllerInput.jump` while the car is
  grounded — a fresh press, not merely held. A continued press through the
  resulting airborne period doesn't re-fire it, and releasing then
  re-pressing while still airborne doesn't fire it either (no double jump
  in this scope). `PhysicsWorld` gains a parallel `car_jump_held: Vec<bool>`
  (starting `false`, kept in lockstep with `cars` by `with_car`) carrying
  the rising-edge state across steps, the same pattern `boost_amount`
  already uses. `JUMP_SPEED` (292 uu/s) is a commonly-cited community
  number. **Not** (at the time) implemented: double jump/dodge, variable
  jump height (holding for a higher jump), wall jump, and air control —
  each a distinct real mechanic, tracked as separate follow-up. 6 new unit
  tests across `drive.rs`/`world.rs` in `rb_physics_bullet` (92 total),
  including an end-to-end test confirming a car with jump input actually
  leaves the ground in a live `PhysicsWorld::step` loop, and a regression
  test confirming that holding jump for a car's entire flight (never
  released) lets it land and settle instead of being relaunched on
  touchdown.
- `RB-PHYSICS-001-FR-011` (air control) — `drive::apply_driven_forces`
  applies torque about the car's local right/up/forward axes, scaled by
  `ControllerInput.pitch`/`yaw`/`roll` (each an `Option<f32>`, `None`
  treated as zero) times one shared `AIR_CONTROL_TORQUE` constant, gated
  on the car *not* touching the ground — the mirror image of
  throttle/steering/handbrake/jump's ground-only gating, so it never
  competes with ground steering for the yaw axis. Unlike ground steering,
  not speed-scaled: a car can spin from a standing start in the air, since
  there's no wheel grip to require momentum for. `AIR_CONTROL_TORQUE` is
  an uncalibrated placeholder shared across all three axes — a documented
  simplification, since real Rocket League's pitch/yaw/roll rates differ
  from each other. **Not** implemented: double jump/dodge, variable jump
  height, and wall jump — each a distinct real mechanic, tracked as
  separate follow-up. 6 new unit tests across `drive.rs`/`world.rs` in
  `rb_physics_bullet` (98 total), including an end-to-end test confirming
  a car with yaw input actually reorients itself mid-air (gravity zeroed)
  in a live `PhysicsWorld::step` loop, and a regression test confirming a
  grounded car stays level despite stray pitch/yaw/roll input.
- `RB-PHYSICS-001-FR-012` (double jump) — `drive::apply_driven_forces`
  fires one more, identical `JUMP_SPEED` instantaneous upward velocity
  change on a fresh airborne press of `ControllerInput.jump`, reusing the
  ground jump's own rising-edge detection and the `JUMP_SPEED` constant
  itself rather than a second edge-detector or a separately-calibrated
  speed. Gated on a new per-car `double_jump_available` flag: landing
  unconditionally restores it, and a fresh airborne press that spends it
  sets it back to `false` until the next landing, so it fires at most once
  per airborne period. `PhysicsWorld` gains a parallel
  `car_double_jump_available: Vec<bool>` (starting `true`, kept in
  lockstep with `cars` by `with_car`). `JUMP_SPEED` is now `pub`.
  Deliberately excludes the directional "dodge" impulse/torque a real
  double jump pairs with, variable jump height, and wall jump — each a
  distinct real mechanic, tracked as separate follow-up. 6 new unit tests
  across `drive.rs`/`world.rs` in `rb_physics_bullet`, minus one
  pre-existing `drive.rs` test whose premise this feature deliberately
  supersedes (103 total), including an end-to-end test confirming a double
  jump fired after a ground jump adds a second `JUMP_SPEED` kick on top of
  the first in a live `PhysicsWorld::step` loop (gravity zeroed), and a
  regression test confirming a spent double jump doesn't refire mid-air no
  matter how many more times jump is released and re-pressed before
  landing.
- `RB-PHYSICS-001-FR-013` (arena walls and wall jump) — `PhysicsWorld`
  gains `walls: Vec<StaticPlane>` and a `with_wall` builder; every body
  (ball and cars) now collides with every wall the same way it already
  collides with the ground (`resolve_ground_contact` renamed
  `resolve_plane_contact`, no behavior change — it never had ground-specific
  logic, just a ground-specific name). `drive::apply_driven_forces` gains a
  wall jump: a fresh airborne jump press while touching a wall
  (`wall_normal`, computed the same way `on_ground` is) fires an impulse
  combining a new `WALL_JUMP_HORIZONTAL_SPEED` (uncalibrated placeholder)
  outward along the wall's normal with `JUMP_SPEED` upward, taking priority
  over the double jump on that press. Wall contact — whether or not jump is
  pressed — unconditionally restores `double_jump_available`, the same
  rule landing uses, so wall jump doesn't cost a player their double jump
  and has no once-per-airborne-period limit of its own. Deliberately
  excludes the directional "dodge" a real wall jump can pair with,
  variable jump height, and any modeled arena footprint beyond generic
  flat walls (octagonal shape, curved transitions, a ceiling,
  multi-wall-corner disambiguation). 7 new unit tests across
  `drive.rs`/`world.rs` in `rb_physics_bullet` (110 total), including an
  end-to-end test confirming a car resting against a wall wall-jumps
  outward and upward, a second end-to-end test confirming a ball shot at a
  wall bounces off it instead of tunnelling through (the same proof
  ball-vs-car collision already has, now for walls), and a regression test
  confirming a car near but not touching an existing wall still gets a
  plain double jump.
- `RB-PHYSICS-001-FR-014` (dodge) — the double jump's fresh press now
  checks `ControllerInput.pitch`/`roll` at the moment it fires: at or
  above a new `DODGE_DEADZONE` on either axis, it fires a directional
  dodge instead of the plain vertical double jump — a purely horizontal
  `DODGE_SPEED` impulse (along `forward_axis` for `pitch`, `right_axis`
  for `roll`) plus an instantaneous `DODGE_ANGULAR_SPEED` spin written
  directly to `RigidBody.angular_velocity` about the perpendicular axis,
  reusing air control's own pitch/roll axis and sign conventions for
  direction (though not its `AIR_CONTROL_TORQUE` magnitude). Both axes can
  contribute at once (a diagonal dodge), simply summed rather than
  normalized — a documented simplification. Below `DODGE_DEADZONE` on both
  axes, the plain vertical double jump fires exactly as before; either way
  the press spends the shared `double_jump_available` resource. Wall jump
  is untouched — it never checks `pitch`/`roll`, so touching a wall always
  gets the fixed wall-jump push-off, never a dodge. `DODGE_SPEED` and
  `WALL_JUMP_HORIZONTAL_SPEED` are now `pub` (mirroring `JUMP_SPEED`) so
  `world.rs`'s end-to-end tests can assert against, and distinguish
  between, all three jump variants' distinct magnitudes. Deliberately
  excludes a dodge variant of the wall jump, flip-cancel, landing
  auto-orientation assistance, and variable jump height. 10 new unit tests
  across `drive.rs`/`world.rs` in `rb_physics_bullet` (120 total),
  including an end-to-end test confirming a car dodges forward with a
  visible flip after a ground jump in a live `PhysicsWorld::step` loop,
  and a regression test confirming a car touching a wall with directional
  stick input still gets the wall jump, not a dodge.

## In progress

- None.

## Blocked

- `RB-RESEARCH-O002` (binary reverse engineering of the shipped Rocket
  League client) — blocked on two things: (1) explicit owner sign-off after
  a legal/practical review, and (2) practically, this sandboxed environment
  has no access to the Rocket League client binary at all, so any actual RE
  work would have to happen on the owner's own machine. See
  `docs/research/RESEARCH-BACKLOG.md`.
- `RB-VERIFY-001`'s stricter manual single-timestamp cross-check (one ball
  position pinned against a remembered/verified instant, e.g. via in-game
  footage or BakkesMod) — the local `corpus_check` gate (40/40 real owner
  replays, see Completed) already closes the "runs correctly on real owner
  data at scale" half of this criterion; this narrower, precision-focused
  half is still open and needs the owner to do the manual cross-check
  locally, since this sandbox has no way to verify an exact remembered
  timestamp.
- `RB-VERIFY-002-FR-001` (the BakkesMod-side capture plugin) — this
  sandboxed environment has no Rocket League install, no BakkesMod, and no
  Windows to build a BakkesMod SDK plugin on at all (same practical
  blocker as `RB-RESEARCH-O002`). `rb_capture_ingest`'s Rust-side parser is
  implemented and tested against a synthetic fixture, but a real capture
  file — and therefore `RB-VERIFY-002`'s acceptance criteria and
  `PHASE-0-CAPTURE-INGEST`'s exit gate — needs this plugin built and run on
  the owner's own machine.
- `PHASE-0-EXIT`'s exit gate isn't fully met yet: `rb_verify_cli` runs
  end-to-end today with all of `RB-VERIFY-003` implemented (ball scoring,
  car scoring, timestamp-tolerant alignment), but only against a real
  replay + a *synthetic* capture (see above), and the divergence number it
  produces still isn't a meaningful fidelity comparison — there's still no
  Phase 1 candidate physics engine wired up to actually consume recorded
  inputs and produce a comparable trajectory (`rb_physics_bullet` now has
  a car body and ball-vs-car collision, but nothing yet connects it to
  recorded controller input or to `rb_verify_cli`).
- `RB-PHYSICS-001`'s combined multi-body solve (each ball-vs-car/car-vs-car
  pair resolves independently, one full solver pass at a time — a real
  approximation once 3+ bodies mutually touch in the same step), a dodge
  variant of the wall jump, flip-cancel, landing auto-orientation
  assistance, variable jump height, and a modeled arena footprint beyond
  generic flat walls — all real, not-yet-started follow-up work (see the
  spec's Non-goals/Open questions); a car can now drive, steer, boost (on
  the ground or in the air), handbrake/drift, take a ground jump, a double
  jump or a directional dodge, and a wall jump, and control itself in the
  air (pitch/yaw/roll), and bounces off the ball/other cars/arena walls,
  but can't yet dodge off a wall, cancel a dodge's flip early, get any
  landing assistance, vary its jump height, or interact with a real
  Rocket League-shaped arena.

## Next

1. `RB-VERIFY-002-FR-001` — write, build, and run the BakkesMod-side
   capture plugin against ADR-0005's JSON-Lines format, on the owner's own
   Windows/BakkesMod/game environment (this sandbox can't).
2. A dodge variant of the wall jump, flip-cancel, landing auto-orientation
   assistance, variable jump height, and/or a modeled arena footprint —
   real follow-up work for `rb_physics_bullet::drive`/`world`;
   `RB-PHYSICS-001-FR-005` (constant calibration, including `drive`'s own
   uncalibrated constants) needs `PHASE-0-EXIT` real data regardless.

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (170 tests: 23 in `rb_domain`, 120 in
  `rb_physics_bullet`, 14 in `rb_replay_ingest` (incl. real-fixture
  integration test), 10 in `rb_capture_ingest` (incl. synthetic-fixture
  test), 3 in `rb_verify_cli` (incl. real end-to-end run), plus doc-tests)
- `cargo run -p rb_replay_ingest --bin corpus_check` (local only, not CI):
  40/40 real owner replays parsed cleanly, 2026-08-28
- `cargo run -p rb_verify_cli --bin rb-verify -- <replay> <capture>`
  (manual, 2026-08-28, default 0.02s timestamp tolerance): `frames
  compared: 6, mean ball distance: 0.25 uu, max ball distance: 0.25 uu,
  car pairs compared: 6, mean car position/rotation/velocity distance:
  2816.42 uu / 2.36 rad / 1307.87 uu/s` against the real replay fixture +
  (now time-aligned) synthetic capture fixture.

## Risks and decisions needed

- `RB-RESEARCH-O001` (build vs. integrate physics) — **resolved**, see
  ADR-0004.
- `RB-RESEARCH-O002` (binary reverse engineering) — needs explicit owner
  sign-off after legal/practical review before any work starts, and needs
  the owner's own machine/game install since this sandbox has neither.
  Owner: baileyrd.
- `RB-RESEARCH-O003` (capture tooling scope) — **resolved**, see ADR-0005
  (one-off script, JSON-Lines format).
