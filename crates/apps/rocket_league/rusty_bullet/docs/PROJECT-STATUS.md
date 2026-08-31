# Project Status

- Last verified main commit: `ed8c59e` (merge of [#101](https://github.com/baileyrd/rusty_bullet/pull/101))
- Verified at: 2026-08-31
- Current milestone: `PHASE-1-PHYSICS-CORE` (box-shaped car bodies, general 3x3 inertia, multi-contact resolution, ball-vs-car collision, car-vs-car collision, body-vs-arena-wall collision, ground-driving car input (throttle/steering), boost, handbrake, a variable-height ground jump, air control, a double jump (plain or a directional, flip-cancelable dodge), a wall jump (itself dodgeable and flip-cancelable the same way), a gentle landing auto-orientation assist, a modeled octagonal arena footprint plus ceiling (`PhysicsWorld::standard_arena`), curved fillets throughout the arena's vertical boundary deflecting the ball and, since FR-027, a car too (via corner-testing, confirmed exact for this containment-style contact by FR-032) — floor/ceiling seams for all 9 walls (cardinal and diagonal corner, the 4 corner walls' own seams distinctly larger than the cardinal walls' since FR-025), all 8 of the corner walls' own vertical edges (FR-022), and all 16 compound corners where a vertical-edge fillet meets a floor- or ceiling-seam fillet (FR-023, sized to match FR-025's bigger corner-wall arches) — and, since FR-024, an actual goal-mouth window (with its own 3 rounded edges) cut into each back wall, with a car now able to drive through it too since FR-028 (via the same per-corner approximation technique), since FR-026, the 4 compound corners per goal where a post's own fillet meets the crossbar's, and, since FR-029, a modeled bounded interior behind each goal window (a solid box) so a ball or car passing through settles instead of flying forever, and, since FR-033, a real mass-spring net panel per goal catching the ball specifically (scoped to the ball only at the time — since FR-038, a car is caught too), and, since FR-030, every ball-vs-car/car-vs-car contact manifold in a step is resolved together as one combined multi-body solve instead of independent pairwise ones, and, since FR-031, uncalibrated placeholder constants have been individually audited against the community reverse-engineering effort (some corrected, some confirmed, the rest explicitly flagged), and, since FR-032, the once-suspected corner-testing under-detection gap for a car vs. a curved fillet was rigorously investigated and found not to exist (a genuine GJK-based replacement was built, found to regress two real tests, and reverted — the honest outcome is a corrected doc comment, not new production code), and, since FR-034, every contact's penetration/positional correction runs on its own separate split-impulse "push" channel instead of folding into the body's real velocity, so resolving deep overlap no longer injects spurious velocity, and, since FR-035, `solver::resolve_dynamic_manifolds` (every ball-vs-car/car-vs-car manifold) warm-starts each call from the previous one's converged impulses instead of zero, converging measurably closer to the true answer for an under-converged manifold, and, since FR-036, the ball's collision radius (`92.75` to `93.15`) and `arena::CEILING_Z` (`2044.0` to `2048.0`) were corrected via real source-level research rather than left as open ambiguities, and, since FR-037, sleeping forcibly zeroes a body's velocity once it's stayed below a linear and an angular threshold for a sustained time, fixing the "bouncy resting contact never settles" limitation neither split impulse nor warm-starting alone could, and, since FR-038, `net::NetMesh::step` catches every car too, not just the ball, closing this port's own former Non-goal, and, since FR-039, a wall jump at a corner (a car touching two walls at once) pushes off along every touched wall's normal summed and normalized instead of picking whichever wall came first, and, since FR-040, a dedicated research pass looked for a real reference for `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS` and found only one uncited, self-disclaimed-non-circular, likely-conflated wiki value, deliberately not adopted (both constants remain genuinely uncalibrated), and, since FR-041, `solver::resolve_dynamic_manifolds` scales each manifold's velocity-row impulse by a parameter-free `1 / k` for a body shared by `k >= 2` manifolds this step, narrowing FR-030's own documented "sandwiched" under-convergence gap (a naive global over-relaxation factor was investigated and rejected, since it provably diverges for that exact case), and, since FR-042, `box_vs_box`'s edge-edge contact point and face-clipping degenerate fallback were validated directly against `btBoxBoxDetector::dBoxBox`'s own real source — this port's finite-segment edge-edge point derivation confirmed more rigorous than the reference's own unclamped-infinite-line one, its synthesize-rather-than-drop fallback confirmed a deliberate favorable divergence, and a candidate fix for the edge-edge sign-selection heuristic built and empirically tested but found genuinely mixed, not adopted, and, since FR-043, this spec's own claim that Bullet's default restitution/friction combine mode is `max` was checked directly against real source and found wrong (the real default is an unclamped product), with this port's own average combine mode re-justified for a correct reason (it preserves the identity `combine(a, a) == a`, which the reference's product doesn't), and, since FR-044, a stale "split impulse isn't implemented" Non-goals bullet (contradicted by FR-034's own already-shipped implementation) was corrected, and, since FR-045, `integrate.rs`'s own Bullet-reference claims were checked directly against real fetched source and confirmed accurate, with one finding worth keeping — its degenerate-quaternion fallback deliberately preserves the prior orientation rather than resetting to identity, matching Bullet's own real choice, and, since FR-046, `body.rs`/`mat3.rs`'s own Bullet-reference claims were likewise checked directly and confirmed accurate, with one similar finding worth keeping — `Mat3::from_quat` doesn't self-correct a non-unit-length input the way Bullet's own version does, safe only because its single call site always receives an already-renormalized orientation, and, since FR-047, `collision.rs`'s remaining closed-form shape pairings (`sphere_vs_plane`, `box_vs_plane`, `sphere_vs_box`, `sphere_vs_sphere`) were likewise checked directly against real fetched source — `sphere_vs_plane` and `sphere_vs_sphere` confirmed exact, `sphere_vs_box`'s deep-penetration face selection confirmed to reproduce Bullet's own exact tie-break check order, and `box_vs_plane` confirmed a deliberate, more rigorous divergence from Bullet's own real single-contact-per-frame-plus-persistence default, in the same spirit as FR-042's `box_vs_box` finding, and, since FR-048, `solver.rs`'s own `restitution_curve`/`plane_space`/`setup_rows`/`resolve_row` and `btContactSolverInfo`'s cited defaults were likewise checked directly and confirmed exact or confirmed-equivalent restructurings, with one genuine, significant finding kept open rather than fixed — this port always derives both friction directions from a fixed, velocity-independent basis, while Bullet's own real default aligns one direction with the actual relative sliding velocity, a physically meaningful difference deliberately left for a dedicated future FR — all implemented in `rb_physics_bullet` and wired into a real multi-car `PhysicsWorld`; real-data constant calibration (FR-005) still blocked on PHASE-0-EXIT) — In Progress
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
- `RB-PHYSICS-001-FR-015` (variable jump height) — the ground jump gains a
  hold window: continuing to hold `ControllerInput.jump` after the fresh
  press that fires it adds a continuous `JUMP_HOLD_ACCELERATION` upward
  force, for up to `JUMP_HOLD_MAX_DURATION` seconds, on top of the fixed
  `JUMP_SPEED` impulse. A new per-car `jump_hold_time_remaining: f32`
  (`PhysicsWorld`'s parallel `car_jump_hold_time_remaining: Vec<f32>`,
  starting `0.0`) is checked and decremented against the *previous* call's
  value before that same call's own ground-jump-press handling can re-arm
  it, so a fresh press's own step only ever fires the plain impulse — only
  continued holding into later calls earns the extra height. Releasing
  `jump` zeroes the window immediately, even with time left. Scoped to the
  ground jump alone: the double jump, a dodge, and the wall jump all
  require releasing jump first to fire, which itself unconditionally
  zeroes the hold window, so none of the three can be boosted by a
  leftover window. `JUMP_HOLD_MAX_DURATION` and `JUMP_HOLD_ACCELERATION`
  are both uncalibrated placeholders — no public reference exists for real
  Rocket League's actual hold-window length or acceleration the way
  `JUMP_SPEED` does. The pre-existing
  `holding_jump_does_not_repeatedly_relaunch_the_car` regression test's run
  duration was extended (1.5s → 3.0s) since a continuously held jump now
  climbs higher and takes longer to land. 6 new unit tests across
  `drive.rs`/`world.rs` in `rb_physics_bullet` (126 total), including an
  end-to-end test confirming a held ground jump reaches a greater peak
  height than a tapped one in a live `PhysicsWorld::step` loop, and a
  regression test confirming a double jump fired after holding the ground
  jump through its whole window still adds exactly one more `JUMP_SPEED`
  kick, not an extra variable-height boost.
- `RB-PHYSICS-001-FR-016` (flip-cancel) — a dodge's spin can now be
  canceled early: a further fresh `ControllerInput.jump` press while
  airborne, not touching a wall, with the double jump already spent by
  that dodge, zeroes `RigidBody.angular_velocity` outright instead of
  leaving the flip to spin indefinitely. A new per-car
  `dodge_flip_active: bool` (`PhysicsWorld`'s parallel
  `car_dodge_flip_active: Vec<bool>`, starting `false`) tracks this: the
  directional-dodge branch sets it `true`; the plain-double-jump branch
  explicitly sets it `false` rather than leaving it alone — closing off a
  real staleness bug this port's own regression tests were written to
  catch and did catch (verified by temporarily removing the fix and
  confirming both the `drive.rs` and `world.rs` regression tests fail
  without it) — without that explicit clear, a much-later, completely
  unrelated plain double jump would leave the flag `true`, letting a
  further press spuriously cancel a flip that no longer exists.
  Flip-cancel touches neither the dodge's own linear velocity nor
  `double_jump_available`. Wall jump keeps its existing priority,
  unchanged. No new physics constants — a state-flag-gated zeroing action,
  not a magnitude to calibrate. 6 new unit tests across
  `drive.rs`/`world.rs` in `rb_physics_bullet` (132 total), including an
  end-to-end test confirming a second jump press cancels a dodge's spin in
  a live `PhysicsWorld::step` loop, and a regression test confirming
  landing and a later plain double jump clear a stale cancelable-flip flag
  there too, not just in `drive.rs` isolation.
- `RB-PHYSICS-001-FR-017` (wall-jump dodge) — the wall jump's own fresh
  press now checks `ControllerInput.pitch`/`roll` against `DODGE_DEADZONE`,
  the same check the ground double jump's press already uses: at or above
  it on either axis, a wall-jump dodge fires instead of the plain fixed
  push-off — the same outward-plus-upward impulse combined with a
  horizontal `DODGE_SPEED` component and `DODGE_ANGULAR_SPEED` spin
  (identical axis/sign conventions to the ground dodge), also arming
  `dodge_flip_active` so its spin is flip-cancelable exactly like a ground
  dodge's. Below the deadzone, the plain wall jump fires exactly as before,
  still never touching `double_jump_available`. Unlike the plain wall
  jump, the dodge variant *does* consume `double_jump_available` — a
  deliberate simplification: since touching a wall unconditionally
  restores `double_jump_available` before this check ever runs, gating the
  dodge variant on it would be vacuous (always true there); having it
  spend the resource instead keeps flip-cancel's existing invariant intact
  with zero changes to its branch ordering. No new physics constants —
  reuses `DODGE_SPEED`/`DODGE_ANGULAR_SPEED`/`WALL_JUMP_HORIZONTAL_SPEED`/
  `JUMP_SPEED` outright. Two pre-existing tests whose entire premise this
  requirement deliberately reverses ("wall jump always ignores stick
  input") were repurposed in place — not silently deleted — to assert the
  new behavior instead. 6 new unit tests across `drive.rs`/`world.rs` in
  `rb_physics_bullet` (138 total): a wall-jump dodge consumes the double
  jump unlike a plain wall jump; its spin can be flip-cancelled; a
  below-deadzone stick deflection still gives a plain wall jump; opposite
  stick sign dodges the opposite direction; a diagonal (pitch+roll)
  wall-jump dodge combines both axes, plus — the real end-to-end proof — a
  wall-jump dodge firing in a live `PhysicsWorld::step` loop, and a second
  end-to-end test confirming its spin is flip-cancelable there too.
- `RB-PHYSICS-001-FR-018` (landing auto-orientation assist) —
  `drive::apply_driven_forces` gains a gentle continuous restoring torque,
  applied while airborne, nudging the car's local up axis back toward
  world up: `up_axis(car).cross(&world_up) * LANDING_AUTO_UPRIGHT_TORQUE`,
  whose magnitude is already proportional to the sine of the car's tilt
  since both vectors are unit length (no correction for a level car, a
  proportionally stronger nudge for a heavily tilted one). Gated on no
  active `pitch`/`roll` air-control input this step (never fights the
  player's own steering) and no fresh `ControllerInput.jump` press this
  step (avoiding a same-step conflict with a dodge's/wall-jump-dodge's/
  double-jump's/flip-cancel's own direct angular-velocity change, both
  resolved by the same `integrate_velocities` call). Real Rocket League
  triggers this on approach to the ground; this port has no raycast or
  distance query to replicate that, so it applies continuously whenever
  airborne instead — a documented simplification. New constant
  `LANDING_AUTO_UPRIGHT_TORQUE` is an uncalibrated placeholder, deliberately
  one order of magnitude smaller than `AIR_CONTROL_TORQUE` so it reads as
  gentle assistance, not full control. Known, accepted limitation: a car
  resting exactly upside-down gives a zero cross product, so no correction
  is computed in that unlikely exact singularity. 5 new unit tests across
  `drive.rs`/`world.rs` in `rb_physics_bullet` (143 total): a tilted
  airborne car with no input gets a corrective torque; an already-upright
  airborne car gets none; the assist has no effect while grounded; it
  doesn't fire while pitch air control is actively held; and — the real
  end-to-end proof — a car tilted 90 degrees with no input trends back
  toward level over 120 steps of a live `PhysicsWorld::step` loop (gravity
  zeroed). This closes out the last item tracked in `drive.rs`'s own
  module doc "Not implemented" list since the dodge (FR-014) increment.
- `RB-PHYSICS-001-FR-019` (modeled arena footprint) — a new `arena` module
  builds Rocket League's real standard-arena boundary entirely from
  `RB-PHYSICS-001-FR-013`'s existing generic `StaticPlane`/`with_wall`
  machinery: no new collision code, since a ceiling and a corner-cut wall
  are each just another flat plane. `arena::standard_ground` is the flat
  floor at `z = 0`; `arena::standard_walls` returns 9 `StaticPlane`s — 2
  side walls (`x = ±SIDE_WALL_X`), 2 back walls (`y = ±BACK_WALL_Y`), a
  ceiling (`z = CEILING_Z`), and 4 diagonal corner walls (one per quadrant)
  cutting off the true rectangular corner, giving the field its real
  octagonal footprint. `SIDE_WALL_X` (4096), `BACK_WALL_Y` (5120), and
  `CEILING_Z` (2044) are commonly-cited community-measured field
  dimensions; the corner walls' inset (`CORNER_LENGTH`, equal along both
  axes) is this project's own uncalibrated placeholder — this port has no
  verified reference for the real arena's actual corner geometry, which
  isn't even a single flat plane in the real field mesh. New
  `PhysicsWorld::standard_arena` convenience constructor wires both into a
  `PhysicsWorld` in one call, alongside (not replacing) the existing
  `PhysicsWorld::new`/`with_wall` ad-hoc-wall capability. Still not
  modeled: curved wall-to-floor/wall-to-ceiling transitions, goal cutouts
  in the back walls, and disambiguating or blending a car's simultaneous
  contact with two walls at a corner for wall-jump purposes (physical
  collision resolution already handles a car touching two walls at once
  correctly regardless — only the wall-jump push-off direction picker
  isn't). 10 new unit tests across `arena.rs`/`world.rs` in
  `rb_physics_bullet` (153 total), including end-to-end tests confirming
  `PhysicsWorld::standard_arena` carries exactly 9 walls and the standard
  ground, a ball bounces off the standard arena's side wall rather than
  escaping, and a ball fired at the true rectangular corner is stopped by
  the diagonal corner wall well before its x or y individually reaches
  either cardinal wall's own position.
- `RB-PHYSICS-001-FR-020` (curved wall-to-floor/wall-to-ceiling
  transitions) — a new `body::StaticQuarterPipe` shape (an immovable
  partial-cylinder fillet, infinite along its own axis like `StaticPlane`)
  and `collision::contacts_vs_quarter_pipe` (sphere-only — a box always
  gets no contact, deliberately deferred). The playable side is the
  *inside* of the fillet's concave face (a skateboard quarter-pipe, ridden
  on the inside): governed only within the 90-degree sector from
  `sector_start` to `sector_end`, contact fires as the sphere's surface
  approaches or crosses the fillet's own radius from inside, pushing back
  toward the axis — the opposite direction convention from a flat plane's
  always-away-from-the-surface push. `StaticQuarterPipe::between_planes`
  derives a fillet's axis/sector automatically from the two flat planes it
  bridges — exact only for two perpendicular, axis-aligned planes (every
  cardinal arena wall's own floor/ceiling seam, not a diagonal corner
  wall's). `PhysicsWorld` gains `curves`/`with_curve`/`resolve_curve_contact`
  (mirroring `walls`/`with_wall`/`resolve_plane_contact`).
  `solver::resolve_contacts`'s second parameter changed from `&StaticPlane`
  to plain `restitution: f32, friction: f32` — the only two fields it ever
  used — so the same solver path now serves a fillet exactly as it already
  served a flat plane, no new solver code needed. `arena::standard_curves`
  builds the 8 fillets (floor-side and ceiling-side, per cardinal wall) via
  a new uncalibrated placeholder `FILLET_RADIUS`;
  `PhysicsWorld::standard_arena` now adds these alongside its 9 walls.
  Still not modeled: a car actually being deflected by a fillet, fillets at
  the 4 diagonal corner walls, and goal cutouts. 15 new unit tests across
  `body.rs`/`collision.rs`/`arena.rs`/`world.rs` in `rb_physics_bullet`
  (168 total), including an end-to-end test confirming a ball resting at
  ordinary flat-floor height within a curve's footprint — already
  overlapping the fillet's own material — gets pushed up off that height
  instead of staying embedded, and a regression test confirming a car in
  the exact same position is completely unaffected.
- `RB-PHYSICS-001-FR-021` (curved corner-wall-to-floor/wall-to-ceiling
  transitions) — extends FR-020's fillet treatment to the 4 diagonal corner
  walls: `arena::standard_curves` now returns 16 `StaticQuarterPipe`s
  (still one floor-side and one ceiling-side fillet per wall, now for all 9
  walls) instead of 8. `StaticQuarterPipe::between_planes` needed no code
  changes — its real correctness requirement was never "axis-aligned
  planes" (FR-020's own doc comment had incorrectly claimed that), only
  that the two bridged planes' normals are mutually perpendicular, which
  holds for a corner wall meeting the floor/ceiling regardless of the
  corner wall's own horizontal rotation. A corner wall's fillet
  `axis_direction` is instead computed via a cross product
  (`floor.normal.cross(&wall.normal)`, already unit length by construction,
  so no `.normalize()`/`.unwrap()` is needed) rather than hand-picked,
  since it isn't a coordinate axis the way a cardinal wall's is. New
  `arena::corner_wall_plane` helper factors out the existing
  (behavior-unchanged) corner-wall plane construction so `standard_curves`
  can reuse it. `PhysicsWorld::standard_arena` picks up the extra 8 curves
  automatically. `FILLET_RADIUS` is reused as-is rather than a second,
  independently chosen radius. Still not modeled: a car actually being
  deflected by any fillet, a fillet at a corner wall's own *vertical* edges
  (where it meets its neighboring side/back wall), and goal cutouts. 4 new
  unit tests across `arena.rs`/`world.rs` in `rb_physics_bullet` (172
  total): `standard_curves` returns exactly 16 fillets; every fillet's axis
  sits radius-in from some vertical wall, cardinal or corner; a corner
  wall's own derived fillet axis sits radius-in from both the corner wall
  and the floor with correctly perpendicular unit sector vectors; the
  cross product computing each of the 4 corner walls' `axis_direction` is
  exactly unit length, plus — the real end-to-end proof — a new
  `PhysicsWorld` test built around a wall with a diagonal (non-axis-aligned)
  normal confirms a ball resting within that diagonal wall's fillet
  footprint gets pushed up off flat-floor height, the same physical proof
  FR-020 gave for a cardinal wall.
- `RB-PHYSICS-001-FR-022` (curved corner-wall vertical-edge fillets) —
  rounds off the standard arena's last remaining sharp edges: the 8
  vertical edges where each of the 4 diagonal corner walls meets its
  neighboring side or back wall. `arena::standard_curves` now returns 24
  `StaticQuarterPipe`s (the 16 floor/ceiling-seam fillets already built,
  plus 8 vertical-edge fillets). Unlike every prior fillet, the two planes
  a vertical-edge fillet bridges aren't perpendicular — a corner wall meets
  its neighbor at 135 degrees, not 90 — which exposed a real gap:
  `StaticQuarterPipe::between_planes` previously only computed the correct
  axis point for perpendicular planes (a shortcut that silently gives the
  wrong point at any other angle). It's now fully general: it solves the
  axis point as a real 2x2 linear system, its own sector angle comes out to
  exactly the angle between the two planes' normals (45 degrees here, 90
  for a floor/ceiling seam), and it self-corrects a "backwards"
  `axis_direction` internally so a caller can pass either of the two
  opposite directions along the shared edge line. `sphere_vs_quarter_pipe`'s
  sector-membership test is likewise generalized from a two-dot-products
  shortcut (only correct for a 90-degree sector) to a signed-cross-product
  test valid for any sector up to 180 degrees. The vertical-edge fillets'
  own `axis_direction` is simply `(0, 0, 1)` (the edge itself is vertical).
  `FILLET_RADIUS` is reused as-is once again. Still not modeled: a car
  actually being deflected by any fillet, the compound corner where a
  vertical-edge fillet meets a floor- or ceiling-seam fillet, and goal
  cutouts. 9 new unit tests across `body.rs`/`arena.rs`/`world.rs` in
  `rb_physics_bullet` (181 total): 5 in `body.rs`, using a synthetic
  non-perpendicular fixture independent of the arena's own geometry —
  axis radius-in from both planes with tangent points on each, the derived
  sector angle matching the angle between the two planes' normals, the
  sharp corner sitting outside its own radius but within its sector, and
  either `axis_direction` sign producing the same correctly-oriented
  sector; 3 in `arena.rs` — exactly 24 fillets, every vertical-edge
  fillet's `axis_direction` running purely along Z, and a corner wall's own
  vertical-edge fillet sitting radius-in from both adjoining walls with a
  45-degree sector; 1 in `world.rs` — the real end-to-end proof, a ball
  embedded past a vertical-edge fillet's own radius at a wall-to-wall angle
  that isn't a right angle gets pushed meaningfully back toward the axis.
- `RB-PHYSICS-001-FR-023` (compound-corner fillets) — rounds off the last
  16 sharp vertices in the standard arena's vertical boundary: the
  compound corners where a corner wall's own vertical-edge fillet (FR-022)
  meets a floor- or ceiling-seam fillet (FR-020/FR-021). A compound corner
  is where three planes meet at once, which no existing cylindrical
  `StaticQuarterPipe` can blend, so this requirement introduces a new
  static shape, `body::StaticCornerFillet` — an immovable sphere riding the
  concave inside of the vertex. Its `between_three_planes` constructor
  reuses the same "radius-in from every bridged plane" invariant
  `StaticQuarterPipe::between_planes` already relies on: since the
  fillet's center must sit exactly `radius` in from all three planes, it's
  also exactly `radius` in from each pair — meaning it already lies on all
  three of that vertex's own pairwise `between_planes` axis lines
  simultaneously, so the center is just those three lines' common
  intersection, solved directly via the classic three-plane-intersection
  cross-product form of Cramer's rule. Containment (new
  `collision::sphere_vs_corner_fillet`) generalizes a `StaticQuarterPipe`'s
  2-sided sector test to a "spherical triangle": inside iff a direction's
  dot product with each of 3 `bounds` is non-negative, each bound the raw
  (non-normalized — only its sign is used) cross product of a pair of
  normals, sign-corrected against the third plane's own normal to always
  point toward the sharp corner — provably correct since that dot product
  is exactly the derivative of the third plane's signed distance along a
  candidate direction. No `.normalize()`/`.unwrap()` needed anywhere in
  this new production code, the same discipline `between_planes`'s own
  FR-022 self-correction established. `arena::standard_corner_fillets`
  builds all 16 (4 per corner wall, times the 4 corner walls) directly from
  the same three flat planes `standard_walls` already builds, reusing
  `FILLET_RADIUS` once again. `PhysicsWorld` gains a parallel
  `corner_fillets: Vec<StaticCornerFillet>` field and a `with_corner_fillet`
  builder, resolved for the ball and every car exactly like `curves` (a
  no-op for a car, same deferred case as every other fillet).
  `PhysicsWorld::standard_arena` wires in all 16 automatically. Still not
  modeled: a car actually being deflected by any fillet, and goal cutouts.
  13 new unit tests across `body.rs`/`collision.rs`/`arena.rs`/`world.rs`
  in `rb_physics_bullet` (194 total): 4 in `body.rs` (using a synthetic
  fixture combining a perpendicular floor with the same 45-degree
  non-perpendicular wall pair `between_planes`'s own FR-022 fixture uses)
  proving the center sits radius-in from all three planes with tangent
  points exactly on each, and the derived `bounds` correctly include the
  direction toward the sharp corner and exclude the direction pointing
  away from it; 5 in `collision.rs` mirroring `sphere_vs_quarter_pipe`'s
  own test shapes (deep-inside no contact, touching zero penetration,
  pushed-past positive penetration toward the center, outside-bounds no
  contact, box always empty); 2 in `arena.rs` — exactly 16 fillets, and
  every fillet's center sits radius-in from a floor/ceiling plane, a
  side/back wall, and a corner wall simultaneously; 2 in `world.rs` —
  `standard_arena` carries exactly 16 corner fillets, plus the real
  end-to-end proof, a ball embedded past a compound-corner fillet's own
  radius gets pushed meaningfully back toward the center.
- `RB-PHYSICS-001-FR-024` (goal cutouts) — opens an actual goal-mouth
  window in each back wall, rounded at its own rim, where every prior
  increment had a single solid, flat plane spanning the full width. New
  static shape `body::StaticGoalWall` — a `StaticPlane` plus a rectangular
  window in the plane's own local `u_axis`/`v_axis` frame — with
  `contains_in_window` testing a point's projection onto that frame
  directly, independent of the point's own depth from the plane. New
  `collision::sphere_vs_goal_wall`/`contacts_vs_goal_wall`: a sphere (the
  ball) gets no contact at all when its center falls inside the window,
  letting it pass through; a box (car) falls straight through to the
  ordinary `contacts_vs_plane` against the wrapped plane, deliberately
  ignoring the window — a zero-regression choice, since a car now sees
  literally the same contact-generation call it always did.
  `arena::standard_walls` drops its 2 back-wall `StaticPlane`s (now 7
  planes instead of 9); new `arena::standard_goal_walls` returns them
  instead as 2 `StaticGoalWall`s, windowed at new commonly-cited constants
  `GOAL_HALF_WIDTH`/`GOAL_HEIGHT`. New `arena::standard_goal_cutout_fillets`
  rounds each window's 3 edges (two posts, one crossbar, per goal — 6
  `StaticQuarterPipe`s, added to the same `curves` list `standard_curves`'s
  24 already populate), each derived via the existing
  `StaticQuarterPipe::between_planes` from the real back-wall plane and a
  second, purely-geometric plane (`goal_post_plane`/`goal_crossbar_plane`)
  representing the post's or crossbar's own inward-/downward-facing
  surface, positioned at exactly the window's own edge so the fillet's
  tangent point lands exactly on the window boundary with no gap or
  overlap. Unlike a real wall, these post/crossbar planes are never
  themselves added as collision geometry — an infinite plane facing
  straight along X (or capping Z) would incorrectly wall off the entire
  rest of the field at that coordinate. `PhysicsWorld` gains a parallel
  `goal_walls: Vec<StaticGoalWall>` field and `with_goal_wall` builder,
  resolved for the ball *and* every car (unlike `curves`/`corner_fillets`'s
  ball-only resolution) — safe precisely because the box path is a no-op
  change from the prior plain-`StaticPlane` behavior. Still not modeled:
  a car actually being deflected by any fillet or driving into a goal, a
  modeled goal interior/net beyond the cutout itself, and the goal's own
  two compound top corners where a post's fillet meets the crossbar's. 17
  new unit tests across `body.rs`/`collision.rs`/`arena.rs`/`world.rs` in
  `rb_physics_bullet` (211 total): 4 in `body.rs` proving
  `contains_in_window` is true at the window's own center and just inside
  each of its four edges, false just outside them, and unaffected by a
  point's distance from the plane; 4 in `collision.rs` — a sphere embedded
  in the window has no contact, a sphere outside the window behaves
  exactly like an ordinary plane contact both embedded and resting exactly
  at the surface, and a box's contact through the windowed wall is
  bit-for-bit identical to plain `contacts_vs_plane` against the same
  wrapped plane; 5 in `arena.rs` — `standard_walls` returns exactly 7
  planes, `standard_goal_walls` returns exactly 2 sharing one offset
  magnitude with each window centered correctly, `standard_goal_cutout_fillets`
  returns exactly 6 fillets each sitting radius-in from a real back wall
  and a post/crossbar plane; 4 in `world.rs` — `standard_arena` carries
  exactly 2 goal walls, a ball fired through a goal-mouth window's center
  passes the back wall's own position while a car aimed at the same spot
  is still stopped by it, and an end-to-end test proving a ball embedded
  past a goal-post fillet's own radius gets pushed meaningfully back
  toward the axis.
- `RB-PHYSICS-001-FR-025` (corner-wall floor/ceiling arch radius) — a
  diagonal corner wall's own floor-seam and ceiling-seam fillets (8 of
  `standard_curves`'s 24 entries) now use a new, distinctly larger
  `arena::CORNER_ARCH_RADIUS` (750 uu) instead of the cardinal walls' own
  `FILLET_RADIUS` (292 uu), matching real Rocket League's noticeably bigger,
  more swept corner-boost curve rather than a scaled-down copy of a cardinal
  wall's small rounding. Because `StaticCornerFillet::between_three_planes`
  needs one shared radius across all three planes it blends to still meet
  its adjoining edge fillets exactly where their axes cross (the same
  no-gap property `RB-PHYSICS-001-FR-023` established), all 16
  `standard_corner_fillets` switch to `CORNER_ARCH_RADIUS` too, since every
  one touches one of these bigger arches. Unaffected, still `FILLET_RADIUS`:
  the 8 cardinal-wall floor/ceiling seams, the 8 vertical corner-edge
  fillets (FR-022), and the 6 goal-cutout edge fillets (FR-024) —
  independent, additive contact sources next to the bigger arches, not
  blended with them. `CORNER_ARCH_RADIUS` is an uncalibrated placeholder
  like every other arena dimension in this crate; a compile-time
  `const _: () = assert!(CORNER_ARCH_RADIUS > FILLET_RADIUS);` enforces the
  "distinctly larger" relationship. Validating this surfaced a real,
  pre-existing (already-documented) latent issue: `StaticQuarterPipe` is
  infinite along its own axis, so a ball fired dead down the arena's own
  center line eventually re-enters some corner-wall arch's resting shell far
  past the goal — already true with the old, smaller `FILLET_RADIUS` (a
  mild, harmless correction around y≈7650-7930), but FR-025's bigger radius
  moves that zone closer in (y≈6300-7700) and turns it into a much sharper,
  solver-destabilizing correction. Fixed by shortening the pre-existing
  `a_ball_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall`
  end-to-end test's flight duration (3.0s → 1.8s) to comfortably clear the
  back wall without re-entering that already-documented infinite-fillet
  zone — a test-scoping fix, not a new capability or Non-goal. 1 new unit
  test in `world.rs` in `rb_physics_bullet` (212 total): the real end-to-end
  proof, a ball embedded past a corner-wall floor arch's own (larger)
  radius gets pushed meaningfully back toward the axis.
- `RB-PHYSICS-001-FR-026` (goal post-crossbar corner fillets) — closes the
  gap `RB-PHYSICS-001-FR-024`'s own doc comment flagged: the two compound
  corners per goal where a post's own vertical edge fillet meets the
  crossbar's own horizontal edge fillet, one per post per goal (4 total).
  New `arena::standard_goal_corner_fillets` builds all 4 directly from
  `StaticCornerFillet::between_three_planes` on the real back wall/post/
  crossbar planes that meet there — the same approach `RB-PHYSICS-001-FR-023`
  used for the arena's own 16 compound corners, and no new shape or
  collision code, since `StaticCornerFillet`/`sphere_vs_corner_fillet`
  already generalize to any three non-parallel planes. Reuses
  `FILLET_RADIUS` unchanged: unlike `FR-025`'s arena corners, both edge
  fillets meeting here already share one radius, so there's no
  mismatched-radius concern. The goal's other two corners, where a post
  meets the floor, deliberately get no such treatment: the window's own
  bottom edge sits exactly at floor level, so a post's fillet there simply
  ends flush with the ground the ball already rolls on, not a sharp,
  unrounded vertex. `PhysicsWorld::standard_arena` wires the 4 new fillets
  in via the same `with_corner_fillet` builder `standard_corner_fillets`'s
  16 already used, bringing `corner_fillets` to 20 total. 3 new unit tests
  across `arena.rs`/`world.rs` in `rb_physics_bullet` (215 total): 2 in
  `arena.rs` — exactly 4 fillets, and every fillet's center sits
  `FILLET_RADIUS` in from a back wall, a post plane, and the crossbar
  plane simultaneously (proving a real triple intersection, not an
  arbitrary point); 1 in `world.rs` — the real end-to-end proof, a ball
  embedded past a goal corner fillet's own radius (on a synthetic
  back-wall/post/crossbar fixture) gets pushed meaningfully back toward
  the center.
- `RB-PHYSICS-001-FR-027` (car deflection by curved fillets) — closes the
  Non-goal repeated across every fillet increment since FR-020: a car
  (box) is now actually deflected by every curved fillet in this port, not
  just the ball. New `collision::box_vs_quarter_pipe`/`box_vs_corner_fillet`
  reuse the same "test every corner" technique `box_vs_plane` already used
  for a flat plane — each of a box's 8 corners is checked as a zero-radius
  sphere via the existing `sphere_vs_quarter_pipe`/`sphere_vs_corner_fillet`,
  and every corner that reports a contact contributes one to the manifold,
  with each surviving contact's `point` overwritten to the corner's own
  world position (not the fillet-surface point those functions themselves
  compute) for the same rel_pos/torque-accuracy reason `box_vs_plane`'s own
  doc comment gives. `contacts_vs_quarter_pipe`/`contacts_vs_corner_fillet`
  now dispatch a `Shape::Box` to these instead of `Vec::new()`; no
  `PhysicsWorld::step` changes were needed at all, since it turned out
  `resolve_curve_contact`/`resolve_corner_fillet_contact` were already
  being called for every car in the scene, just as a silent no-op until
  now. Documented as an approximation, not a full convex-vs-curved-surface
  narrow phase (no GJK/EPA support-mapping machinery was added): a box
  face resting flush against a shallow curve can have every one of its own
  corners still just clear of the fillet while the face's middle already
  overlaps it, under-detecting that case — the same "exact per test-point,
  an approximation of the whole shape" caveat this crate has always
  carried for curved geometry. `StaticGoalWall`/`contacts_vs_goal_wall` is
  unaffected — a goal wall isn't a curved fillet, so a car still sees the
  same solid, full-width back wall it always has, and still can't drive
  into a goal. 3 net new/replaced unit tests across `collision.rs`/`world.rs`
  in `rb_physics_bullet` (218 total): `collision.rs` replaced its two old
  "box vs. curved fillet is always empty" regression tests with proofs
  that an embedded box gets a correctly-directed contact and a
  clearly-outside-the-sector/bounds box still gets none; `world.rs`
  replaced `a_car_is_not_deflected_by_a_curved_transition` (whose entire
  premise this increment reverses) with an end-to-end proof that a car
  resting within a curve's footprint gets pushed up exactly like the ball
  does, and added a compound-corner-fillet car test checking the car's
  *worst corner penetration* shrinks (not that its center of mass
  approaches the fillet's center, the way the equivalent ball test
  checks) — an oriented box's corners sit at different depths at once, so
  resolving one corner's contact can rotate the box in a way that moves
  its center away from the fillet even as every individual corner's own
  overlap is being corrected; this was found empirically (an earlier,
  center-of-mass-based assertion actually failed) and led to the more
  careful, still-correct invariant.
- `RB-PHYSICS-001-FR-028` (car actually driving into a goal) — closes the
  last goal-related Non-goal repeated across FR-024 through FR-027: a car
  (box) can now actually drive through a goal-mouth window, not just the
  ball. New `collision::box_vs_goal_wall` tests each of a box's 8 corners
  individually against `StaticGoalWall::contains_in_window` — a corner
  whose own projection falls inside the window contributes no contact at
  all, the same pass-through rule `sphere_vs_goal_wall` already applies to
  the ball's single center point, applied per corner instead. A corner
  outside the window behaves exactly like an ordinary `box_vs_plane`
  corner test. `contacts_vs_goal_wall` now dispatches a `Shape::Box` to
  `box_vs_goal_wall` instead of falling through to an unwindowed
  `contacts_vs_plane`. No `PhysicsWorld::step` changes needed — exactly
  like FR-027's own discovery, `resolve_goal_wall_contact` was already
  being called for every car in the scene (it always needed the wall's
  plain-plane collision even before this fix). A real emergent behavior
  worth noting: because each corner is tested independently, a car only
  partly lined up with the window gets a genuine partial block — the
  corners still outside it register contacts and stop the car there,
  while the corners inside register none — rather than the all-or-nothing
  result a single-point sphere test necessarily produces. Still not
  modeled: a modeled goal interior/net — the goal opens onto open,
  unbounded space beyond the back wall for a car now too, not a bounded
  volume. 3 net new/replaced unit tests across `collision.rs`/`world.rs`
  in `rb_physics_bullet` (221 total): `collision.rs` replaced its old
  "box vs. goal window ignores the window entirely" regression test with
  three proofs (a box squarely inside the window has no contact, a box
  straddling the window's own edge collides only on the corners still
  outside it, and a box entirely outside the window behaves like an
  ordinary plane); `world.rs` replaced
  `a_car_is_still_stopped_by_the_standard_arenas_back_wall_at_the_goal_mouth`
  (whose entire premise this increment reverses) with a live end-to-end
  proof that a car fired at the goal-mouth center actually passes the back
  wall (mirroring the ball's own equivalent proof, same 1.8s
  flight-duration bound for the same pre-existing `StaticQuarterPipe`
  infinite-axis reason), plus a regression guard confirming a car aimed at
  the solid part of the wall is still stopped by it.
- `RB-PHYSICS-001-FR-029` (modeled goal interior) — closes the "a ball or
  car passes into open, unbounded space" gap repeated across FR-024
  through FR-028's own "Still not modeled" lists: a ball or car passing
  through a goal-mouth window now settles inside a bounded goal box
  instead of flying forever. New `body::StaticBoundedWall` collides only
  *within* a rectangular bound — the opposite gate from `StaticGoalWall`'s
  window (solid everywhere *except* inside a rectangle) — with new
  `collision::sphere_vs_bounded_wall`/`box_vs_bounded_wall`/
  `contacts_vs_bounded_wall` dispatching by shape, the box path using the
  same "test every corner" technique FR-027/FR-028 established. New
  `arena::standard_goal_back_walls` (2 plain, unbounded `StaticPlane`s,
  `GOAL_DEPTH` behind each real back wall — deliberately unbounded, since
  nothing can reach that plane except by first passing through the window)
  plus `arena::standard_goal_side_walls`/`standard_goal_roofs` (4 and 2
  bounded walls, reusing `goal_post_plane`/`goal_crossbar_plane` completely
  unchanged, bounded to the goal's own depth/width/height footprint — an
  unbounded plane at either position would incorrectly wall off the entire
  main field, the same problem those planes' own pre-existing doc comments
  already documented). `PhysicsWorld` gains `bounded_walls`/
  `with_bounded_wall`, resolved for the ball and every car like
  `goal_walls`. Two real test-design findings worth keeping: the 3 new live
  end-to-end proofs are deliberately isolated to a minimal scene built from
  just the new wall(s) under test, not the full `PhysicsWorld::standard_arena`
  — using the full arena, a ball fired sideways or upward from deep inside
  the goal box got flung to wildly wrong positions, root-caused to the
  pre-existing "a `StaticQuarterPipe`'s sector-membership test only checks
  angle, not radial distance" limitation, spuriously triggered by the
  standard arena's own goal-cutout-edge fillets sitting near the window;
  separately, an early version zeroed only the ball's own restitution and
  got nondeterministic results, since the wall's own default 0.5
  restitution still applied in the solver — fixed by zeroing the wall's
  restitution too. Still not modeled: a genuine net *mesh* — this models a
  solid bounding volume standing in for the net's functional role, not
  springy/catching netting or a real net's own visual sag. 21 net new/
  renamed unit tests across `body.rs`/`collision.rs`/`arena.rs`/`world.rs`
  in `rb_physics_bullet` (242 total): 4 in `body.rs` for
  `contains_in_bound` (mirroring `StaticGoalWall::contains_in_window`'s own
  tests with the gate inverted), 5 in `collision.rs` against a synthetic
  fixture, 8 in `arena.rs` proving the new geometry functions place things
  correctly, and 4 in `world.rs` (1 wiring-count check plus the 3 live
  end-to-end proofs described above, plus renaming the pre-existing
  wall-count test to account for the 2 new back-of-net planes).
- `RB-PHYSICS-001-FR-030` (combined multi-body solve) — closes the "3+
  bodies mutually touching in the same step" approximation tracked since
  multi-car support first landed: `PhysicsWorld::step` now resolves every
  ball-vs-car and car-vs-car contact manifold together, instead of
  resolving each pair independently (its own full `SOLVER_ITERATIONS`
  pass, fully applied) before the next pair's setup even reads a body's
  velocity. New `solver::resolve_dynamic_manifolds` gives every body index
  that takes part in at least one manifold its own `DeltaVelocity`
  accumulator, shared across every manifold that body is in for the whole
  solve — a real shared island solve. New helper `delta_pair_mut`
  generalizes the `Vec::split_at_mut` disjoint-borrow trick the car-vs-car
  loop already used (previously adjacent indices only) to arbitrary index
  pairs. The old `TwoBodyDelta` struct is gone; `resolve_two_body_row` now
  takes each body's `DeltaVelocity` separately, which is what makes
  sharing one accumulator across manifolds possible. Static contacts
  (ground, arena walls, curves, corner fillets, goal walls, bounded walls)
  are deliberately unchanged — a body's contact with static geometry never
  depends on another dynamic body, so resolving it independently loses no
  information. Measured, not just assumed: a left-right symmetric "pinch"
  (a ball exactly touching two identical, much heavier cars closing in
  from opposite sides at equal speed, restitution zero) has a true
  simultaneous-solve answer of all three bodies ending near zero velocity
  (total momentum is exactly zero). Resolving each pair independently left
  the ball at ~99% of a single car's own closing speed, as if the
  first-resolved contact's effect was almost entirely discarded by the
  second; the combined solve, at this crate's existing 10 solver
  iterations, leaves the ball measurably slower (~89.5 vs. ~98.9 units/s)
  but doesn't fully converge to zero that quickly — a known, common
  Gauss-Seidel limitation for a light body sandwiched between two much
  heavier ones (confirmed, not shipped, by checking that far more
  iterations converge the combined solve much closer to zero, while the
  independent-pairwise result never changes regardless of iteration
  count — proof the old approach's error was structural, not an
  iteration-count shortfall). 2 new tests
  (`solver::tests::resolve_dynamic_manifolds_keeps_more_of_every_bodys_contact_than_resolving_pairs_independently`,
  `world::tests::a_ball_pinched_between_two_closing_cars_is_resolved_by_a_shared_multi_body_solve`),
  244 total in `rb_physics_bullet` (+2 over FR-029's 242).
- `RB-PHYSICS-001-FR-031` (constant-calibration audit — does NOT close
  `FR-005`) — `FR-005`'s real calibration against recorded ground truth
  stays blocked on `PHASE-0-EXIT`; this narrower requirement sources
  every uncalibrated placeholder constant in `drive.rs`/`arena.rs` against
  the community reverse-engineering effort instead: the RocketSim
  (`ZealanL/RocketSim`) and RLUtilities (`samuelpmish/RLUtilities`) source
  code plus the RLBot wiki's "Useful Game Values" page — three
  independently-written references, agreement across all three treated as
  high confidence. Corrected with code changes: `drive::JUMP_SPEED`
  (`292.0` → `875.0/3.0`, ≈291.667 uu/s) and
  `drive::JUMP_HOLD_ACCELERATION` (`1400.0` → `4375.0/3.0`, ≈1458.33
  uu/s²) to their precise real values; split `drive::MAX_CAR_SPEED` (2300,
  boost's own cap, confirmed correct) from a new
  `drive::UNBOOSTED_MAX_CAR_SPEED` (1410, throttle's own cap) — a real
  behavioral fix, since throttle alone could previously reach the boosted
  top speed. Confirmed already correct, no change: `JUMP_HOLD_MAX_DURATION`
  (0.2), `BOOST_ACCELERATION` (991.667), `MAX_BOOST` (100), gravity (-650),
  `GOAL_DEPTH` (880). Explicitly flagged as audited-but-still-uncalibrated
  (a real reference exists but doesn't safely port into this port's own
  unit system/mechanic shape, or no reference exists at all):
  `DODGE_SPEED`, `DODGE_ANGULAR_SPEED`, `WALL_JUMP_HORIZONTAL_SPEED`,
  `STEER_TORQUE`, `AIR_CONTROL_TORQUE`, `HANDBRAKE_FRICTION_MULTIPLIER`,
  `LANDING_AUTO_UPRIGHT_TORQUE`, `FILLET_RADIUS`, `CORNER_ARCH_RADIUS`.
  Surfaced two open ambiguities without acting on them (ball radius 91.25
  vs. this port's 92.75; `CEILING_Z` 2044 vs. RocketSim's cited 2048) —
  recorded as open questions rather than guessed at. 1 new test
  (`drive::tests::throttle_alone_cannot_reach_the_boosted_top_speed`), 245
  total in `rb_physics_bullet` (+1 over FR-030's 244).
- `RB-PHYSICS-001-FR-032` (genuine convex-vs-curved-surface narrow phase
  investigation, resolved — no code change to the narrow phase itself) —
  set out to replace `box_vs_quarter_pipe`/`box_vs_corner_fillet`'s
  per-corner technique with a real GJK/EPA convex-vs-convex narrow phase,
  on the strength of a limitation FR-027's own doc comments claimed: a
  box face resting flush against a shallow curve could have every corner
  still clear of the fillet while the face's middle already overlapped
  it, under-detecting that case. Building the replacement (a from-scratch
  GJK closest-points implementation) and swapping it in broke two
  pre-existing, previously-passing end-to-end tests, because it answered
  a different question than the one this contact needs: a
  quarter-pipe/corner-fillet's contact test is a *containment* question
  (is the box's farthest point from the axis/center at or beyond radius),
  not a nearest-point one, and distance-from-a-line/point is a convex
  function whose maximum over a convex polytope (the box) is always
  attained at a corner — so the original per-corner technique is
  mathematically exact for this question, not an approximation. Reverted
  `box_vs_quarter_pipe`/`box_vs_corner_fillet` to their original FR-027
  implementations and deleted the now-unused GJK module entirely,
  correcting every doc comment across the crate and this spec that had
  inherited FR-027's unverified claim. 1 new test
  (`collision::tests::no_point_on_a_boxs_face_is_ever_farther_from_a_quarter_pipes_axis_than_its_own_corners`,
  densely sampling all 6 faces of a car-sized box against the exact
  geometry the two broken tests used), 246 total in `rb_physics_bullet`
  (+1 over FR-031's 245).
- `RB-PHYSICS-001-FR-033` (genuine net mesh, implemented, ball only) —
  closes the "genuine net mesh" Non-goal `FR-029`'s own doc comment left
  open. New module `net` (`net::NetMesh`): a rectangular mass-spring grid
  of point masses (each a real `RigidBody::sphere`, tiny and light,
  reusing this crate's existing rigid-body/collision/solver machinery
  rather than a bespoke penalty-force system), every perimeter point
  anchored (fixed, representing attachment to the rigid goal frame) and
  every interior point free, connected by structural (horizontal/vertical)
  and shear (diagonal) springs (Hooke's law plus velocity damping — the
  one genuinely new piece of physics math this requirement adds).
  `NetMesh::step` sub-steps its own internal physics for numerical
  stability and resolves the ball's contact against every free point it
  overlaps via a new `collision::sphere_vs_sphere` (this crate's first
  real sphere-vs-sphere contact — previously an unimplemented, callerless
  placeholder) plus the *existing* `solver::resolve_contacts_between`
  two-body path. New `arena::standard_nets` builds one `net::NetMesh` per
  goal, `NET_DEPTH` behind the real back wall and well in front of
  `FR-029`'s own rigid back-of-net plane (unchanged, still a car's real
  backstop — a car isn't tested against the net at all, a documented
  Non-goal). `PhysicsWorld` gains `nets`/`with_net`, resolved after every
  other contact each step. Every new constant is an uncalibrated
  placeholder — real Rocket League net material properties have never been
  published. 10 new tests: 5 in `net.rs` (perimeter anchoring, zero-stretch
  springs at rest, anchored points immovable under gravity, an undisturbed
  net settling instead of oscillating forever, and the real catching proof
  — a ball fired at the net's own center loses over half its speed within
  1 second compared to free flight); `collision.rs` replaced the old
  `contacts_between_two_spheres_is_empty` regression test with 2 proving
  `sphere_vs_sphere`'s own correctness (net +1); 2 in `arena.rs`; 2 in
  `world.rs` (a wiring-count test plus the real live end-to-end proof — a
  ball fired at a lone net panel in an isolated minimal scene loses at
  least half its speed compared to the identical shot with no net
  present). 256 total in `rb_physics_bullet` (+10 over FR-032's 246).
- `RB-PHYSICS-001-FR-034` (split impulse, implemented) — closes the
  "no split impulse" half of the solver's documented simplification gap,
  leaving only warm-starting/sleeping open. `ConstraintRow`/`TwoBodyRow`
  (`solver.rs`) each split their normal row's combined penetration+velocity
  `rhs` term into two independent fields; a new, entirely separate "push"
  pseudo-velocity channel (`resolve_push_row`/`resolve_two_body_push_row`)
  is now solved alongside the real one every iteration, fed only by a
  contact's positional (penetration/ERP) error, never its velocity/
  restitution error. After each manifold's iterations finish, the real
  delta still updates the body's velocity exactly as before, and the new
  push delta is applied directly to the body's position/orientation via a
  new `apply_push_delta` (built on the existing
  `integrate::integrate_transform`) — mirroring Bullet's own
  `btSolverBody::writebackVelocity`. Wired into `resolve_contacts`,
  `resolve_contacts_between`, and `resolve_dynamic_manifolds` with zero
  call-site changes anywhere outside `solver.rs`. 2 new `solver.rs` tests
  directly prove a deeply-penetrating, at-rest contact now leaves near-zero
  real velocity while the body/bodies' positions measurably separate; 4
  pre-existing `world.rs` live end-to-end fillet tests, which had encoded
  the old pre-split-impulse "coasts past the resting distance under
  residual velocity" behavior in their own assertions, were tightened to
  check settling at (not past) the resting distance instead — a stronger
  proof this fix is real, not just internally self-consistent. 258 total
  in `rb_physics_bullet` (+2 over FR-033's 256).
- `RB-PHYSICS-001-FR-035` (warm-starting, implemented for
  `resolve_dynamic_manifolds` only) — a new `solver::ContactCache` carries
  a manifold's converged real-channel impulses from one call to the next,
  matched by each contact's approximate world position. A new
  `warm_start_two_body_row` applies each row's cached impulse directly to
  the manifold's shared `DeltaVelocity` accumulators before iterating —
  merely setting `TwoBodyRow::applied_impulse` would do nothing on its own
  here (`GLOBAL_CFM` is always `0.0`), so the seed has to be baked into
  the starting delta itself, mirroring Bullet's own warm-start applying
  the cached impulse to the solver body's temporary velocity at setup,
  before any iteration runs. `resolve_dynamic_manifolds` gained a new
  `caches` parameter (one `ContactCache` per body-index pair); every call
  rebuilds it from only that call's manifolds, so a pair no longer
  touching drops automatically. `PhysicsWorld` gains one persistent
  `dynamic_manifold_caches` field. Deliberately scoped to this one call
  site: `resolve_contacts`/`resolve_contacts_between` (every
  static-geometry contact) stay un-warm-started, since this port's fixed
  `SOLVER_ITERATIONS` already fully converges every one-body/two-body
  scenario this crate tests — warm-starting has no scenario to
  demonstrate value against there yet, unlike `resolve_dynamic_manifolds`,
  which already had FR-030's own documented extreme-mass-ratio
  "sandwiched" case that doesn't fully converge within one call. 1 new
  `solver.rs` test reuses that exact scenario across two calls (cold, then
  warm vs. a repeated cold from the identical post-call-1 state) and shows
  the warm run lands measurably closer to the true zero-velocity
  equilibrium. This does NOT fix the still-open "bouncy resting contact
  never settles" limitation — that comes from restitution re-triggering
  off a fresh gravity-induced closing velocity every frame, independent of
  where the solver starts; sleeping (still unimplemented) is the actual
  fix. 259 total in `rb_physics_bullet` (+1 over FR-034's 258).
- `RB-PHYSICS-001-FR-036` (ball radius / `CEILING_Z` constant-ambiguity
  resolution, implemented) — a dedicated follow-up to FR-031's own audit,
  resolving the two genuine ambiguities it surfaced but deliberately didn't
  act on, using real source-level research (RocketSim's and RLUtilities'
  own source, and the current RLBot wiki, read directly rather than
  guessed at). Ball radius: FR-031 had framed this as "`92.75` vs.
  `91.25`", but the real games actually split the ball into a smaller
  inertia radius (`91.25`) and a distinctly larger collision radius
  (`93.15`, the mesh's own collision margin) — a split this port's single
  unified radius field can't represent, and since this port has no
  separate collision margin of its own, the collision radius is the
  correct single-constant analog. Every `92.75` literal across
  `solver.rs`/`world.rs`/`net.rs`/`collision.rs` became `93.15`, not
  `91.25`. `arena::CEILING_Z`: confirmed, via both RocketSim's
  `ARENA_HEIGHT = 2048.f` and an independent reconstruction from real
  extracted collision-mesh geometry, to share the same reference point, so
  `2044.0` became `2048.0`. Also corrected two mis-documented claims (not
  new findings): `arena::CORNER_LENGTH` and `arena::GOAL_DEPTH` were
  wrongly described as uncalibrated placeholders — both are confirmed
  exact, so only their doc comments changed, not their values.
  `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS` remain untouched and still
  genuinely uncalibrated (no analytic reference exists for either — a
  separate mesh-ingestion follow-up, deliberately left for later). No new
  tests, matching FR-031's own precedent for constant-only corrections; all
  259 pre-existing tests pass unchanged (total unchanged from FR-035).
- `RB-PHYSICS-001-FR-037` (sleeping, implemented) — closes the "no
  sleeping" half of the solver's documented gap FR-035 left open (FR-035
  closed the warm-starting half). New `body::RigidBody` fields
  `is_sleeping`/`sleep_timer` and two methods:
  `update_sleep_state(&mut self, dt)` — called for the ball and every car
  once every other contact each step is resolved but before the transform
  integrates — forcibly zeroes a body's velocity once it's stayed below
  both a linear and an angular threshold for a sustained time, fixing the
  "bouncy resting contact never settles" limitation that neither split
  impulse nor warm-starting could (restitution re-triggers off a fresh
  gravity-induced closing velocity every frame regardless of where the
  solver's iteration starts); and `wake(&mut self)`, called unconditionally
  by `drive::apply_driven_forces` whenever a car's `ControllerInput` is
  genuinely active, before that input's own force has had a chance to move
  it — necessary since a resultant-velocity-only wake check would zero
  right back out a driving force whose one-frame delta is itself smaller
  than the sleep threshold, permanently stranding an asleep car. All three
  new threshold constants are uncalibrated placeholders (no public
  reference for what, if any, real Rocket League's own engine uses
  internally here). 8 new tests (5 in `body.rs`, 3 in `world.rs`,
  including a direct demonstration that a nonzero-restitution resting ball
  now actually falls asleep at exactly zero velocity instead of bouncing
  forever); all pre-existing tests pass unchanged. 267 total in
  `rb_physics_bullet` (+8 over FR-036's 259).
- `RB-PHYSICS-001-FR-038` (car-vs-net contact, implemented) — closes this
  port's own former Non-goal that a car passes straight through a
  `net::NetMesh`'s spatial footprint untouched. `net::NetMesh::step`
  changed from a single `&mut RigidBody` (the ball alone) to `&mut
  [RigidBody]` (every body that can touch the net); no new collision code
  was needed, since `collision::contacts_between` already dispatches to
  `sphere_vs_box` for a car against a net point the same way it always has
  for ball-vs-car. `PhysicsWorld::step` reuses the same ball-plus-cars
  snapshot `solver::resolve_dynamic_manifolds` already resolved that step
  for the net-step call too. All of `net.rs`'s pre-existing tests updated
  only their call syntax, not their own assertions. 3 new tests (2 in
  `net.rs`, 1 in `world.rs` — the live-`PhysicsWorld` "caught vs. free
  flight" proof mirroring the ball's own version); all pre-existing tests
  pass unchanged. 271 total in `rb_physics_bullet` (+3 over FR-039's 268).
- `RB-PHYSICS-001-FR-039` (wall-jump corner disambiguation, implemented) —
  closes the "first wall in `self.walls`" simplification FR-013 originally
  documented, made reachable in the standard arena for the first time by
  FR-019's diagonal corner walls. `PhysicsWorld::step`'s per-car wall-normal
  computation now sums every wall a car is touching this step and
  normalizes the result, instead of picking whichever wall comes first — a
  car touching exactly one wall is unaffected (summing a single unit vector
  and normalizing it is a no-op), a car touching two walls at a corner now
  pushes off diagonally, blending both, instead of firing along only one of
  them depending on iteration order. No new collision code needed —
  physical contact resolution already handled simultaneous multi-wall
  contact correctly; only the wall-jump push-off direction picker was
  affected. 1 new `world.rs` test (a car touching two perpendicular walls
  at once, asserting the push-off comes out diagonal); all pre-existing
  tests pass unchanged. 268 total in `rb_physics_bullet` (+1 over FR-037's
  267).
- `RB-PHYSICS-001-FR-040` (fillet-radius calibration research, investigated)
  — a dedicated research pass, matching FR-036's own real-source-research
  method, targeting the two uncalibrated placeholder constants FR-036
  itself left untouched: `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS`.
  Searched RocketSim/RLUtilities source, the RLBot wiki, and RLGym's game
  values; found exactly one candidate, the RLBot wiki's uncited "wall
  bottom ramp radius: approx. 256, not circular". Deliberately not adopted
  — no citation, doesn't distinguish the two constants' distinctly
  different radii, explicitly disclaims being circular, and shares its
  numeral with RLGym's unrelated `RAMP_HEIGHT` (a ramp's height, not a
  curve's radius), suggesting a possible wiki conflation rather than an
  independent measurement. Both constants remain unchanged and genuinely
  uncalibrated; closing this for real needs actual extracted mesh data
  (e.g. via `RLArenaCollisionDumper`), the same Windows/Rocket League
  environment blocker `RB-VERIFY-002-FR-001` already documents. No new
  tests (documentation-only, no value changed); all pre-existing tests
  pass unchanged. 271 total in `rb_physics_bullet` (unchanged from
  FR-038).
- `RB-PHYSICS-001-FR-041` (sandwiched-solve convergence, implemented) —
  investigated whether anything short of real recorded data could narrow
  FR-030's own documented extreme-mass-ratio "sandwiched"
  under-convergence gap at this crate's fixed `SOLVER_ITERATIONS = 10`. A
  naive global SOR-style relaxation factor was tried first: factors above
  1.0 made FR-030's own symmetric-pinch scenario measurably diverge,
  while factors below 1.0 monotonically improved it — matching standard
  PGS/SOR theory for a tightly-coupled multi-constraint body.
  `solver::resolve_dynamic_manifolds` now scales each manifold's
  velocity-row impulse by a parameter-free `1 / k` instead (`k` = the
  number of manifolds sharing a body this step) — mathematically
  dominant rather than a tuned magic number, so unlike raising
  `SOLVER_ITERATIONS` it needed no real data to justify adopting.
  Narrows FR-030's own result from ~89.5 to ~32 units/s at zero added
  iteration cost; a body touched by only one other body this step
  (`k == 1`) is a mathematical no-op, confirmed by a dedicated
  bit-for-bit-equivalence test. Does not achieve full convergence within
  one call's fixed `SOLVER_ITERATIONS` — real recorded multi-car contact
  data would still be needed for that. 2 new tests, 273 total in
  `rb_physics_bullet` (+2 over FR-040's 271).
- `RB-PHYSICS-001-FR-042` (box-vs-box reference validation, investigated)
  — fetched and read Bullet's own `btBoxBoxDetector::dBoxBox` reference
  source directly to validate two "reasonable, tested choices, never
  validated against the reference" this spec's own Open Questions
  flagged. (1) Edge-edge contact point: confirmed this port's finite-segment
  closest-point derivation (Ericson's construction) is strictly more
  rigorous than the reference's own unclamped-infinite-line
  `dLineClosestApproach` — a genuine improvement, not merely equivalent.
  (2) Face-clipping degenerate fallback: confirmed the reference contains
  the exact same undocumented "should never happen" judgment call this
  port's own comment already made, with this port's own choice to
  synthesize a contact rather than drop it (as the reference does)
  confirmed a deliberate, favorable divergence. (3) A candidate fix for
  the edge-edge tangent sign-selection heuristic (swap the center-to-center
  vector for the SAT-resolved normal, matching the reference's own
  approach) was built and empirically tested against a brute-force ground
  truth across 50,000 randomized configurations, found genuinely mixed
  (better for realistic shallow penetration, worse for deep penetration,
  neither reliably optimal) — not adopted. No new tests (documentation-only,
  no value or behavior changed); all 273 pre-existing tests pass unchanged.
- `RB-PHYSICS-001-FR-043` (restitution/friction combine-mode reference
  validation, investigated) — this spec's own Open Questions claimed,
  without ever having checked, that Bullet's default combine mode is `max`
  for both restitution and friction. Fetched and read
  `btManifoldResult.h`/`btManifoldResult.cpp` in full and found that claim
  wrong: the real default for both is an unclamped product (`a * b`;
  friction additionally clamps to `[-10, 10]`), with no `max` mode
  anywhere in the reference. This port's own average combine mode is kept
  anyway, now for a correct reason: it preserves the identity
  `combine(a, a) == a`, which the reference's real product does not
  (`0.5 * 0.5 == 0.25`), and most bodies here currently share the same
  uncalibrated placeholder `0.5` coefficient. Corrected the wrong claim
  everywhere it appeared (spec, `solver.rs`, `body.rs`). 2 new tests pin
  `combine_restitution`/`combine_friction`'s own identity-preserving
  behavior directly; all 273 pre-existing tests pass unchanged. 275 total
  in `rb_physics_bullet` (+2 over FR-042's 273).
- `RB-PHYSICS-001-FR-044` (stale Non-goals correction, investigated) —
  this spec's own top-level "Non-goals (this increment)" section still
  carried a "Split impulse. This port always takes Bullet's non-split
  contact-resolution branch" bullet, contradicted by
  `RB-PHYSICS-001-FR-034`'s own already-shipped implementation (its own
  Requirements entry, the version 0.34.0 Change History entry, and
  `rb_physics_bullet::solver`'s own module doc comment all already
  correctly describe split impulse as implemented — only this one
  Non-goals bullet had never been updated). Confirmed the implementation
  is genuinely present by locating `solver::resolve_push_row`/
  `resolve_two_body_push_row`/`apply_push_delta` directly in `solver.rs`,
  and confirmed via a repo-wide `grep` that this was the only stale
  occurrence anywhere in code or docs. Corrected the bullet to a
  strikethrough-and-close note, matching the same convention this section
  already uses for its own two other resolved Non-goals items. Zero
  production code changed. No new tests (documentation-only); all 275
  pre-existing tests pass unchanged.
- `RB-PHYSICS-001-FR-045` (`integrate.rs` reference validation,
  investigated) — fetched and read Bullet's real `btRigidBody.cpp`/`.h`,
  `btTransformUtil.h`, `btQuaternion.h`, and `btScalar.h` to check every
  Bullet-reference claim `integrate.rs`'s own doc comments make. Confirmed
  `apply_damping`'s "Bullet's default" claim and exact formula,
  `integrate_velocities`'s `MAX_ANGVEL` constant and clamp formula, and
  `integrate_transform`'s `ANGULAR_MOTION_THRESHOLD`/Taylor
  coefficient/sinc formula all byte-for-byte accurate. Found one minor
  numeric difference (this port's degenerate-quaternion guard uses
  `1e-12`, the reference's own `SIMD_EPSILON` is `FLT_EPSILON` — ~5 orders
  of magnitude larger) — not adopted, behaviorally indistinguishable for
  every reachable scenario. Found one more significant thing: this
  function's own check-then-normalize fallback isn't defensive theater —
  it matches Bullet's real fallback choice (preserve the prior
  orientation, never reset to identity), which an unconditional
  `Quat::normalize` call would have silently gotten wrong. 1 new test
  pins this distinction directly; all 275 pre-existing tests pass
  unchanged. 276 total in `rb_physics_bullet` (+1 over FR-044's 275).
- `RB-PHYSICS-001-FR-046` (`body.rs`/`mat3.rs` reference validation,
  investigated) — fetched and read Bullet's real `btSphereShape.cpp`,
  `btBoxShape.cpp`, `btRigidBody.cpp`/`.h`, and `btMatrix3x3.h` to check
  every Bullet-reference claim `body.rs`'s `Shape::local_inertia`/
  `RigidBody::update_inertia_tensor` and `mat3.rs`'s
  `Mat3::scaled_columns`/`Mat3::from_quat` make. Confirmed the
  sphere/box local-inertia formulas, `update_inertia_tensor`'s matrix
  formula, and `Mat3::scaled_columns`'s per-column scaling all
  byte-for-byte accurate. Found one genuine difference:
  `Mat3::from_quat` hardcodes an `s = 2` factor assuming an exactly
  unit-length input quaternion, while the reference's own
  `btMatrix3x3::setRotation` self-corrects for a non-unit-length input
  via `s = 2 / q.length2()` — not adopted, since this function's only
  production call site always receives an already-renormalized
  orientation (per FR-045's own finding), making the reference's own
  self-correction unreachable here. 1 new test pins this distinction;
  all 276 pre-existing tests pass unchanged. 277 total in
  `rb_physics_bullet` (+1 over FR-045's 276).
- `RB-PHYSICS-001-FR-047` (`collision.rs` remaining closed-form shape
  pairings reference validation, investigated) — fetched and read
  Bullet's real `btConvexPlaneCollisionAlgorithm.cpp`/`.h`,
  `btSphereBoxCollisionAlgorithm.cpp`, `btSphereSphereCollisionAlgorithm.cpp`,
  and `btManifoldPoint.h` to check every Bullet-reference claim
  `sphere_vs_plane`, `box_vs_plane`, `sphere_vs_box`, and `sphere_vs_sphere`
  make (`box_vs_box` was already checked this way, FR-042). Confirmed
  `sphere_vs_plane` and `sphere_vs_sphere` exact, and `sphere_vs_box`'s
  deep-penetration face selection confirmed to reproduce Bullet's own
  exact `+x, -x, +y, -y, +z, -z` face-check tie-break order, not just a
  mathematically-equivalent alternative. Found one genuine, deliberate
  divergence: real `btConvexPlaneCollisionAlgorithm` generates only one
  contact point per frame via a single GJK support query (its own
  multi-point "perturbation" path is configured off by Bullet's own real
  default), relying on several frames of persistent-manifold accumulation
  to reach a resting box's full 4-corner manifold, where `box_vs_plane`
  computes all 4 corners exactly in one pass — not adopted, confirmed a
  favorable divergence in the same spirit as FR-042's `box_vs_box`
  finding. 1 new test pins the exact tie-break-order match; all 277
  pre-existing tests pass unchanged. 278 total in `rb_physics_bullet`
  (+1 over FR-046's 277).
- `RB-PHYSICS-001-FR-048` (`solver.rs` constraint-row setup/resolve
  reference validation, investigated) — fetched and read Bullet's real
  `btSequentialImpulseConstraintSolver.cpp`/`.h`, `btContactSolverInfo.h`,
  and `btVector3.h` to check every Bullet-reference claim
  `restitution_curve`, `plane_space`, `setup_rows`, and `resolve_row`
  make. Confirmed `plane_space` byte-for-byte exact against real
  `btPlaneSpace1`; `restitution_curve` behaviorally exact (its `.max(0.0)`
  folds in a clamp real Bullet applies at its one call site instead);
  `setup_rows`'s normal/friction row formulas exact against real
  `setupContactConstraint`/`setupFrictionConstraint` (correcting a stale
  citation to an unrelated function); `resolve_row`'s single unified
  two-bound resolver behaviorally equivalent to Bullet's own two separate
  resolvers; and all 6 of `btContactSolverInfo`'s cited defaults exact.
  Found one genuine, significant divergence, not adopted: this port
  always derives both friction directions from a fixed,
  velocity-independent basis, while real Bullet's actual default aligns
  friction direction 1 with the tangential component of the current
  relative sliding velocity — a fixed two-axis friction limit can
  over/under-estimate the true circular friction cone by up to `sqrt(2)`
  relative to the real slide direction, flagged as open follow-up work
  for a dedicated future FR (the same scoping already used for
  FR-030/FR-034/FR-035/FR-037) rather than folded into this pass. 1 new
  test pins the `restitution_curve`/call-site-clamp equivalence; all 278
  pre-existing tests pass unchanged. 279 total in `rb_physics_bullet`
  (+1 over FR-047's 278).

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
## Next

1. `RB-VERIFY-002-FR-001` — write, build, and run the BakkesMod-side
   capture plugin against ADR-0005's JSON-Lines format, on the owner's own
   Windows/BakkesMod/game environment (this sandbox can't).

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (323 tests: 23 in `rb_domain`, 273 in
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
