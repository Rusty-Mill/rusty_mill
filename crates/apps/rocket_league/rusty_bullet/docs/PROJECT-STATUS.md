# Project Status

- Last verified main commit: `2eddfe7` (merge of [#19](https://github.com/baileyrd/rusty_bullet/pull/19))
- Verified at: 2026-08-28
- Current milestone: `PHASE-1-PHYSICS-CORE` (box-shaped car bodies, general 3x3 inertia, multi-contact resolution, ball-vs-car collision, and car-vs-car collision *detection* all implemented in `rb_physics_bullet`; multi-car `PhysicsWorld` wiring, driven car input, and constant calibration still open) — In Progress
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
  ball-vs-car already uses. **Not** wired up: `PhysicsWorld` still models
  exactly one car, so `box_vs_box` has no live caller in a real simulated
  scene — multi-car `PhysicsWorld` support is separate, larger,
  explicitly open follow-up work (see Blocked/Next), not silently done or
  silently skipped. 4 new unit tests in `rb_physics_bullet` (62 total).

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
- `RB-PHYSICS-001`'s multi-car `PhysicsWorld` support (needed to give the
  new `box_vs_box` car-vs-car collision detection a real caller) and
  driven car input — both real, not-yet-started follow-up work (see the
  spec's Non-goals/Open questions); multi-car support doesn't block the
  current scope (only one car exists in any real simulated scene today),
  but driven input is needed before a car in this physics core can do
  anything beyond free-fall, resting on the ground, and passively bouncing
  off the ball.

## Next

1. `RB-VERIFY-002-FR-001` — write, build, and run the BakkesMod-side
   capture plugin against ADR-0005's JSON-Lines format, on the owner's own
   Windows/BakkesMod/game environment (this sandbox can't).
2. Driven car input, and/or multi-car `PhysicsWorld` support (to give
   `box_vs_box` a real caller) — both real follow-up work for
   `rb_physics_bullet`; `RB-PHYSICS-001-FR-005` (constant calibration)
   needs `PHASE-0-EXIT` real data regardless.

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (112 tests: 23 in `rb_domain`, 62 in
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
