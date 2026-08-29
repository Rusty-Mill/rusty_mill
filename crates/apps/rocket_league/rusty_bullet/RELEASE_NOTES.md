# Release Notes

Tracks notable changes to this repo, one entry per merged change against
`main`, reverse chronological. Pre-1.0, no version tags yet — entries are
keyed by the commit/PR that shipped them.

---

## Jump input
**2026-08-29** · [#29](https://github.com/baileyrd/rusty_bullet/pull/29) · `689b006`

- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains a
  single ground jump (`RB-PHYSICS-001-FR-010`) — a fixed `JUMP_SPEED`
  instantaneous upward velocity change (via `RigidBody::apply_impulse`,
  not a continuous force) fired on the *rising edge* of
  `ControllerInput.jump` while the car is grounded — a fresh press, not
  merely held.
- **Edge detection:** holding jump through the resulting airborne period
  doesn't re-fire it, and releasing then re-pressing while still airborne
  doesn't fire it either (this increment has no double jump to grant).
  `PhysicsWorld` gains a parallel `car_jump_held: Vec<bool>` (starting
  `false`, kept in lockstep with `cars` by `with_car`) carrying "was jump
  held as of the previous step" across calls — the same pattern
  `boost_amount` already uses for cross-call resource state.
- **Constants, honestly labeled:** `JUMP_SPEED` (292 uu/s) is a
  commonly-cited community number, applied as a flat velocity change
  regardless of the car's mass (matching how the real jump impulse
  doesn't scale with mass either).
- **Not implemented** (explicitly, not silently dropped): double
  jump/dodge (a second airborne jump, usually paired with a directional
  impulse/torque), variable jump height (real Rocket League adds extra
  upward accel for as long as jump is held, up to a cap — this port
  always applies the same fixed impulse), wall jump (needs arena walls,
  out of scope), and air control (pitch/yaw/roll torque while airborne) —
  each a distinct real mechanic, tracked as separate follow-up work.
- 6 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet` (92
  total): jump gives a grounded car upward velocity, has no effect while
  airborne, doesn't re-fire on a second call while still held, and fires
  again after a release-then-re-press, plus — the real end-to-end proof —
  a car with jump input in a live `PhysicsWorld::step` loop actually
  leaves the ground, and a regression test confirming that holding jump
  for a car's entire flight (never released) lets it land and settle
  instead of being relaunched on touchdown.

---

## Handbrake input
**2026-08-29** · [#27](https://github.com/baileyrd/rusty_bullet/pull/27) · `56f9cb4`

- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains a
  handbrake mechanic (`RB-PHYSICS-001-FR-009`) — while
  `ControllerInput.handbrake` is held and the car is grounded (gated like
  throttle/steering — a free-floating box has no wheels to lock), the
  car's `RigidBody.friction` is temporarily multiplied by a new
  `HANDBRAKE_FRICTION_MULTIPLIER`, letting the car's existing momentum
  carry it into a slide instead of gripping the ground and turning
  cleanly. Releasing handbrake restores the car's own friction.
- **Design note:** this reuses the ground-contact solver's existing
  Coulomb-friction machinery rather than inventing a separate lateral-slip
  system — this port has no per-wheel tire model, so there's no
  rear-specific grip to lose the way a real car's handbrake works. A
  uniform, temporary reduction of the whole car's one friction value is a
  deliberately simple stand-in, not a claim of mechanistic fidelity.
- **Added:** `PhysicsWorld` gains a parallel `car_base_friction: Vec<f32>`,
  snapshotted from each car's own constructed `friction` by `with_car`, so
  handbrake restores the car's own base value on release — not some
  crate-wide default, even when a car was built with a custom friction.
- **Constants, honestly labeled:** `HANDBRAKE_FRICTION_MULTIPLIER` is an
  uncalibrated placeholder with no public reference at all (like
  `STEER_TORQUE`), chosen only to produce a visibly reduced (not zero)
  grip in tests.
- **Not implemented** (explicitly, not silently dropped): jump and air
  control (pitch/yaw/roll torque while airborne) — each a distinct real
  mechanic, tracked as separate follow-up work.
- 5 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet` (86
  total): handbrake reduces friction while grounded, has no effect while
  airborne, and releasing it restores the car's base friction; releasing
  handbrake restores a car's own *non-default* base friction (not a
  hardcoded constant); and — the real end-to-end proof — a car already
  sliding sideways in a live `PhysicsWorld::step` loop retains more of
  that slide under handbrake's reduced friction than under normal grip.

---

## Boost input
**2026-08-29** · [#25](https://github.com/baileyrd/rusty_bullet/pull/25) · `40e70cd`

- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains a boost
  force (`RB-PHYSICS-001-FR-008`) — a flat forward force
  (`BOOST_ACCELERATION * mass`, not speed-tapered like throttle, capped at
  the same `MAX_CAR_SPEED` ceiling) applied whenever
  `ControllerInput.boost` is set and the car has boost remaining. Unlike
  throttle and steering, boost is **not** gated on ground contact — it's
  modeled as a rocket, not an engine, so it works identically airborne,
  matching real Rocket League.
- **Added:** `PhysicsWorld::set_car_boost`, setting a car's current boost
  amount directly. `PhysicsWorld` gains a parallel `car_boost: Vec<f32>`
  (kept in lockstep with `cars` by `with_car`, starting at a full tank —
  `drive::MAX_BOOST`). Holding boost input drains the tank at
  `BOOST_CONSUMPTION_RATE` per second whenever held, even once the forward
  force itself stops applying at `MAX_CAR_SPEED` — matching real Rocket
  League's "holding boost drains fuel regardless of whether it's still
  accelerating you" — clamping at zero (no effect once empty).
- **Changed:** `frame()` now reports each car's actual live `boost_amount`
  instead of a hardcoded `0.0`.
- **Constants, honestly labeled:** `MAX_CAR_SPEED`, `MAX_BOOST` (100, a
  full tank), and `BOOST_ACCELERATION` (~991.667 uu/s^2) are commonly-cited
  community numbers (the same body of public research `PhysicsWorld`'s
  gravity constant comes from); `BOOST_CONSUMPTION_RATE` is this project's
  own simplified constant approximating "a full tank lasts roughly 3
  seconds" rather than Rocket League's real drain curve. Reusing
  `MAX_CAR_SPEED` as boost's speed cap too (real Rocket League doesn't
  share one ceiling between throttle and boost) is a documented
  simplification — see the spec's Open questions.
- **Not implemented** (explicitly, not silently dropped): jump, air
  control (pitch/yaw/roll torque while airborne), and handbrake/drift —
  each a distinct real mechanic, tracked as separate follow-up work.
- 6 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet` (81
  total): boost accelerates a car regardless of ground contact, drains the
  tank over time and clamps at zero, has no effect once the tank is empty,
  and still drains the tank even once the car is at `MAX_CAR_SPEED` and the
  forward force stops applying, plus — the real end-to-end proof — a car
  given full boost input with gravity zeroed in a live `PhysicsWorld::step`
  loop actually drives forward while airborne, and a regression test
  confirming a new car starts with a full boost tank.

---

## Driven car input (ground throttle and steering)
**2026-08-29** · [#23](https://github.com/baileyrd/rusty_bullet/pull/23) · `f1a0381`

- **Added:** `rb_physics_bullet::drive`, coupling `rb_domain::ControllerInput`
  into a throttle force (along the car's local forward axis, capped at
  `MAX_CAR_SPEED`) and a steering torque (about the car's local up axis,
  scaled by current speed so a stationary car can't turn in place) —
  `RB-PHYSICS-001-FR-007`. Both are gated on the car actually touching the
  ground; a free-floating car has no wheels to grip, so airborne input
  does nothing yet.
- **Added:** `PhysicsWorld::set_car_input`, setting a car's current
  `ControllerInput`, which persists across steps until changed again
  (matching how a real controller's state holds between frames).
  `PhysicsWorld::step` computes each car's ground-contact state up front
  and applies its driven forces alongside gravity, before integrating
  velocities.
- **Changed:** `frame()` now reports each car's actual driving input
  (`Some(input)`) instead of always `None`.
- **Constants, honestly labeled:** `MAX_CAR_SPEED` (2300 uu/s) is a
  commonly-cited community number (the same body of public research
  `PhysicsWorld`'s gravity constant comes from); `THROTTLE_ACCELERATION`
  is this project's own simplified constant standing in for Rocket
  League's real speed-dependent throttle curve; `STEER_TORQUE` is an
  uncalibrated placeholder with no public reference at all, chosen only to
  produce a visibly responsive turn in tests.
- **Not implemented** (explicitly, not silently dropped): boost, jump, air
  control (pitch/yaw/roll torque while airborne), and handbrake/drift —
  each a distinct real mechanic, tracked as separate follow-up work. A car
  with no input set behaves exactly as a free rigid box always has.
- 10 new unit tests in `rb_physics_bullet` (75 total): a neutral input is
  a no-op, throttle accelerates/caps-at-max-speed/reverses/is
  grounded-only, steering is speed-gated (a parked car can't turn) and
  sign-correct, and — the real end-to-end proof — a car given throttle
  input in a live `PhysicsWorld::step` loop actually drives forward across
  the ground, plus a regression test confirming a car with no input set is
  unaffected.

---

## Multi-car PhysicsWorld support
**2026-08-29** · [#21](https://github.com/baileyrd/rusty_bullet/pull/21) · `28b8d4c`

- **Changed (breaking):** `PhysicsWorld.car: Option<RigidBody>` is
  replaced by `cars: Vec<RigidBody>`. `with_car` now appends, so calling
  it repeatedly builds a scene with any number of cars —
  `PhysicsWorld::new(ball, ground).with_car(a).with_car(b)` is a two-car
  scene. No cap is imposed by this crate (Rocket League's real 8-car limit
  is a gameplay rule, not a physics-core one).
- **Changed:** `PhysicsWorld::step` now resolves every car's ground
  contact, every ball-vs-car pair, and every car-vs-car pair each step —
  `collision::box_vs_box` (added in the previous release but with no live
  caller) now runs for real in a live scene, one pair at a time, not just
  under a unit test. `frame()` assigns each car's `player_id` as its index
  in `cars`.
- **Not implemented** (explicitly, not silently dropped): a combined
  multi-body solve — each pair is still resolved independently, its own
  full solver pass, rather than one simultaneous solve across every
  contact touching in the same step. This is a real approximation once 3+
  bodies are mutually touching at once (e.g. a car pinned between the ball
  and another car); driven car input also remains not implemented.
- 3 new unit tests in `rb_physics_bullet` (65 total): `with_car` called
  twice builds a two-car scene, `frame()` assigns sequential `player_id`s
  across multiple cars, and — the real end-to-end proof — two cars shot
  head-on at each other in a live `PhysicsWorld::step` loop actually
  bounce off each other instead of tunnelling through.

---

## Car-vs-car collision detection
**2026-08-29** · [#19](https://github.com/baileyrd/rusty_bullet/pull/19) · `2eddfe7`

- **Added:** `collision::box_vs_box`, a general separating-axis test
  (SAT) between two oriented boxes (`RB-PHYSICS-001-FR-006`) — 3+3 face
  axes plus 9 edge-pair cross-product axes, the same overall structure as
  `btBoxBoxDetector::dBoxBox`. When every axis shows overlap, the
  minimum-penetration axis becomes the contact normal; a face axis
  produces a clipped face manifold (0-4 points, via a box-specific closed
  form of incident-face-vs-reference-face clipping), an edge axis a
  single edge-edge point (via a standard closest-point-between-segments
  construction).
- **Changed:** `collision::contact_between` is renamed `contacts_between`
  and now returns `Vec<Contact>` uniformly (previously `Option<Contact>`)
  — needed since box-vs-box can return a manifold where sphere-vs-box
  always returned at most one point. `solver::resolve_contact_between` is
  similarly generalized to `resolve_contacts_between`, resolving an entire
  manifold between two dynamic bodies (mirroring `resolve_contacts`'
  existing multi-contact structure for one body vs. a static plane) rather
  than a single contact.
- **Not wired up** (explicitly, not silently dropped): `PhysicsWorld`
  still models exactly one ball and one optional car, so `box_vs_box` has
  no live caller in a real simulated scene — a second car colliding with
  the first never actually happens yet. Wiring it in needs multi-car
  `PhysicsWorld` support, a distinct, larger scope decision tracked as
  separate follow-up work, not this change's scope.
- 4 new unit tests in `rb_physics_bullet` (62 total): no contact for
  far-apart boxes, a 4-point manifold with correct depth/normal for a
  symmetric flat overlap, argument-order antisymmetry (matching the
  sphere-vs-box convention), a partial manifold for a non-flat rotated
  overlap, and (in `solver`) a generalized `resolve_contacts_between`
  settling two colliding boxes' face-to-face manifold without spurious net
  rotation — the same property already verified for the one-body
  ground-manifold case.

---

## Ball-vs-car collision
**2026-08-28** · [#17](https://github.com/baileyrd/rusty_bullet/pull/17) · `2f12c8f`

- **Added:** `rb_physics_bullet` gains analytic sphere-vs-box contact
  generation (`collision::sphere_vs_box`, dispatched via
  `collision::contact_between`) completing `RB-PHYSICS-001-FR-004` — the
  ball and car now actually collide with each other, not just the ground.
  A closed-form closest-point-on-box query handles the ordinary case; a
  second case handles the sphere's center already being inside the box
  (deep penetration), pushing out through whichever face is nearest.
- **Added:** a two-dynamic-body sequential-impulse solver path
  (`solver::resolve_contact_between`), generalizing the existing
  body-vs-static-plane constraint rows to carry both bodies' mass/inertia
  contributions — the generic path Bullet's real solver always runs
  (`resolve_contacts`'s one-body-only shortcut only worked because a
  static plane's side of that math is always zero).
- **Added:** `rb_domain::Quat::conjugate` (`btQuaternion::inverse`),
  needed to transform a world-space point into a rotated box's local
  frame.
- **Changed:** `PhysicsWorld::step` is restructured into Bullet's actual
  staged pipeline — integrate every body's velocity, then resolve every
  contact (ground contacts for each body, then the one ball-vs-car
  contact), then integrate every body's transform — instead of stepping
  each body fully in isolation, so ball-vs-car resolution sees the same
  pre-integration state ground contacts do.
- **Not implemented** (explicitly, not silently dropped): box-vs-box
  collision (two cars against each other) — this scope has exactly one
  car, so it never arises; driven car input remains a free rigid box with
  nothing coupling throttle/steer/boost into it.
- 11 new unit tests in `rb_physics_bullet` (58 total) and 1 in `rb_domain`
  (23 total), including an end-to-end `PhysicsWorld::step` test confirming
  a ball shot at a stationary car actually bounces off it instead of
  tunnelling through, and solver tests confirming the two-body path
  conserves linear momentum and leaves a much heavier body barely moving
  from a much lighter body's impact.

---

## Box-shaped car bodies
**2026-08-28** · [#15](https://github.com/baileyrd/rusty_bullet/pull/15) · `24468cf`

- **Added:** `rb_physics_bullet` gains a unified `RigidBody`/`Shape`
  design (`RB-PHYSICS-001-FR-004`) — one rigid-body type serving both the
  ball (sphere) and a car (box), matching Bullet's own architecture
  (`btRigidBody` plus a polymorphic `btCollisionShape`) rather than a
  separate type per shape. `Sphere` is gone; `RigidBody::sphere(...)` and
  `RigidBody::car_box(half_extents, ...)` are the new constructors.
- **Added:** `Mat3`, a general 3x3 matrix (ported from
  `btMatrix3x3::setRotation`/`scaled`) — needed because a box's inertia
  tensor is anisotropic, unlike a sphere's isotropic (scalar) one.
  `RigidBody` now carries `inv_inertia_local` (diagonal, body frame) and
  recomputes a full `inv_inertia_world` matrix each step
  (`update_inertia_tensor`) from the body's current orientation. A
  sphere's `inv_inertia_world` is mathematically orientation-independent,
  so this is a strict generalization — sphere behavior is unchanged.
- **Added:** analytic box-vs-plane contact generation — tests all 8
  corners against the plane (exact for a box vs. an infinite plane, not
  an approximation), producing 1 to 4 contacts depending on orientation
  (4 resting flat, 2 on an edge, 1 on a corner).
- **Added:** multi-contact manifold resolution — the solver now resolves
  an entire manifold (`resolve_contacts`, 1-4 points) together each
  iteration, sharing one accumulated velocity delta, instead of one
  contact at a time. A box dropped flat settles without spuriously
  tipping onto an edge — verified by a dedicated test.
- **Added:** `PhysicsWorld::with_car`, an optional car body stepped and
  collided against the ground independently from the ball.
- **Not implemented** (explicitly, not silently dropped): box-vs-sphere
  (car-vs-ball) collision — the two bodies never collide with each other
  yet, needing a real convex narrow-phase algorithm (SAT or GJK/EPA);
  driven car input — a car here is a free rigid box, nothing couples
  throttle/steer/boost into it; constant calibration
  (`RB-PHYSICS-001-FR-005`) still needs real `PHASE-0-EXIT` data.
- **Verified:** 21 new unit tests (47 total in `rb_physics_bullet`, 96 in
  the workspace): box inertia formula, orientation-dependent inertia
  (unlike a sphere's), box-vs-plane contact counts for flat/edge/corner/
  embedded cases, a box in free-fall matching the same kinematics as a
  sphere, and — the key multi-contact regression test — a box dropped
  flat settling on the ground without tipping over or accumulating
  spurious spin.
- 21 new unit tests; `cargo fmt --check`, `clippy -D warnings`, and
  `cargo test --workspace` all pass.

## Timestamp-tolerant alignment
**2026-08-28** · [#13](https://github.com/baileyrd/rusty_bullet/pull/13) (merge commit `59266ea`)

- **Added:** `rb_domain::divergence::score` now aligns frames by nearest
  `timestamp_secs` instead of list index (`RB-VERIFY-003-FR-003`) — an
  `O(recorded.len() + candidate.len())` merge over both sequences'
  existing chronological order, not a binary search per frame. A match
  only counts if the two frames' timestamps are within a new required
  `max_timestamp_delta_secs` parameter; a recorded frame with nothing
  that close on the candidate side is skipped, not force-matched to the
  nearest-but-still-distant option. `DivergenceScore.frames_compared`'s
  meaning changes accordingly: it's no longer capped at
  `min(recorded.len(), candidate.len())` — a much shorter candidate
  sequence can now be matched against every recorded frame within
  tolerance of it.
- **Added:** `rb_verify_cli::DEFAULT_MAX_TIMESTAMP_DELTA_SECS` (0.02s,
  reasoned from the vendored replay fixture's own ~0.036s average
  sampling interval, not yet empirically tuned) and an optional third
  `rb-verify` CLI argument to override it.
- **Fixed:** implementing real timestamp alignment surfaced an actual bug
  in `rb_capture_ingest`'s synthetic fixture — its timestamps started at
  `0.0`, but the vendored replay fixture's ball doesn't produce a frame
  until roughly **11.78 seconds** in (kickoff countdown; frames before the
  ball spawns are omitted by design). The old index-pairwise comparison
  silently compared these temporally unrelated frames anyway, since it
  only ever looked at list position — exactly the failure mode FR-003
  exists to catch. Corrected the fixture's timestamps to actually overlap
  the replay's real timeline.
- **Verified:** 2 new unit tests in `rb_domain::divergence` (different
  tick rates aligning correctly with hand-computed expected matches; a
  shorter candidate sequence still matching every in-tolerance recorded
  frame). One existing test was replaced since its premise — sequence
  length alone caps how many frames compare — no longer holds. Manually
  re-run end-to-end against the corrected fixtures (default 0.02s
  tolerance): `frames compared: 6, mean ball distance: 0.25 uu, max ball
  distance: 0.25 uu, car pairs compared: 6, mean car
  position/rotation/velocity distance: 2816.42 uu / 2.36 rad / 1307.87
  uu/s`. `RB-VERIFY-003` now has all three functional requirements
  implemented.
- 2 new unit tests (75 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## Car-state divergence scoring
**2026-08-28** · [#11](https://github.com/baileyrd/rusty_bullet/pull/11) (merge commit `a1b8a47`)

- **Added:** `rb_domain::divergence::DivergenceScore` gains a `cars:
  CarDivergence` field — mean/max car position distance, rotation distance
  (radians), and velocity distance, plus the number of car pairs compared
  (`RB-VERIFY-003-FR-002`). Cars are matched between the recorded and
  candidate sequences by `player_id` within each frame pair; a car present
  on only one side of a pair is skipped for that frame, not an error.
- **Added:** `Quat::angle_to` (`rb_domain::state`) — the angle between two
  rotations, in radians. Uses an `atan2`-based half-angle formula rather
  than the more obvious `2.0 * dot.acos()`: `acos` is numerically unstable
  exactly where this metric cares most (near-identical rotations, where a
  tiny `f32` rounding difference would otherwise produce a spuriously
  large angle). Handles the quaternion double-cover (`q` and `-q` are the
  same rotation) via the dot product's absolute value.
- **Changed:** `rb-verify`'s output now prints car-pair count and
  position/rotation/velocity stats alongside the existing ball stats.
- **Verified:** 8 new unit tests in `rb_domain` (4 car-scoring cases: 
  identical states, known position/velocity offsets, a known rotation
  offset, a car unmatched on one side; 3 for `angle_to`). Manually re-run
  end-to-end against the same real replay fixture + synthetic capture
  fixture: `car pairs compared: 5, mean car position/rotation/velocity
  distance: 2823.85 uu / 2.36 rad / 1369.44 uu/s`. As before, these
  numbers are not a fidelity signal — the two fixtures are unrelated
  matches — they only confirm car scoring runs correctly end-to-end.
- 8 new unit tests (73 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## Divergence scoring CLI wiring
**2026-08-28** · [#9](https://github.com/baileyrd/rusty_bullet/pull/9) (merge commit `f10d017`)

- **Added:** `rb_verify_cli::score_replay_against_capture` (new `lib.rs`)
  — the actual composition-root wiring, ingesting a replay via
  `rb_replay_ingest` and a capture via `rb_capture_ingest` and running
  `rb_domain::divergence::score` on the results. `main.rs` is now a thin
  argument-parsing/output wrapper over it, kept separate so the wiring
  itself is unit-testable without spawning a process.
- **Changed:** `rb-verify`'s output is now a small human-readable summary
  (frames compared, mean/max ball distance) instead of a raw `Debug` dump.
- **Verified:** 3 new unit tests against `rb_replay_ingest`'s vendored
  replay fixture and `rb_capture_ingest`'s synthetic capture fixture
  (happy path, missing-replay, missing-capture). Manually run end-to-end:
  `frames compared: 5, mean ball distance: 0.25 uu, max ball distance:
  0.25 uu`. This proves the ingest → score pipeline runs without erroring
  across both real adapters — explicitly **not** a fidelity measurement,
  since the replay and capture are unrelated matches and
  `RB-VERIFY-003-FR-002`/`FR-003` (car-state scoring, timestamp-tolerant
  alignment) are still open.
- 3 new unit tests (66 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## BakkesMod capture ingestion — JSON-Lines parser + shared input schema
**2026-08-28** · [#7](https://github.com/baileyrd/rusty_bullet/pull/7) (merge commit `dc7e82f`)

- **Added:** `rb_domain::ControllerInput` and `CarState.input:
  Option<ControllerInput>` (ADR-0005) — a shared controller-input schema
  for both ingestion adapters. `throttle`/`steer` are always a number;
  `pitch`/`yaw`/`roll` are `Option<f32>` since only BakkesMod captures can
  ever populate them (a replay's dodge impulse/torque vectors are a
  different kind of quantity, not an analog stick angle). Resolves
  `RB-VERIFY-001-FR-004`, deferred since replay ingestion landed.
- **Changed:** `rb_replay_ingest::convert` now attaches recovered input
  (throttle/steer normalized from replicated bytes, jump/boost/handbrake
  from `subtr_actor`'s boolean flags) to every car it converts. 4 new unit
  tests (14 total in the crate).
- **Added:** `rb_capture_ingest` now really parses capture files
  (`RB-VERIFY-002-FR-002`/`NFR-001`): the capture format is JSON Lines, one
  `{"timestamp_secs", "ball", "cars"}` object per tick (ADR-0005), decoded
  via a new `wire` module (`serde`/`serde_json`, justified in
  `Cargo.toml`) into `rb_domain::PhysicsFrame`s with every car's `input`
  populated. 10 new unit tests, run against a synthetic, hand-authored
  fixture — see `crates/rb_capture_ingest/fixtures/README.md`.
- **Resolved:** `RB-RESEARCH-O003` (BakkesMod tooling scope) — a one-off
  script writing an unversioned format, not a reusable harness, per
  ADR-0005.
- Known limitation stated plainly, mirroring `RB-RESEARCH-O002`'s own
  practical blocker: the BakkesMod-side plugin that would actually write a
  capture file (`RB-VERIFY-002-FR-001`) has not been built — this
  sandboxed environment has no Rocket League, BakkesMod, or Windows
  environment to build or run it in. `PHASE-0-CAPTURE-INGEST`'s exit gate
  (a real capture, cross-checked against BakkesMod's own overlay) stays
  open until the owner builds and runs that plugin on their own machine.
- 14 new unit tests (63 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## Replay ingestion — local real-corpus validation gate
**2026-08-28** · [#5](https://github.com/baileyrd/rusty_bullet/pull/5) (merge commit `0b2253d`)

- **Added:** `corpus_check`, a local/gitignored-corpus health-check binary
  (`cargo run -p rb_replay_ingest --bin corpus_check [dir]`,
  `RB-VERIFY-001-NFR-003`) — runs the real `boxcars` + `subtr-actor` +
  `convert` pipeline against every `.replay` file in a directory (default
  `replays/` at the workspace root, already `.gitignore`d) and exits
  non-zero on any parse failure. A checkout with no corpus present is a
  deliberate no-op, matching `RLEvalSystem`'s own gitignored-corpus
  convention.
- **Verified:** run once against 40 of the owner's own real match replays
  (`baileyrd/replays`) — 40/40 parsed cleanly, durations 19s-717s, 2-11
  players per match, ball Z consistently within plausible soccar bounds.
  Closes the "runs correctly on real owner data at scale" half of
  `RB-VERIFY-001`'s owner-data acceptance criterion; the stricter manual
  single-timestamp cross-check remains open. Marks `PHASE-0-REPLAY-INGEST`
  Done.
- No new dependencies; no `rb_domain`/`rb_replay_ingest` library code
  changed. The owner's real replay files are never committed — only
  aggregate results (counts, ranges) appear in this repo's docs.

## Replay ingestion — boxcars + subtr-actor
**2026-08-28** · [#3](https://github.com/baileyrd/rusty_bullet/pull/3) (merge commit `93ad0e9`)

- **Added:** `rb_replay_ingest` now really parses `.replay` files
  (`RB-VERIFY-001-FR-001/002/003`): `boxcars` parses the raw replay/network
  stream, `subtr-actor` resolves it into frame-indexed ball/car
  `RigidBody` state, and a new `convert.rs` maps that into
  `rb_domain::PhysicsFrame`. Verified end-to-end against a real vendored
  replay fixture (12,029 frames, ~428s match).
- **Added:** `subtr-actor` as a dependency, justified in
  `Cargo.toml` — avoids hand-rolling `boxcars`' actor-graph resolution
  (net-cache/property-id resolution, quantized rotation decoding), a
  substantial and error-prone parsing layer with an existing,
  permissively-licensed, purpose-built solution.
- **Changed:** `RB-RESEARCH-S004`'s "replay input is lossy/inferred at
  best" finding is revised — `subtr-actor` actually recovers raw
  throttle/steer bytes and boost/jump/dodge/powerslide booleans directly
  from the replay's replicated input actor. Still not wired into
  `rb_domain`'s types (`RB-VERIFY-001-FR-004` stays open pending a schema
  decision made jointly with `RB-VERIFY-002`).
- Known limitation stated plainly: the vendored fixture is a third
  party's replay, used only to prove the pipeline runs correctly on real
  bytes — it does not satisfy `RB-VERIFY-001`'s acceptance criterion of a
  manually-verified position check against the owner's own match, since
  this environment has no access to the owner's replay files.
- 10 new unit tests (51 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## Physics core v0 — Bullet3 port (sphere vs. ground)
**2026-08-28** · [#1](https://github.com/baileyrd/rusty_bullet/pull/1) (merge commit `7bdc3fc`)

- **Added:** `rb_physics_bullet`, a from-scratch Rust port of specific
  Bullet3 (zlib-licensed) algorithms — rigid-body integration
  (`btRigidBody`) and the sequential-impulse contact solver
  (`btSequentialImpulseConstraintSolver`) — scoped to a dynamic sphere (the
  ball) against a static plane (the ground). Resolves the build-vs-integrate
  physics question via ADR-0004, ahead of `PHASE-0-EXIT` divergence data
  existing, on the strength of Bullet3's direct relevance and permissive
  license.
- **Added:** vector/quaternion algebra (dot, cross, normalize, quaternion
  product/rotation) on `rb_domain`'s `Vec3`/`Quat`, justified by the
  physics crate as a second real consumer.
- Known, deliberate scope cuts stated plainly: no car (box) rigid bodies or
  general 3x3 inertia tensor yet, no split impulse, no warm-starting or
  sleeping — a bouncy (restitution > 0) resting contact does not settle
  under this solver, by design of what v0 covers, not by accident. See
  `RB-PHYSICS-001` and `rb_physics_bullet::solver`'s module doc.
- Also completed the legal/practical review `RB-RESEARCH-O002` (binary
  reverse engineering of the shipped client) needed: Epic/Psyonix's EULA
  and Rocket League's Code of Conduct both contractually prohibit reverse
  engineering, and this sandbox has no access to the game binary regardless
  — still open pending the owner's own legal counsel and sign-off.
- 26 new unit tests (41 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## Repo bootstrap — full lifecycle baseline
**2026-08-28** · landed directly on `main` at commit `5be2078` (predates this repo's "always PR" convention; no PR exists for it)

- **Added:** Full `rust-repo-lifecycle` + `repo-config` bootstrap: charter,
  system architecture, a 6-spec tree (`RB-VERIFY-001/002/003` fully
  specified for Phase 0; `RB-PHYSICS-001`/`RB-SIM-001`/`RB-NET-001` as
  forward-looking placeholders), 3 ADRs (server-authoritative netcode,
  verification-first ordering, Bullet-fidelity target), a research backlog
  (6 settled findings + 3 tracked open questions), a Phase 0-4 roadmap with
  exit criteria tied to the divergence metric, requirement-level
  traceability, AGENTS.md/WORKFLOW.md, and the standard governance file set
  (README, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, PR/issue
  templates).
- **Added:** Minimal buildable Cargo workspace — `rb_domain` (physics frame
  types, `PhysicsStateSource` port, divergence-scoring algorithm),
  `rb_replay_ingest`/`rb_capture_ingest` (adapter stubs implementing the
  port), `rb_verify_cli` (composition-root binary). Divergence scoring
  (`RB-VERIFY-003-FR-001`) is real and unit-tested; both ingestion adapters
  are intentionally stubbed (`IngestError::NotImplemented`) — `boxcars`
  parsing and the BakkesMod capture format are Phase 0 delivery work, not
  bootstrap scaffolding.
- Known scope cut, stated plainly: no physics/simulation/netcode code
  exists yet — this PR is the governed baseline the rest of the project
  builds against, per ADR-0002's verification-first ordering.
- 11 unit tests added (6 in `rb_domain`, 1 each in the two adapter stubs,
  plus workspace doc-tests); `cargo fmt --check`, `clippy -D warnings`, and
  `cargo test --workspace` all pass.
