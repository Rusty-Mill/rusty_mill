# Release Notes

Tracks notable changes to this repo, one entry per merged change against
`main`, reverse chronological. Pre-1.0, no version tags yet — entries are
keyed by the commit/PR that shipped them. Older entries are trimmed to
their one-line summary; the full text of every entry up to `72494f4` is
in git history, and each requirement's spec section holds its detail.

---

## Spin in the car's own axes
**2026-10-01** · `RB-VERIFY-003` 0.14.0

- `--self-trace` now prints each car's spin as roll, pitch and yaw rates.
  The capture's mid-flip response to a yaw input at 4.867 s could not be
  read in world axes; keeping yaw air control on mid-flip (tried, reverted)
  moved the simulated spin the wrong way. 436 tests.

## Wheels let go when the real ones do
**2026-10-01** · `RB-PHYSICS-001-FR-092` · ADR-0019 amendment

- Wheel rays now reach the fully extended wheel (RocketSim cut 2.5 uu off),
  and the sticky force acts one step after the wheels touch. A jump from
  rest now matches every tick of the owner's capture: 295.7 uu/s, six
  ticks of +4.0, then +6.7. 435 tests.

## Jumps rise like the real ones
**2026-10-01** · `RB-PHYSICS-001-FR-091` · ADR-0019

- The ground jump now pushes at full strength from the press tick, without
  RocketSim's 0.62 early scale-down (a TODO in its source), and the
  suspension stops pushing once the jump lifts the car. A jump from rest
  gives 295.7 uu/s on the press tick (recorded 295.9), then +4.0 per tick
  while the wheels touch and +6.7 after, as in the owner's capture. 433
  tests.

## Cars ride on suspension
**2026-10-01** · `RB-PHYSICS-001-FR-090` · ADR-0018

- A standard car now rides on RocketSim's four suspended wheel rays, with
  its hitbox offset 13.9 uu forward and 20.8 uu up as in the game. It
  settles at 17.0 uu, the recorded rest height (it rested at 19.3 on its
  box), and lands on springs instead of bouncing. 432 tests.

## Throttle turns the car through its front wheels
**2026-10-01** · `RB-PHYSICS-001-FR-089`

- The engine now pushes at each wheel along that wheel's heading, as in
  RocketSim, instead of at the car's centre. Steered front wheels add a
  turning force under throttle: at 950 uu/s with full steer the steady
  yaw rate rises from 2.33 to 2.45 rad/s (the real capture: 2.40). This
  was the earliest real-capture error, 47 uu/s by 4.05 s. 427 tests.

## Grounded on wheels, not the box
**2026-10-01** · `RB-PHYSICS-001-FR-088` · ADR-0017

- A car now counts as on the ground when three of its four wheel rays reach
  the floor, as in RocketSim. Before, it took a box corner touching. At
  5.758 s in the real capture, the candidate's box was bouncing 3 uu off
  the floor, so a jump press fired a side dodge and velocity error jumped
  from 430 to 1,248 uu/s. 424 tests.

## Flips turn as fast as the real ones
**2026-10-01** · `RB-PHYSICS-001-FR-087` · `RB-VERIFY-003` 0.13.0

- `--self-trace` now shows the spin each car's orientation change implies
  (`q-rate`). The recorded flip turns at ~7.6 rad/s while reporting the
  5.5 rad/s cap: the game clamps spin after moving the car. The port
  clamped before, so its flips lagged ~2 rad/s, the orientation error that
  made boost push the wrong way at 4.94 s. Now clamped after, as in
  RocketSim. 419 tests.

## Steering through the wheels
**2026-10-01** · `RB-PHYSICS-001-FR-086` · ADR-0016

- Each wheel now grips sideways at its own contact point, as in RocketSim,
  and the steered front wheels turn the car. The turn rate builds up over
  ~0.25 s instead of snapping to its target, matching the recorded ramp,
  and a released turn dies away. This targets the heading error that
  tilted the 4.317 s flip. 417 tests.

## Spin in the trace; flips start on the press tick
**2026-10-01** · `RB-PHYSICS-001-FR-085` · `RB-VERIFY-003` 0.12.0

- `rb-verify --self-trace` prints recorded and simulated angular velocity.
  It showed the real game's flip spinning on the press tick (one tick
  earlier than the port), now fixed, and traced the flip's remaining
  orientation error to ground steering snapping the yaw rate instead of
  ramping it. 418 tests.

## Air control is Rocket League's
**2026-10-01** · `RB-PHYSICS-001-FR-084` · ADR-0015

- Pitch, yaw and roll in the air now use RocketSim's torque (130, 95, 400)
  and damping (30, 20, 50) with its torque scale, so a car's spin also
  decays when the stick is released. Throttle gives the small air push
  RocketSim has. The port's placeholder air torque and its invented
  airborne auto-upright are gone. 418 tests.

## Flips spin, stall and cancel like Rocket League's
**2026-10-01** · `RB-PHYSICS-001-FR-083` · ADR-0014

- A dodge's flip is now RocketSim's: a torque over 0.65 s (starting the
  tick after the press), air control off meanwhile, pitch locked to
  0.95 s, the fall damped from 0.15 s (the recorded vz stall at
  about -15 uu/s), and cancelled by holding pitch against the flip. The
  old instant spin and the jump-press-again cancel are gone. 421 tests.

## Dodges go the way the stick points
**2026-10-01** · `RB-PHYSICS-001-FR-082` · ADR-0013

- The real capture's forward-left dodge at 4.317 s went backward at
  ~2,175 uu/s in the candidate. Pitch and roll were read with the opposite
  signs from Rocket League's (RocketSim's `-controls.pitch`, -right and
  -forward air axes). They now match, and the dodge impulse is RocketSim's
  500 uu/s (was a 1400 placeholder): its formula predicts ~628 uu/s for
  that dodge, 621 was recorded. The push is now horizontal even when the
  car is tilted. Position error at 5.0 s fell from 1,590 to 79 uu on the
  first re-trace. 3 new tests (422 total).

## Tire grip: cars roll, grip sideways, and drift on the handbrake
**2026-10-01** · `RB-PHYSICS-001-FR-081` · ADR-0012

- A car's box no longer slides on the floor. Its contact there is
  frictionless, and RocketSim's tire model grips instead: sideways by the
  lateral friction curve, coast-braking at 525 uu/s², braking at
  3500 uu/s² against opposing throttle. The handbrake ramps in and cuts
  sideways grip to a tenth while keeping most forward grip. Targets the
  real capture's velocity-direction lag and ~10% speed gap after FR-080:
  velocity error at 4.0 s is now 9.9 uu/s. A released turn now stops
  instead of spinning on. 11 net new tests (419 total).

## Steering from Rocket League's real steer-angle curve
**2026-10-01** · `RB-PHYSICS-001-FR-080` · ADR-0011

- Steering now sets the car's yaw rate from RocketSim's
  `STEER_ANGLE_FROM_SPEED_CURVE` (powerslide curve on handbrake) through
  the bicycle model over the 85 uu Octane wheelbase, replacing the
  placeholder `STEER_TORQUE` whose shape FR-065 found was backwards. Turns
  tightest from a stop, gently at speed, mirrored in reverse. 6 new tests
  (408 total).

## Fixed the driving-car hop (restitution threshold units)
**2026-10-01** · `RB-PHYSICS-001-FR-079`

- The first real `--self-trace` showed a driving car grounded only one
  tick in three, so throttle under-applied and a jump fired as a side
  dodge. Cause: Bullet's 0.2 m/s restitution threshold copied as 0.2 uu/s;
  now 10 uu/s (RocketSim's `BT_TO_UU = 50`). New regression test: before,
  airborne 24 of 36 ticks; after, grounded every tick. 402 tests pass.

## Per-frame candidate trace (`rb-verify --self-trace`)
**2026-10-01** · `RB-VERIFY-003-FR-005`

- New `rb-verify --self-trace <capture> <from-secs> <to-secs>` prints,
  for every car at every frame in the window, the recorded input and the
  recorded vs. simulated position, velocity and orientation error, on the
  same time axis `--self-growth` uses. Built because the first real
  `--self-growth` run (`test2.jsonl`) showed an abrupt car-only
  derailment at ~3 s. On the synthetic fixture it already shows a ground
  jump firing as a dodge; see `docs/PROJECT-STATUS.md` Next for the
  hitbox-offset hypothesis. 4 new tests (401 total).

## Slimmed `RELEASE_NOTES.md` to summaries
**2026-10-01** · docs only

- This file went from 222 KB to 44 KB. All 95 entries keep their heading
  and their date/PR/commit line. The 92 entries over ~900 characters are
  trimmed to their first bullet, which in every entry is the summary, cut
  at a sentence end near 350 characters. Full text of every entry up to
  `72494f4` is in git history; requirement detail lives in each spec
  section.

---

## Slimmed `TRACEABILITY.md` to pointers
**2026-10-01** · docs only

- `docs/traceability/TRACEABILITY.md` went from 239 KB to 46 KB. All 91
  requirement rows and all 7 columns stay; only cells over ~220 characters
  changed (76 rows):
  - Decision/interface: the interfaces the row names that also appear in
    its Implementation cell, then other qualified (`module::name`)
    interfaces (up to 4), plus any ADRs.
  - Verification: the first clause.
  - State: the status plus its first qualifier.
  - PR/release: commit lists over 100 characters point to this file.
- The full narrative for each requirement already lives in its spec
  section. The previous long-form matrix is in git history at `8160c97`.
- Stale `crates/rb_physics_bullet/src/drive.rs` paths now read
  `src/drive/` (split in #164).

---

## ADRs for Phase 1 physics modeling choices
**2026-09-30** · docs only

- Five ADRs recorded retroactively. Each cites the FRs and PRs where the
  choice was made, the alternatives the spec actually documents, and when
  to revisit: - ADR-0006: arena from analytic primitives, not the dumped
  triangle mesh. - ADR-0007: car vs. curved geometry by 8-corner testing.

---

## Trimmed `PROJECT-STATUS.md`; fixed broken doc tables
**2026-09-30** · docs only

- `docs/PROJECT-STATUS.md` went from 2,046 lines / 162 KB to 137 lines / 7
  KB. The 89-entry "Completed" log (duplicated by this file, the
  changelog, and the roadmap) is replaced by a phase table and the last
  five merges; the full log stays in git history at `a245d35`.

---

## Grouped per-car drive state into `DriveState`
**2026-09-30** · refactor, no behavior change

- New `rb_physics_bullet::drive::DriveState` holds the six per-car values
  `apply_driven_forces` carries across steps; `DriveState::new(base_friction)`
  gives `with_car`'s old defaults. Each field's rules moved from the
  function's doc comment onto the field itself.
- `apply_driven_forces(car, input, on_ground, wall_normal, &mut state, dt)`
  replaces the 11-parameter form, and `PhysicsWorld` keeps a
  `Vec<DriveState>` instead of six parallel `Vec`s. Both production
  `too_many_arguments` suppressions in the drive path are gone.
- 2 new tests (`DriveState::new` defaults; one step updates every field it
  owns). The existing test helper chain packs/unpacks at its innermost
  call, so all 395 existing tests pass unmodified (397 total).

---

## Split `drive.rs` by mechanic; fixed stale docs
**2026-09-30** · refactor, no behavior change

- `crates/rb_physics_bullet/src/drive.rs` (4,242 lines, ~2,800 of them
  tests) is now `drive/{mod,ground,air,jump,boost,tests}.rs`. Each
  mechanic's constants and logic live together; `apply_driven_forces`
  keeps its signature and delegates to one function per mechanic.
- The ground dodge and wall-jump dodge were two near-identical ~30-line
  blocks; both now call a single `jump::apply_dodge` (float operation
  order preserved, so results are bit-identical).
- `AGENTS.md` crate map gained the missing `rb_physics_bullet` entry;
  `docs/PROJECT-STATUS.md` header now points at `978aa8f`.
- All 395 tests pass unmodified; fmt/clippy/rustdoc clean.

---

## Implemented the divergence-growth diagnostic
**2026-09-04** · `RB-VERIFY-003-FR-004`

- Built out the diagnostic scoped in the previous entry: a new
  `rb_domain::divergence::score_windows(recorded, candidate,
  max_timestamp_delta_secs, window_secs) -> Vec<(f32, DivergenceScore)>`
  partitions the same nearest-timestamp-matched frame pairs the existing
  whole-run `score` uses into consecutive `window_secs`-wide time buckets
  and scores each …

---

## Scoped a divergence-growth diagnostic
**2026-09-04** · `RB-VERIFY-003-FR-004`

- The whole-run fidelity number from the previous entry (mean car position
  distance `4508.71` uu) can't tell us *why* the candidate engine diverged
  that much — a single mean/max pair over an entire ~23-second run
  collapses "many small modeling errors compounding" and "one early
  mechanic mismatch derailing everything after it" into the same number.

---

## Ran the candidate engine against a real capture — this project's first genuine fidelity number
**2026-09-04** · `RB-PHYSICS-001-FR-077`

- The owner ran `cargo run -p rb_verify_cli -- --self test2.jsonl` on
  their own machine against the real BakkesMod capture from
  `RB-VERIFY-002-FR-001` (2,818 frames), producing this project's first
  genuine fidelity number — a candidate trajectory actually simulated from
  the capture's own recorded input, scored against that same capture's own
  …

---

## Calibrated the crate's own tests to the real car hitbox (FR-078)
**2026-09-03** · `crates/rb_physics_bullet`

- **`RB-PHYSICS-001-FR-078` implemented, verified.** Every existing
  `car_box`-style test helper across `rb_physics_bullet`
  (`body.rs`/`collision.rs`/`drive.rs`/`net.rs`/`solver.rs`/`world.rs`)
  that models a real car was switched from the old placeholder
  half-extents (`Vec3::new(60.0, 30.0, 18.0)`) to the confirmed real
  `body::CAR_HALF_EXTENTS` …

---

## Wired the candidate engine into rb_verify_cli (FR-077)
**2026-09-03** · `crates/rb_verify_cli`

- **`RB-PHYSICS-001-FR-077` implemented; real-capture run pending.**
  `rb_verify_cli` gains `score_capture_against_candidate`: seeds a
  `PhysicsWorld` from a capture's own first grounded, neutral frame (a new
  `is_grounded_and_neutral` heuristic — proxying for `FR-076`'s unset
  hidden jump/double-jump/dodge state actually being accurate there), …

---

## Implemented the candidate-engine plumbing scoped for FR-005 (FR-076)
**2026-09-02** · `crates/rb_physics_bullet`

- **`RB-PHYSICS-001-FR-076` implemented.** `rb_physics_bullet` can now
  seed a `PhysicsWorld` from a recorded `PhysicsFrame`
  (`PhysicsWorld::from_frame`) and simulate it forward using a recorded
  per-tick controller-input sequence (`world::simulate_recorded`) — the
  two pieces `FR-005`'s real-data constant calibration needs before it can
  produce any …

---

## Scoped the Phase 1 candidate engine FR-005 needs
**2026-09-02** · `docs/specifications/physics/RB-PHYSICS-001-physics-core-port.md`

- **Design only — no code.** With `PHASE-0-EXIT` closed,
  `RB-PHYSICS-001-FR-005` ("calibrate constants against real recorded
  ground truth") is unblocked but has no way to actually run yet: nothing
  feeds a capture's recorded controller input into `rb_physics_bullet` to
  produce a candidate trajectory to score.

---

## Ran the verification pipeline end-to-end on real data for the first time, closing all of Phase 0
**2026-09-02** · `crates/rb_verify_cli`

- **Fed the new real BakkesMod capture into `rb_verify_cli`**: `cargo run
  -p rb_verify_cli --
  crates/rb_replay_ingest/fixtures/subtr-actor-sample.replay <real
  capture>` …

---

## Built, loaded, and fixed the BakkesMod capture plugin against a real game
**2026-09-02** · `bakkesmod-plugin/rusty_bullet_capture/`

- **Closed the one step that couldn't happen in a sandbox**:
  `RB-VERIFY-002-FR-001`'s BakkesMod capture plugin had only ever been
  source-written and grounded against a real SDK clone, never actually
  compiled or run — this required the owner's own Windows/BakkesMod/Rocket
  League environment.

---

## Confirmed the dodge deadzone already matches real Rocket League exactly
**2026-09-01** · [#157](https://github.com/baileyrd/rusty_bullet/pull/157) · `2f5a3eb`

- **This spec's own Open Questions had claimed `DODGE_DEADZONE` "still has
  no public reference at all... so it may be off by a large factor,"** and
  `FR-074`'s own Non-goals (mirroring `FR-073`'s identical earlier claim)
  separately framed RocketSim's all-or-nothing dodge-cancellation check as
  "a real but separate architectural difference" from this …

---

## Near-axis-aligned dodges now snap to a pure single axis, matching real Rocket League
**2026-09-01** · [#155](https://github.com/baileyrd/rusty_bullet/pull/155) · `00039fc`

- **`FR-073`'s own Non-goals had flagged RocketSim's post-normalization
  small-component zeroing as "a separate, independent simplification"** —
  a mis-scoping this change corrects: it isn't a separate mechanism at
  all, but a further pure post-processing step on the exact normalized
  `(pitch, roll)` pair `drive::normalize_dodge_direction` already …

---

## Yaw input now contributes to a dodge's direction, matching real Rocket League
**2026-09-01** · [#153](https://github.com/baileyrd/rusty_bullet/pull/153) · `99a498a`

- **This port's dodge/wall-jump-dodge direction read `pitch`/`roll` stick
  input only, never `yaw`** — `RB-PHYSICS-001-FR-059`'s own Non-goals (and
  `FR-072`'s own doc comment) had already found and flagged this gap: real
  Rocket League's own `dodgeDir` combines `yaw + roll` for its horizontal
  component, so a yaw-only stick nudge (no roll held) should …

---

## Diagonal dodges are no longer faster than axis-aligned ones
**2026-09-01** · [#151](https://github.com/baileyrd/rusty_bullet/pull/151) · `8f3fcd2`

- **This port summed each dodge axis' own full-strength `(pitch, roll)`
  contribution independently** — `RB-PHYSICS-001-FR-059`'s own Non-goals
  had already found and flagged this exact gap: a diagonal dodge (both
  axes held) came out `sqrt(2)`-ish times faster than an axis-aligned one,
  "a separate, independent behavioral question this requirement …

---

## Real air-control damping mechanism (audit finding)
**2026-09-01** · [#149](https://github.com/baileyrd/rusty_bullet/pull/149) · `b4aa727`

- **`RB-PHYSICS-001-FR-068`'s own Non-goals had already found RocketSim's
  `CAR_AIR_CONTROL_DAMPING = Vec(30, 20, 50)` exists** but left it as "a
  separate, independent addition left for a future requirement" without
  examining the mechanism behind it.

---

## Real flip-cancel is continuous, pitch-stick-driven, and pitch-axis-only (audit finding)
**2026-09-01** · [#147](https://github.com/baileyrd/rusty_bullet/pull/147) · `be2b755`

- **This port's flip-cancel (`RB-PHYSICS-001-FR-016`) triggers on a fresh
  jump press and zeros the car's angular velocity outright** — its own doc
  comment claimed this matched real Rocket League, but that claim was
  never checked against real source.

---

## Real dodge spin is a continuous per-axis torque over a fixed window, not an instantaneous kick (audit finding)
**2026-09-01** · [#145](https://github.com/baileyrd/rusty_bullet/pull/145) · `46053ce`

- **`drive::DODGE_ANGULAR_SPEED` (`5.5` rad/s) applies a flat
  angular-velocity kick at flip start** — `RB-PHYSICS-001-FR-031`'s
  original audit had already found real reference constants
  (`FLIP_TORQUE_X=260`, `FLIP_TORQUE_Y=224`, `0.65`s) but not the
  mechanism behind them.

---

## Real per-axis air-control torque ratio (pitch/yaw/roll)
**2026-09-01** · [#143](https://github.com/baileyrd/rusty_bullet/pull/143) · `77b047d`

- **All three axes shared one flat `AIR_CONTROL_TORQUE` magnitude** —
  `RB-PHYSICS-001-FR-031`'s original audit had already found real
  air-control torque coefficients exist but didn't adopt them, since
  they're absolute torques calibrated against real Rocket League's own
  specific car mass/inertia tensor — the same "false precision" reasoning
  that kept …

---

## Real Rocket League has no distinct wall-jump mechanic or constant at all (audit finding)
**2026-09-01** · [#141](https://github.com/baileyrd/rusty_bullet/pull/141) · `98f587a`

- **`drive::WALL_JUMP_HORIZONTAL_SPEED` had no public reference at all** —
  this port pushes a wall-jumping car outward along the touched wall's
  normal by this fixed speed, on top of the same vertical `JUMP_SPEED`
  every other jump variant uses.

---

## Real handbrake friction reduction is anisotropic, not a single uniform multiplier (audit finding)
**2026-09-01** · [#139](https://github.com/baileyrd/rusty_bullet/pull/139) · `45b107f`

- **`drive::HANDBRAKE_FRICTION_MULTIPLIER` had no public reference at
  all** — this port multiplies the car's own single isotropic
  `RigidBody.friction` by this factor while `handbrake` is held and
  grounded.

---

## Real steering is a wheeled-vehicle raycast model, not a torque (audit finding)
**2026-09-01** · [#137](https://github.com/baileyrd/rusty_bullet/pull/137) · `8a967c1`

- **`drive::STEER_TORQUE` had no public reference at all** — this port
  applies a direct yaw torque about the car's local up axis, scaled up
  with speed via `speed_factor`.

---

## Real mandatory minimum-hold window for a ground jump's variable-height acceleration
**2026-09-01** · [#135](https://github.com/baileyrd/rusty_bullet/pull/135) · `e201222`

- **`drive::JUMP_HOLD_MAX_DURATION`'s own doc comment had named this exact
  gap** since `RB-PHYSICS-001-FR-031`'s original audit: real Rocket League
  scales its jump-hold acceleration down during a `JUMP_MIN_TIME` (0.025s)
  mandatory window rather than applying it flat from the first held step —
  "that two-phase ramp isn't modeled here."

---

## Real Rocket League uses per-contact-pair-type restitution/friction (audit finding)
**2026-09-01** · [#133](https://github.com/baileyrd/rusty_bullet/pull/133) · `0483b46`

- **`RB-PHYSICS-001-FR-043` had left open** which formula matches real
  Rocket League for `solver::combine_restitution`/`combine_friction` (this
  port's own average, kept over Bullet's real unclamped-product default).

---

## Real ball material properties via a new `RigidBody::ball` constructor
**2026-09-01** · [#131](https://github.com/baileyrd/rusty_bullet/pull/131) · `a1a0812`

- **`RB-PHYSICS-001-FR-061`'s own Non-goals had deferred adopting
  `BALL_DRAG`** for lack of a dedicated ball-construction API — `sphere`
  gives every caller an identical generic `restitution = 0.5`/`friction =
  0.5`/`linear_damping = 0.0` placeholder, with no way to say "this one is
  a real ball."

---

## Hard caps on ball linear/angular speed
**2026-09-01** · [#129](https://github.com/baileyrd/rusty_bullet/pull/129) · `b5eefa6`

- **The ball had no linear or angular speed cap of any kind** — unlike the
  car, which has had a hard angular-speed ceiling since
  `RB-PHYSICS-001-FR-057`, the ball's `RigidBody.linear_damping`/
  `angular_damping` both default to `0.0` and nothing else ever bounded
  its velocity.

---

## Landing auto-orientation vs. real auto-flip/auto-roll (audit finding)
**2026-09-01** · [#127](https://github.com/baileyrd/rusty_bullet/pull/127) · `6348835`

- **`RB-PHYSICS-001-FR-057`'s own Non-goals had left open** whether real
  Rocket League's auto-flip (`CAR_AUTOFLIP_IMPULSE/TORQUE/TIME/
  NORMZ_THRESH/ROLL_THRESH`) could map onto this port's own
  `drive::LANDING_AUTO_UPRIGHT_TORQUE` "without further investigation."

---

## Real forward-speed-dependent dodge impulse scaling
**2026-09-01** · [#125](https://github.com/baileyrd/rusty_bullet/pull/125) · `5f20ac4`

- **`RB-PHYSICS-001-FR-031`'s own audit had already found real Rocket
  League's dodge impulse has "direction/speed-dependent scaling"** but
  couldn't adopt it — the audit only had `RLConst.h`'s bare constant
  declarations, not the formula they combine into.

---

## Real speed-dependent throttle taper
**2026-09-01** · [#123](https://github.com/baileyrd/rusty_bullet/pull/123) · `b729cc8`

- **`THROTTLE_ACCELERATION`'s own doc comment had named this exact gap
  since it was introduced**: full flat acceleration right up to a hard
  cutoff at `UNBOOSTED_MAX_CAR_SPEED`, not a genuine taper — "a real
  simplification (not a taper)."

---

## Hard cap on car angular speed
**2026-09-01** · [#121](https://github.com/baileyrd/rusty_bullet/pull/121) · `65c35e9`

- **Nothing previously bounded how fast sustained air control torque (or a
  dodge's own kick, or the landing-orientation assist) could spin a car**
  — holding full pitch/yaw/roll indefinitely spun a car arbitrarily fast,
  unlike real Rocket League.

---

## Boost acceleration ground/air split
**2026-09-01** · [#119](https://github.com/baileyrd/rusty_bullet/pull/119) · `4eafed3`

- **Fetched RocketSim's own `RLConst.h` directly** and found this port's
  own single flat `drive::BOOST_ACCELERATION` constant collapsed two
  genuinely distinct reference values into one: `BOOST_ACCEL_GROUND =
  2975/3` (≈991.667, exactly matching this port's existing value) and a
  distinctly higher `BOOST_ACCEL_AIR = 3175/3` (≈1058.333, about 6.5%
  more).

---

## `GOAL_HALF_WIDTH`/`GOAL_HEIGHT` reference confirmation
**2026-09-01** · [#117](https://github.com/baileyrd/rusty_bullet/pull/117) · `fd53770`

- **Fetched the current RLBot wiki's "Useful Game Values" page directly**
  (the same page `RB-PHYSICS-001-FR-036`'s own research already used to
  confirm `arena::GOAL_DEPTH`) and confirmed `arena::GOAL_HALF_WIDTH`
  (`892.755`) and `arena::GOAL_HEIGHT` (`642.775`) exact against its own
  cited "Goal center-to-post"/"Goal height" numbers — no value change, …

---

## Goal-wall/bounded-wall corner-testing overlap investigation
**2026-09-01** · [#115](https://github.com/baileyrd/rusty_bullet/pull/115) · `bf8e713`

- **Closed the one question `RB-PHYSICS-001-FR-028`'s own doc comment left
  open**: could `collision::box_vs_goal_wall`'s per-corner window test
  under-detect a car's face resting flush against the window's own edge,
  every corner just clear of it while the face's middle already overlapped
  it — the same category of concern `RB-PHYSICS-001-FR-032` …

---

## `combine_friction` defensive clamp
**2026-09-01** · [#113](https://github.com/baileyrd/rusty_bullet/pull/113) · `310f588`

- **`RB-PHYSICS-001-FR-043` fetched and read real Bullet's own
  `btManifoldResult::calculateCombinedFriction`/`calculateCombinedRestitution`
  source** to correct this spec's wrong claim about the reference's
  default combine mode, but never separately examined one more detail
  visible in that same source: real Bullet's own
  `calculateCombinedFriction` …

---

## Static-vs-dynamic combined-solve ordering investigation
**2026-09-01** · [#111](https://github.com/baileyrd/rusty_bullet/pull/111) · `524b593`

- **`PhysicsWorld::step` resolved a body's now-combined static contacts
  and its combined dynamic manifolds as two separate solves** — one fully
  resolved and applied before the other's own setup for that same body
  ever read the result — the same independent-pairwise gap
  `RB-PHYSICS-001-FR-030`/`RB-PHYSICS-001-FR-050`/`RB-PHYSICS-001-FR-051`
  already …

---

## Static multi-surface contact combined-solve investigation
**2026-09-01** · [#109](https://github.com/baileyrd/rusty_bullet/pull/109) · `6581c7f`

- **`PhysicsWorld::step` resolved a body's contact against each static
  shape type independently and sequentially** — the ground, then every
  wall, then every curve, then every corner fillet, then every goal wall,
  then every bounded wall, one independent `solver::resolve_contacts` call
  per shape — the exact independent-pairwise shape …

---

## Net-point contact combined-solve investigation
**2026-09-01** · [#107](https://github.com/baileyrd/rusty_bullet/pull/107) · `4d1a4b8`

- **`net::NetMesh::step` resolved every body-vs-net-point contact
  independently and sequentially**, one pair at a time via
  `solver::resolve_contacts_between` — the exact independent-pairwise
  shape `RB-PHYSICS-001-FR-030` already proved under-converges (and can be
  genuinely order-dependent) for a shared body touched by 2+ others in the
  same step.

---

## Velocity-aligned friction direction selection
**2026-09-01** · [#105](https://github.com/baileyrd/rusty_bullet/pull/105) · `1954adf`

- **Closes the genuine, significant divergence `RB-PHYSICS-001-FR-048`
  found and explicitly left open**: this port's `setup_rows` and
  `setup_two_body_rows` always derived both friction directions from a
  fixed, velocity-independent `plane_space(&contact.normal)` basis, where
  real Bullet's actual default aligns friction direction 1 with the
  tangential …

---

## `solver.rs` constraint-row setup/resolve reference validation
**2026-08-31** · [#103](https://github.com/baileyrd/rusty_bullet/pull/103) · `69c07b9`

- **Fetched and read Bullet's real
  `btSequentialImpulseConstraintSolver.cpp`/`.h`, `btContactSolverInfo.h`,
  and `btVector3.h` directly** to check every Bullet-reference claim
  `restitution_curve`, `plane_space`, `setup_rows`, and `resolve_row`
  make.

---

## `collision.rs` remaining closed-form shape pairings reference validation
**2026-08-31** · [#101](https://github.com/baileyrd/rusty_bullet/pull/101) · `ed8c59e`

- **Fetched and read Bullet's real `btConvexPlaneCollisionAlgorithm.cpp`/
  `.h`, `btSphereBoxCollisionAlgorithm.cpp`,
  `btSphereSphereCollisionAlgorithm.cpp`, and `btManifoldPoint.h`
  directly** to check every Bullet-reference claim `sphere_vs_plane`,
  `box_vs_plane`, `sphere_vs_box`, and `sphere_vs_sphere` make —
  `box_vs_box` was already checked this …

---

## `body.rs`/`mat3.rs` reference validation
**2026-08-31** · [#99](https://github.com/baileyrd/rusty_bullet/pull/99) · `4d3de85`

- **Fetched and read Bullet's real `btSphereShape.cpp`, `btBoxShape.cpp`,
  `btRigidBody.cpp`/`.h`, and `btMatrix3x3.h` directly** to check every
  Bullet-reference claim `body.rs`'s `Shape::local_inertia`/
  `RigidBody::update_inertia_tensor` and `mat3.rs`'s
  `Mat3::scaled_columns`/`Mat3::from_quat` make — the same rigor already
  applied to `collision.rs` …

---

## `integrate.rs` reference validation
**2026-08-31** · [PR #97](https://github.com/baileyrd/rusty_bullet/pull/97) · `cbd9918`

- **Fetched and read Bullet's real `btRigidBody.cpp`/`.h`,
  `btTransformUtil.h`, `btQuaternion.h`, and `btScalar.h` directly** to
  check every Bullet-reference claim `integrate.rs`'s own doc comments
  make — the same rigor already applied to `collision.rs` (FR-042) and
  `solver.rs` (FR-043).

---

## Stale "split impulse" Non-goals correction
**2026-08-31** · [PR #95](https://github.com/baileyrd/rusty_bullet/pull/95) · `45cb184`

- This project's own spec still carried a "Split impulse. This port always
  takes Bullet's non-split contact-resolution branch" Non-goals bullet —
  contradicted by `RB-PHYSICS-001-FR-034`'s own already-shipped
  implementation from earlier in this project.

---

## Restitution/friction combine-mode reference validation
**2026-08-31** · [PR #93](https://github.com/baileyrd/rusty_bullet/pull/93) · `aa9938d`

- **This project's own spec claimed, without ever having checked, that
  Bullet's default restitution/friction combine mode is `max` for both.**
  Fetched and read `btManifoldResult.h`/`btManifoldResult.cpp` in full and
  found that claim wrong.

---

## Box-vs-box reference validation
**2026-08-31** · [PR #91](https://github.com/baileyrd/rusty_bullet/pull/91) · `feabc32`

- **Fetched and read Bullet's own `btBoxBoxDetector::dBoxBox` reference
  source directly** to validate two "reasonable, tested choices, never
  validated against the reference" this project's own spec flagged as
  open.

---

## Sandwiched-solve convergence
**2026-08-31** · [PR #89](https://github.com/baileyrd/rusty_bullet/pull/89) · `4b0a133`

- **Investigated whether anything short of real recorded data could narrow
  `RB-PHYSICS-001-FR-030`'s own documented extreme-mass-ratio "sandwiched"
  under-convergence gap** at this crate's fixed `SOLVER_ITERATIONS = 10`.

---

## Fillet-radius calibration research
**2026-08-31** · [PR #87](https://github.com/baileyrd/rusty_bullet/pull/87) · `f92ceed`

- **A dedicated research pass looked for a real reference for
  `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS`** — the two uncalibrated
  placeholder constants FR-036's own constant-ambiguity research left
  untouched — searching this port's established reference tier
  (RocketSim/RLUtilities source, the RLBot wiki, RLGym's game values).

---

## Car-vs-net contact
**2026-08-31** · [PR #85](https://github.com/baileyrd/rusty_bullet/pull/85) · `fdbd940`

- **A car is now caught by a goal net too, not just the ball** — closes
  this port's own former Non-goal that "a car still passes straight
  through a `net::NetMesh`'s spatial footprint untouched."

---

## Wall-jump corner disambiguation
**2026-08-31** · [PR #86](https://github.com/baileyrd/rusty_bullet/pull/86) · `99234c6`

- **A wall jump at a corner now pushes off diagonally, blending both
  touched walls**, instead of firing along only one of them depending on
  iteration order. `PhysicsWorld::step`'s per-car wall-normal computation
  sums every wall a car is touching this step and normalizes the result,
  instead of `Iterator::find`-ing the first match.

---

## Sleeping
**2026-08-31** · [PR #83](https://github.com/baileyrd/rusty_bullet/pull/83) · `33c4b77`

- **A body's velocity now forcibly zeroes once it's stayed below a linear
  and an angular threshold for a sustained time**, closing the "no
  sleeping" half of the solver's own documented gap warm-starting left
  open. New `body::RigidBody::update_sleep_state`/`wake`.

---

## Ball radius and ceiling height corrections
**2026-08-31** · [PR #81](https://github.com/baileyrd/rusty_bullet/pull/81) · `ab892bf`

- **Resolved both constant ambiguities `RB-PHYSICS-001-FR-031`'s own audit
  surfaced but deliberately didn't act on**, using real source-level
  research (cloning and reading RocketSim's and RLUtilities' own source,
  and the current RLBot wiki, rather than guessing from prior
  training-data recall).

---

## Warm-starting
**2026-08-31** · [PR #79](https://github.com/baileyrd/rusty_bullet/pull/79) · `a79d923`

- **`solver::resolve_dynamic_manifolds` (every ball-vs-car/car-vs-car
  manifold) now warm-starts from the previous call's converged impulses**
  instead of zero.

---

## Split impulse
**2026-08-31** · [PR #77](https://github.com/baileyrd/rusty_bullet/pull/77) · `dedfeec`

- **Deep penetration correction no longer injects spurious velocity into a
  contact.** Every contact's normal row now also solves a second, entirely
  separate "push" pseudo-velocity channel
  (`solver::resolve_push_row`/`resolve_two_body_push_row`), fed only by
  that contact's own positional (penetration/ERP) error — never its
  velocity/restitution error, …

---

## Genuine goal net
**2026-08-31** · [PR #75](https://github.com/baileyrd/rusty_bullet/pull/75) · `e1ffb4f`

- **Each goal now has a real mass-spring net catching the ball**,
  replacing part of `RB-PHYSICS-001-FR-029`'s solid-bounding-box stand-in
  with actual springy/catching behavior — the "ball tangles in netting"
  case this project's own Non-goals had left open since FR-029 shipped.

---

## Curved-fillet narrow-phase investigation
**2026-08-31** · [PR #73](https://github.com/baileyrd/rusty_bullet/pull/73) · `51e633a`

- **Investigated a claimed corner-testing under-detection bug for a car
  vs. a curved fillet, found it doesn't exist — no change to the narrow
  phase itself.** `RB-PHYSICS-001-FR-027`'s own doc comments claimed
  `box_vs_quarter_pipe`/`box_vs_corner_fillet`'s per-corner technique was
  an approximation, not a full convex-vs-curved-surface narrow phase: a …

---

## Constant-calibration audit
**2026-08-31** · [PR #71](https://github.com/baileyrd/rusty_bullet/pull/71) · `4c7b9a2`

- **A scoped audit of every uncalibrated placeholder constant** in
  `drive.rs`/`arena.rs`, sourced against the community reverse-engineering
  effort — deliberately does NOT close `RB-PHYSICS-001-FR-005`'s real-data
  calibration, which still needs `PHASE-0-EXIT`.

---

## Combined multi-body solve
**2026-08-31** · [PR #69](https://github.com/baileyrd/rusty_bullet/pull/69) · `dfbefb4`

- **`PhysicsWorld::step` now resolves every ball-vs-car and car-vs-car
  contact manifold together as one combined multi-body solve**, instead of
  resolving each pair independently and fully applying it before the next
  pair's setup even reads a body's velocity — closing the "3+ bodies
  mutually touching in the same step" approximation this project has …

---

## Modeled goal interior
**2026-08-31** · [PR #67](https://github.com/baileyrd/rusty_bullet/pull/67) · `9b69c0c`

- **A ball or car passing through a goal-mouth window now settles inside a
  bounded goal box** instead of sailing forever into open, unbounded space
  — closing the "modeled goal interior/net" gap repeated across
  `RB-PHYSICS-001-FR-024` through `FR-028`'s own "Still not modeled"
  lists.

---

## Car actually driving into a goal
**2026-08-31** · [PR #65](https://github.com/baileyrd/rusty_bullet/pull/65) · `3141f1e`

- **A car (box) can now actually drive into a goal**, closing the last
  goal-related Non-goal repeated across `RB-PHYSICS-001-FR-024` through
  `FR-027` — until now, `collision::contacts_vs_goal_wall` sent a car
  straight through to an unwindowed `contacts_vs_plane`, so it always
  collided with the full, solid back wall even though the ball already …

---

## Car deflection by curved fillets
**2026-08-31** · [PR #63](https://github.com/baileyrd/rusty_bullet/pull/63) · `f13e5f5`

- **A car (box) is now actually deflected by every curved fillet in this
  port**, closing the Non-goal repeated across every fillet increment
  since `RB-PHYSICS-001-FR-020` — until now, a car drove straight through
  wall-to-floor/ceiling seams, corner-wall vertical edges, compound
  corners, and goal-cutout edges, untouched;

---

## Goal post-crossbar corner fillets
**2026-08-31** · [#61](https://github.com/baileyrd/rusty_bullet/pull/61) · `c179716`

- **Rounds off the two compound corners per goal where a post's own
  vertical edge fillet meets the crossbar's own horizontal edge fillet**,
  one per post per goal (4 total) — closing a gap
  `RB-PHYSICS-001-FR-024`'s own doc comment explicitly flagged as
  deliberately not blended into a single smooth vertex.

---

## Corner-wall floor/ceiling arch radius
**2026-08-31** · [#59](https://github.com/baileyrd/rusty_bullet/pull/59) · `ff1391a`

- **A diagonal corner wall's own floor-seam and ceiling-seam fillets are
  now distinctly larger than a cardinal wall's**, matching real Rocket
  League's noticeably bigger, more swept corner-boost curve rather than a
  scaled-down copy of a cardinal wall's small rounding.

---

## Goal cutouts
**2026-08-30** · [#57](https://github.com/baileyrd/rusty_bullet/pull/57) · `34234b6`

- **Opens an actual goal-mouth window in each back wall**, where every
  prior increment had a single solid, flat plane spanning the full width.

---

## Compound-corner fillets
**2026-08-30** · [#55](https://github.com/baileyrd/rusty_bullet/pull/55) · `5d2db86`

- **Rounds off the last 16 sharp vertices in the standard arena's vertical
  boundary** — the compound corners where a corner wall's own
  vertical-edge fillet (`FR-022`) meets a floor- or ceiling-seam fillet
  (`FR-020`/`FR-021`), near that corner wall's own top or bottom endpoint.

---

## Curved corner-wall vertical-edge fillets
**2026-08-30** · [#53](https://github.com/baileyrd/rusty_bullet/pull/53) · `d466ae2`

- **Rounds off the standard arena's last remaining sharp edges** — the 8
  vertical edges where each of the 4 diagonal corner walls meets its
  neighboring side or back wall.

---

## Curved corner-wall-to-floor/wall-to-ceiling transitions
**2026-08-30** · [#51](https://github.com/baileyrd/rusty_bullet/pull/51) · `d746d08`

- **Extends `RB-PHYSICS-001-FR-020`'s fillet treatment to the 4 diagonal
  corner walls** `RB-PHYSICS-001-FR-019` introduced —
  `arena::standard_curves` now returns 16 `StaticQuarterPipe`s (still one
  floor-side and one ceiling-side fillet per wall, now for all 9 walls)
  instead of 8.

---

## Curved wall-to-floor/wall-to-ceiling transitions
**2026-08-30** · [#49](https://github.com/baileyrd/rusty_bullet/pull/49) · `8053a71`

- **Added:** a new `body::StaticQuarterPipe` shape — an immovable
  partial-cylinder fillet connecting two perpendicular flat planes,
  infinite along its own axis like `StaticPlane` — and `collision::
  contacts_vs_quarter_pipe`, a sphere-only narrow-phase test
  (`RB-PHYSICS-001-FR-020`).

---

## Modeled arena footprint
**2026-08-30** · [#47](https://github.com/baileyrd/rusty_bullet/pull/47) · `cc68213`

- **Added:** a new `arena` module builds Rocket League's real
  standard-arena boundary entirely from `RB-PHYSICS-001-FR-013`'s existing
  generic `StaticPlane`/`PhysicsWorld::with_wall` machinery
  (`RB-PHYSICS-001-FR-019`) — no new collision code, since a ceiling and a
  corner-cut wall are each just another flat plane.

---

## Landing auto-orientation
**2026-08-30** · [#45](https://github.com/baileyrd/rusty_bullet/pull/45) · `b5ed2cd`

- **Added:** `drive::apply_driven_forces` gains a gentle continuous
  restoring torque, applied while airborne, nudging the car's local up
  axis back toward world up (`RB-PHYSICS-001-FR-018`). Real Rocket League
  triggers this assist on approach to the ground;

---

## Wall-jump dodge
**2026-08-30** · [#43](https://github.com/baileyrd/rusty_bullet/pull/43) · `3b08fdf`

- **Added:** the wall jump's own fresh press (`RB-PHYSICS-001-FR-013`) now
  checks `ControllerInput.pitch`/`roll` against `DODGE_DEADZONE`
  (`RB-PHYSICS-001-FR-017`), the same check the ground double jump's press
  already uses (`RB-PHYSICS-001-FR-014`): at or above it on either axis, a
  **wall-jump dodge** fires instead of the plain fixed push-off — the …

---

## Flip-cancel
**2026-08-30** · [#41](https://github.com/baileyrd/rusty_bullet/pull/41) · `14d986d`

- **Added:** a dodge's spin (`RB-PHYSICS-001-FR-014`) can now be canceled
  early (`RB-PHYSICS-001-FR-016`) — a further fresh `ControllerInput.jump`
  press while airborne, not touching a wall, with the double jump already
  spent by that dodge, zeroes `RigidBody.angular_velocity` outright
  instead of leaving the flip to spin indefinitely.

---

## Variable jump height input
**2026-08-30** · [#39](https://github.com/baileyrd/rusty_bullet/pull/39) · `9266c6c`

- **Added:** the ground jump (`RB-PHYSICS-001-FR-010`) gains a hold window
  (`RB-PHYSICS-001-FR-015`) — continuing to hold `ControllerInput.jump`
  after the fresh press that fires it adds a continuous
  `JUMP_HOLD_ACCELERATION` upward force, for up to
  `JUMP_HOLD_MAX_DURATION` seconds, on top of the press's own fixed
  `JUMP_SPEED` impulse.

---

## Dodge input
**2026-08-30** · [#37](https://github.com/baileyrd/rusty_bullet/pull/37) · `72150f5`

- **Added:** the double jump's fresh press (`RB-PHYSICS-001-FR-014`) now
  checks `ControllerInput.pitch`/`roll` at the moment it fires: at or
  above a new `DODGE_DEADZONE` on either axis, it fires a directional
  dodge instead of the plain vertical double jump — a purely horizontal
  `DODGE_SPEED` impulse (along `forward_axis` for `pitch`, `right_axis` …

---

## Wall jump input
**2026-08-30** · [#35](https://github.com/baileyrd/rusty_bullet/pull/35) · `b748b86`

- **Added:** `PhysicsWorld` gains arena walls (`RB-PHYSICS-001-FR-013`) —
  `walls: Vec<StaticPlane>` and a `with_wall` builder (mirroring
  `with_car`).

---

## Double jump input
**2026-08-30** · [#33](https://github.com/baileyrd/rusty_bullet/pull/33) · `7c9524a`

- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains a
  double jump (`RB-PHYSICS-001-FR-012`) — one more, identical `JUMP_SPEED`
  instantaneous upward velocity change fired on a fresh (rising-edge)
  press of `ControllerInput.jump` while the car is airborne, reusing the
  ground jump's own edge detection rather than a second edge-detector.

---

## Air control input
**2026-08-29** · [#31](https://github.com/baileyrd/rusty_bullet/pull/31) · `431ff56`

- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains air
  control (`RB-PHYSICS-001-FR-011`) — torque about the car's local right,
  up, and forward axes, scaled directly by `ControllerInput.pitch`/`yaw`/
  `roll` (each an `Option<f32>`, `None` treated as zero) times one shared
  `AIR_CONTROL_TORQUE` constant, applied whenever the car is *not* …

---

## Jump input
**2026-08-29** · [#29](https://github.com/baileyrd/rusty_bullet/pull/29) · `689b006`

- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains a
  single ground jump (`RB-PHYSICS-001-FR-010`) — a fixed `JUMP_SPEED`
  instantaneous upward velocity change (via `RigidBody::apply_impulse`,
  not a continuous force) fired on the *rising edge* of
  `ControllerInput.jump` while the car is grounded — a fresh press, not
  merely held.

---

## Handbrake input
**2026-08-29** · [#27](https://github.com/baileyrd/rusty_bullet/pull/27) · `56f9cb4`

- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains a
  handbrake mechanic (`RB-PHYSICS-001-FR-009`) — while
  `ControllerInput.handbrake` is held and the car is grounded (gated like
  throttle/steering — a free-floating box has no wheels to lock), the
  car's `RigidBody.friction` is temporarily multiplied by a new …

---

## Boost input
**2026-08-29** · [#25](https://github.com/baileyrd/rusty_bullet/pull/25) · `40e70cd`

- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains a boost
  force (`RB-PHYSICS-001-FR-008`) — a flat forward force
  (`BOOST_ACCELERATION * mass`, not speed-tapered like throttle, capped at
  the same `MAX_CAR_SPEED` ceiling) applied whenever
  `ControllerInput.boost` is set and the car has boost remaining.

---

## Driven car input (ground throttle and steering)
**2026-08-29** · [#23](https://github.com/baileyrd/rusty_bullet/pull/23) · `f1a0381`

- **Added:** `rb_physics_bullet::drive`, coupling
  `rb_domain::ControllerInput` into a throttle force (along the car's
  local forward axis, capped at `MAX_CAR_SPEED`) and a steering torque
  (about the car's local up axis, scaled by current speed so a stationary
  car can't turn in place) — `RB-PHYSICS-001-FR-007`.

---

## Multi-car PhysicsWorld support
**2026-08-29** · [#21](https://github.com/baileyrd/rusty_bullet/pull/21) · `28b8d4c`

- **Changed (breaking):** `PhysicsWorld.car: Option<RigidBody>` is
  replaced by `cars: Vec<RigidBody>`. `with_car` now appends, so calling
  it repeatedly builds a scene with any number of cars —
  `PhysicsWorld::new(ball, ground).with_car(a).with_car(b)` is a two-car
  scene.

---

## Car-vs-car collision detection
**2026-08-29** · [#19](https://github.com/baileyrd/rusty_bullet/pull/19) · `2eddfe7`

- **Added:** `collision::box_vs_box`, a general separating-axis test (SAT)
  between two oriented boxes (`RB-PHYSICS-001-FR-006`) — 3+3 face axes
  plus 9 edge-pair cross-product axes, the same overall structure as
  `btBoxBoxDetector::dBoxBox`. When every axis shows overlap, the
  minimum-penetration axis becomes the contact normal;

---

## Ball-vs-car collision
**2026-08-28** · [#17](https://github.com/baileyrd/rusty_bullet/pull/17) · `2f12c8f`

- **Added:** `rb_physics_bullet` gains analytic sphere-vs-box contact
  generation (`collision::sphere_vs_box`, dispatched via
  `collision::contact_between`) completing `RB-PHYSICS-001-FR-004` — the
  ball and car now actually collide with each other, not just the ground.
  A closed-form closest-point-on-box query handles the ordinary case;

---

## Box-shaped car bodies
**2026-08-28** · [#15](https://github.com/baileyrd/rusty_bullet/pull/15) · `24468cf`

- **Added:** `rb_physics_bullet` gains a unified `RigidBody`/`Shape`
  design (`RB-PHYSICS-001-FR-004`) — one rigid-body type serving both the
  ball (sphere) and a car (box), matching Bullet's own architecture
  (`btRigidBody` plus a polymorphic `btCollisionShape`) rather than a
  separate type per shape. `Sphere` is gone;

## Timestamp-tolerant alignment
**2026-08-28** · [#13](https://github.com/baileyrd/rusty_bullet/pull/13) (merge commit `59266ea`)

- **Added:** `rb_domain::divergence::score` now aligns frames by nearest
  `timestamp_secs` instead of list index (`RB-VERIFY-003-FR-003`) — an
  `O(recorded.len() + candidate.len())` merge over both sequences'
  existing chronological order, not a binary search per frame.

## Car-state divergence scoring
**2026-08-28** · [#11](https://github.com/baileyrd/rusty_bullet/pull/11) (merge commit `a1b8a47`)

- **Added:** `rb_domain::divergence::DivergenceScore` gains a `cars:
  CarDivergence` field — mean/max car position distance, rotation distance
  (radians), and velocity distance, plus the number of car pairs compared
  (`RB-VERIFY-003-FR-002`). Cars are matched between the recorded and
  candidate sequences by `player_id` within each frame pair;

## Divergence scoring CLI wiring
**2026-08-28** · [#9](https://github.com/baileyrd/rusty_bullet/pull/9) (merge commit `f10d017`)

- **Added:** `rb_verify_cli::score_replay_against_capture` (new `lib.rs`)
  — the actual composition-root wiring, ingesting a replay via
  `rb_replay_ingest` and a capture via `rb_capture_ingest` and running
  `rb_domain::divergence::score` on the results.

## BakkesMod capture ingestion — JSON-Lines parser + shared input schema
**2026-08-28** · [#7](https://github.com/baileyrd/rusty_bullet/pull/7) (merge commit `dc7e82f`)

- **Added:** `rb_domain::ControllerInput` and `CarState.input:
  Option<ControllerInput>` (ADR-0005) — a shared controller-input schema
  for both ingestion adapters. `throttle`/`steer` are always a number;

## Replay ingestion — local real-corpus validation gate
**2026-08-28** · [#5](https://github.com/baileyrd/rusty_bullet/pull/5) (merge commit `0b2253d`)

- **Added:** `corpus_check`, a local/gitignored-corpus health-check binary
  (`cargo run -p rb_replay_ingest --bin corpus_check [dir]`,
  `RB-VERIFY-001-NFR-003`) — runs the real `boxcars` + `subtr-actor` +
  `convert` pipeline against every `.replay` file in a directory (default
  `replays/` at the workspace root, already `.gitignore`d) and exits
  non-zero …

## Replay ingestion — boxcars + subtr-actor
**2026-08-28** · [#3](https://github.com/baileyrd/rusty_bullet/pull/3) (merge commit `93ad0e9`)

- **Added:** `rb_replay_ingest` now really parses `.replay` files
  (`RB-VERIFY-001-FR-001/002/003`): `boxcars` parses the raw
  replay/network stream, `subtr-actor` resolves it into frame-indexed
  ball/car `RigidBody` state, and a new `convert.rs` maps that into
  `rb_domain::PhysicsFrame`.

## Physics core v0 — Bullet3 port (sphere vs. ground)
**2026-08-28** · [#1](https://github.com/baileyrd/rusty_bullet/pull/1) (merge commit `7bdc3fc`)

- **Added:** `rb_physics_bullet`, a from-scratch Rust port of specific
  Bullet3 (zlib-licensed) algorithms — rigid-body integration
  (`btRigidBody`) and the sequential-impulse contact solver
  (`btSequentialImpulseConstraintSolver`) — scoped to a dynamic sphere
  (the ball) against a static plane (the ground).

## Repo bootstrap — full lifecycle baseline
**2026-08-28** · landed directly on `main` at commit `5be2078` (predates this repo's "always PR" convention; no PR exists for it)

- **Added:** Full `rust-repo-lifecycle` + `repo-config` bootstrap:
  charter, system architecture, a 6-spec tree (`RB-VERIFY-001/002/003`
  fully specified for Phase 0;
