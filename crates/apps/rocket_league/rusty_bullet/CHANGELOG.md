# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- Repo lifecycle bootstrap: charter, system architecture, spec tree
  (`RB-VERIFY-001/002/003`, `RB-PHYSICS-001`, `RB-SIM-001`, `RB-NET-001`),
  ADR-0001..0003, research backlog, roadmap, traceability, AGENTS.md,
  WORKFLOW.md, and the standard governance file set.
- Cargo workspace (`rb_domain`, `rb_replay_ingest`, `rb_capture_ingest`,
  `rb_verify_cli`) with a working, unit-tested divergence-scoring algorithm
  and stubbed ingestion adapters.
- `rb_physics_bullet`: a Rust port of Bullet3's rigid-body integration and
  sequential-impulse contact solver, scoped to a dynamic sphere vs. a
  static plane (ADR-0004). Vector/quaternion algebra added to
  `rb_domain::state`.
- `rb_replay_ingest`: real `.replay` parsing via `boxcars` + `subtr-actor`
  (`RB-VERIFY-001-FR-001/002/003`), verified against a vendored real
  replay fixture.
- `rb_replay_ingest`: `corpus_check` bin, a local/gitignored-corpus health
  check (`RB-VERIFY-001-NFR-003`) — validated against 40 of the owner's
  real match replays (40/40 clean), closing the "runs correctly on real
  owner data at scale" half of `RB-VERIFY-001`'s owner-data acceptance
  criterion.
- `rb_domain::ControllerInput` and `CarState.input` (ADR-0005), a shared
  controller-input schema; `rb_replay_ingest` now attaches recovered input
  to every car (`RB-VERIFY-001-FR-004`).
- `rb_capture_ingest`: real capture-file parsing via a new JSON-Lines
  format (`RB-VERIFY-002-FR-002`/`NFR-001`, ADR-0005), verified against a
  synthetic fixture. The BakkesMod-side plugin that would write a real
  capture (`RB-VERIFY-002-FR-001`) is not yet built.
- `rb_verify_cli`: `score_replay_against_capture`, wiring ingestion to
  `rb_domain::divergence::score`. Manually run end-to-end against a real
  replay fixture and a capture file; not yet a fidelity measurement.
- `rb_domain::divergence::score` now also scores car position/rotation/
  velocity divergence (`RB-VERIFY-003-FR-002`), matching cars between
  sequences by `player_id`. New `Quat::angle_to` computes rotation
  distance.
- `rb_domain::divergence::score` now aligns frames by nearest timestamp
  instead of list index (`RB-VERIFY-003-FR-003`), within a new required
  `max_timestamp_delta_secs` parameter. `rb_verify_cli` gains
  `DEFAULT_MAX_TIMESTAMP_DELTA_SECS` and an optional third CLI argument
  to override it. `RB-VERIFY-003` now has all three functional
  requirements implemented.
- `rb_physics_bullet`: box-shaped car bodies (`RB-PHYSICS-001-FR-004`) —
  a unified `RigidBody`/`Shape` design, a general 3x3 inverse inertia
  tensor (`Mat3`), analytic box-vs-plane contact generation (1-4 points),
  and multi-contact manifold resolution in the solver. `PhysicsWorld`
  gains an optional car body (`with_car`). Box-vs-sphere collision and
  driven car input remain not implemented.
- `rb_physics_bullet`: ball-vs-car collision, completing
  `RB-PHYSICS-001-FR-004` — analytic sphere-vs-box contact generation
  (`collision::sphere_vs_box`/`contact_between`, handling both the
  ordinary and deep-penetration cases) and a two-dynamic-body
  sequential-impulse solver path (`solver::resolve_contact_between`).
  `rb_domain::Quat` gains `conjugate`. Box-vs-box collision and driven car
  input remain not implemented.
- `rb_physics_bullet`: car-vs-car collision *detection*
  (`RB-PHYSICS-001-FR-006`) — `collision::box_vs_box`, a 15-axis
  separating-axis test between two oriented boxes, producing a clipped
  face manifold (0-4 points) or a single edge-edge point. Not wired into
  `PhysicsWorld`: this scope has exactly one car, so the collision has no
  live caller yet — multi-car `PhysicsWorld` support remains not
  implemented.
- `rb_physics_bullet`: multi-car `PhysicsWorld` support, completing
  `RB-PHYSICS-001-FR-006` — `PhysicsWorld::step` now resolves every car's
  ground contact, every ball-vs-car pair, and every car-vs-car pair
  (running `box_vs_box` for real in a live scene for the first time), one
  pair at a time. A combined multi-body solve across 3+ simultaneously
  touching bodies and driven car input remain not implemented.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-007`) — couples
  `rb_domain::ControllerInput` into ground-driving throttle force and
  steering torque on a car, gated on ground contact. `PhysicsWorld` gains
  `set_car_input` (a car's current, persistent input) and `frame()` now
  reports it. Boost, jump, air control, and handbrake remain not
  implemented.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-008`) — boost: a flat
  forward force (not speed-tapered like throttle), not gated on ground
  contact (works identically airborne). `PhysicsWorld` gains a depletable
  `car_boost: Vec<f32>` resource and `set_car_boost`; holding boost drains
  it over time, even once the force stops applying at `MAX_CAR_SPEED`;
  `frame()` now reports each car's actual `boost_amount`. Jump, air
  control, and handbrake remain not implemented.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-009`) — handbrake: while
  held and grounded, temporarily reduces the car's `RigidBody.friction`
  (restored on release), letting existing momentum carry it into a slide.
  `PhysicsWorld` gains `car_base_friction: Vec<f32>`, snapshotted per car
  by `with_car`, so release restores each car's own base friction. Jump
  and air control remain not implemented.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-010`) — a single ground
  jump: a fixed instantaneous upward velocity change fired on the rising
  edge of `ControllerInput.jump` while grounded, gated so holding or a
  release-then-re-press mid-air doesn't re-fire it. `PhysicsWorld` gains
  `car_jump_held: Vec<bool>` to track the rising-edge state per car.
  Double jump/dodge, variable jump height, wall jump, and air control
  remain not implemented.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-011`) — air control:
  torque about the car's local right/up/forward axes from
  `ControllerInput.pitch`/`yaw`/`roll`, gated on the car *not* touching
  the ground, not speed-scaled (unlike ground steering). Double
  jump/dodge, variable jump height, and wall jump remain not implemented.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-012`) — double jump: one
  more, identical `JUMP_SPEED` impulse fired on a fresh airborne press of
  `ControllerInput.jump`, reusing the ground jump's own rising-edge
  detection and the `JUMP_SPEED` constant (now `pub`) itself. Gated on a
  new per-car `double_jump_available` flag rather than ground contact —
  restored on landing, consumed by use. `PhysicsWorld` gains
  `car_double_jump_available: Vec<bool>`. The dodge directional
  impulse/torque, variable jump height, and wall jump remain not
  implemented.
- `rb_physics_bullet` (`RB-PHYSICS-001-FR-013`) — arena walls and wall
  jump: `PhysicsWorld` gains `walls: Vec<StaticPlane>`/`with_wall`, and
  every body now collides with every wall via the same
  body-vs-static-plane machinery the ground already uses
  (`resolve_ground_contact` renamed `resolve_plane_contact`).
  `rb_physics_bullet::drive::apply_driven_forces` gains a wall jump — an
  outward-plus-upward impulse fired on a fresh airborne jump press while
  touching a wall, taking priority over the double jump on that press but
  restoring (not consuming) `double_jump_available` on mere contact. The
  dodge directional impulse/torque, variable jump height, and a modeled
  arena footprint beyond generic flat walls remain not implemented.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-014`) — dodge: the double
  jump's fresh press now checks `ControllerInput.pitch`/`roll`, firing a
  directional dodge (a horizontal `DODGE_SPEED` impulse plus an
  instantaneous `DODGE_ANGULAR_SPEED` spin) instead of the plain vertical
  double jump whenever either exceeds a new `DODGE_DEADZONE`, reusing air
  control's own pitch/roll axis and sign conventions. Shares the double
  jump's `double_jump_available` resource; wall jump never dodges,
  regardless of stick input. `DODGE_SPEED` and `WALL_JUMP_HORIZONTAL_SPEED`
  are now `pub`. A dodge variant of the wall jump, flip-cancel, landing
  auto-orientation assistance, and variable jump height remain not
  implemented.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-015`) — variable jump
  height: the ground jump gains a hold window — continuing to hold
  `ControllerInput.jump` after the fresh press that fires it adds a
  continuous `JUMP_HOLD_ACCELERATION` upward force, for up to
  `JUMP_HOLD_MAX_DURATION` seconds, on top of the fixed `JUMP_SPEED`
  impulse; releasing `jump` (or the window running out) stops the extra
  acceleration immediately. A new `jump_hold_time_remaining` is checked
  and decremented before the ground jump's own fresh-press handling can
  re-arm it, so a fresh press's own step is unaffected. Scoped to the
  ground jump alone — the double jump, a dodge, and the wall jump remain
  fixed-impulse. `PhysicsWorld` gains `car_jump_hold_time_remaining:
  Vec<f32>`.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-016`) — flip-cancel: a
  dodge's spin can now be canceled early — a further fresh
  `ControllerInput.jump` press while airborne, not touching a wall, with
  the double jump already spent, zeroes `RigidBody.angular_velocity`
  outright. A new `dodge_flip_active` flag tracks this: the dodge branch
  sets it, the plain-double-jump branch explicitly clears it (preventing a
  stale flag from an earlier dodge from leaking into a later unrelated
  double jump). Doesn't touch linear velocity or `double_jump_available`;
  wall jump keeps priority, unchanged. `PhysicsWorld` gains
  `car_dodge_flip_active: Vec<bool>`.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-017`) — wall-jump dodge:
  the wall jump's own fresh press now checks
  `ControllerInput.pitch`/`roll` against `DODGE_DEADZONE`, same as the
  ground double jump; at or above it, a wall-jump dodge fires (the wall
  push-off combined with a `DODGE_SPEED` component and
  `DODGE_ANGULAR_SPEED` spin, arming `dodge_flip_active`); below it, the
  plain wall jump fires unchanged. Unlike the plain wall jump, the dodge
  variant consumes `double_jump_available` — a documented simplification,
  since gating it on that flag would be vacuous (wall touch always
  restores it first). No new physics constants. Two pre-existing tests
  asserting the old "wall jump always ignores stick input" premise were
  repurposed to assert the new behavior.
### Changed
- `rb_verify_cli`'s `main.rs` is now a thin CLI wrapper over the new
  `lib.rs`; `rb-verify`'s output is a human-readable summary instead of a
  raw `Debug` dump, now including car-divergence stats.
- `rb_physics_bullet`'s `Sphere` type is replaced by `RigidBody` (with a
  `Shape` enum for sphere/box); `RigidBody::sphere(...)` replaces
  `Sphere::new(...)`.
- `rb_physics_bullet::PhysicsWorld::step` is restructured into Bullet's
  actual staged pipeline (integrate every body's velocity → resolve every
  contact → integrate every body's transform) instead of stepping each
  body fully in isolation, so ball-vs-car contact resolution sees the same
  pre-integration state ground contacts do.
- `rb_physics_bullet::collision::contact_between` is renamed
  `contacts_between` and now returns `Vec<Contact>` (was
  `Option<Contact>`), and `solver::resolve_contact_between` is renamed
  `resolve_contacts_between` and now takes a manifold slice (was a single
  `&Contact`) — needed to support box-vs-box's up-to-4-point case
  uniformly with sphere-vs-box's single point.
- `rb_physics_bullet::PhysicsWorld.car: Option<RigidBody>` is renamed
  `cars: Vec<RigidBody>` (breaking); `with_car` now appends instead of
  replacing, so it's callable any number of times to build a multi-car
  scene.
- `rb_physics_bullet::PhysicsWorld::frame()` now reports each car's
  current `ControllerInput` as `Some(input)` instead of always `None`.
### Fixed
- `rb_capture_ingest`'s synthetic test fixture had timestamps that didn't
  overlap the vendored replay fixture's real timeline (off by ~11.78s) —
  invisible under the old index-pairwise frame comparison, surfaced once
  real timestamp alignment landed. Corrected.
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
