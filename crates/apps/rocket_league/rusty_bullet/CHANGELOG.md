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
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-018`) — landing
  auto-orientation assist: a gentle continuous restoring torque, applied
  while airborne, nudging the car's local up axis back toward world up
  (`up_axis(car).cross(&world_up) * LANDING_AUTO_UPRIGHT_TORQUE`, whose
  magnitude is already proportional to the tilt angle's sine). Gated on no
  active `pitch`/`roll` this step and no fresh `jump` press this step
  (avoiding a same-step conflict with a dodge's/wall-jump-dodge's/
  double-jump's/flip-cancel's own direct angular-velocity change). Applies
  continuously whenever airborne rather than only near the ground — this
  port has no raycast/distance query to replicate real Rocket League's
  actual ground-proximity trigger. New constant
  `LANDING_AUTO_UPRIGHT_TORQUE` is an uncalibrated placeholder, one order
  of magnitude smaller than `AIR_CONTROL_TORQUE`. No new `PhysicsWorld`
  state.
- `rb_physics_bullet::arena` (`RB-PHYSICS-001-FR-019`) — modeled arena
  footprint: a new module builds Rocket League's real standard-arena
  boundary from FR-013's existing `StaticPlane`/`with_wall` machinery (no
  new collision code). `standard_ground` is the flat floor at `z = 0`;
  `standard_walls` returns 9 planes — 2 side walls, 2 back walls, a
  ceiling, and 4 diagonal corner walls cutting off the true rectangular
  corner, giving the field its real octagonal footprint. `SIDE_WALL_X`
  (4096), `BACK_WALL_Y` (5120), and `CEILING_Z` (2044) are commonly-cited
  field dimensions; the corner inset (`CORNER_LENGTH`) is this project's
  own uncalibrated placeholder with no real-mesh reference. New
  `PhysicsWorld::standard_arena` convenience constructor wires both into a
  scene in one call, alongside the existing `PhysicsWorld::new`/`with_wall`
  ad-hoc-wall capability. Curved wall-to-floor/wall-to-ceiling transitions,
  goal cutouts, and corner-touch disambiguation for wall-jump purposes
  remain not modeled.
- `rb_physics_bullet::body`/`collision`/`world`/`arena`
  (`RB-PHYSICS-001-FR-020`) — curved wall-to-floor/wall-to-ceiling
  transitions: a new `StaticQuarterPipe` shape (an immovable
  partial-cylinder fillet, infinite along its own axis) and
  `contacts_vs_quarter_pipe` (sphere-only; a box always returns no
  contact). The playable side is the *inside* of the fillet's concave face
  (like a skateboard quarter-pipe): governed only within a 90-degree
  sector, contact fires as the sphere's surface crosses the fillet's own
  radius from inside, pushing back toward the axis (the opposite direction
  from a flat plane's push). `StaticQuarterPipe::between_planes` derives a
  fillet's geometry automatically from two perpendicular, axis-aligned
  flat planes. `PhysicsWorld` gains `curves`/`with_curve`/
  `resolve_curve_contact`; `solver::resolve_contacts`'s second parameter
  changed from `&StaticPlane` to plain `restitution`/`friction` (the only
  two fields it ever used) so the same solver path serves a fillet too.
  `arena::standard_curves` builds the 8 cardinal-wall fillets (new
  uncalibrated placeholder `FILLET_RADIUS`); `PhysicsWorld::standard_arena`
  now adds these alongside its 9 walls. A car (box) actually being
  deflected by a fillet, fillets at the 4 diagonal corner walls, and goal
  cutouts remain not modeled.
- `rb_physics_bullet::arena` (`RB-PHYSICS-001-FR-021`) — curved
  corner-wall-to-floor/wall-to-ceiling transitions, extending FR-020 to the
  4 diagonal corner walls: `arena::standard_curves` now returns 16
  `StaticQuarterPipe`s (one floor-side and one ceiling-side per wall, all 9
  walls) instead of 8. `StaticQuarterPipe::between_planes` needed no code
  changes — its real correctness requirement was never "axis-aligned
  planes," only that the two bridged planes' normals are mutually
  perpendicular, which holds for a corner wall meeting the floor/ceiling
  regardless of the corner wall's own horizontal rotation. A corner wall's
  fillet `axis_direction` is computed via a cross product
  (`floor.normal.cross(&wall.normal)`, already unit length by construction)
  rather than hand-picked, since it isn't a coordinate axis the way a
  cardinal wall's is. A car actually being deflected by any fillet, a
  fillet at a corner wall's own vertical edges (now implemented, see
  FR-022), and goal cutouts remain not modeled.
- `rb_physics_bullet::body`/`collision`/`arena` (`RB-PHYSICS-001-FR-022`) —
  curved corner-wall vertical-edge fillets: rounds off the 8 remaining
  sharp edges in the standard arena's octagonal footprint, where each of
  the 4 diagonal corner walls meets its neighboring side or back wall.
  `arena::standard_curves` now returns 24 `StaticQuarterPipe`s (the 16
  floor/ceiling-seam fillets already built, plus 8 vertical-edge fillets).
  Unlike every prior fillet, the two planes a vertical-edge fillet bridges
  aren't perpendicular (a corner wall meets its neighbor at 135 degrees,
  not 90) — `StaticQuarterPipe::between_planes` is now fully general to
  handle this: it solves the axis point as a real 2x2 linear system rather
  than assuming orthogonal normals, its own sector angle comes out to
  exactly the angle between the two planes' normals (45 degrees here, 90
  for a floor/ceiling seam), and it self-corrects a "backwards"
  `axis_direction` internally so a caller can pass either of the two
  opposite directions along the shared edge line. `sphere_vs_quarter_pipe`'s
  sector-membership test is likewise generalized from a two-dot-products
  shortcut (only correct for a 90-degree sector) to a signed-cross-product
  test valid for any sector up to 180 degrees. `FILLET_RADIUS` is reused
  as-is once again. A car actually being deflected by any fillet, the
  compound corner where a vertical-edge fillet meets a floor/ceiling-seam
  fillet (now implemented, see FR-023), and goal cutouts remain not
  modeled.
- `rb_physics_bullet::body`/`collision`/`arena` (`RB-PHYSICS-001-FR-023`) —
  compound-corner fillets: rounds off the last 16 sharp vertices in the
  standard arena's vertical boundary, where a corner wall's own
  vertical-edge fillet (FR-022) meets a floor- or ceiling-seam fillet
  (FR-020/FR-021). Introduces a new static shape, `body::StaticCornerFillet`
  (an immovable sphere blending three flat planes at a vertex, since no
  cylindrical `StaticQuarterPipe` can blend three planes at once), with
  constructor `between_three_planes` solving the center as the common
  intersection of the three planes' pairwise `StaticQuarterPipe::
  between_planes` axis lines via the cross-product form of Cramer's rule.
  New `collision::sphere_vs_corner_fillet` generalizes a
  `StaticQuarterPipe`'s 2-sided sector test to a 3-bound "spherical
  triangle" containment test, each bound a sign-corrected, non-normalized
  cross product of a pair of the three normals. New `arena::
  standard_corner_fillets` builds all 16 (4 per corner wall, times the 4
  corner walls) directly from the same three flat planes `standard_walls`
  already builds, reusing `FILLET_RADIUS` once again. `PhysicsWorld` gains
  `corner_fillets`/`with_corner_fillet`, resolved for the ball and every
  car exactly like `curves`. A car actually being deflected by any fillet
  and goal cutouts (now implemented, see FR-024) remain not modeled.
- `rb_physics_bullet::body`/`collision`/`arena` (`RB-PHYSICS-001-FR-024`) —
  goal cutouts: opens an actual goal-mouth window in each back wall, where
  every prior increment had a single solid, flat plane spanning the full
  width. Introduces a new static shape, `body::StaticGoalWall` (a
  `StaticPlane` plus a rectangular window in the plane's own local
  `u_axis`/`v_axis` frame), with `contains_in_window` testing a point's
  projection onto that frame directly. New
  `collision::sphere_vs_goal_wall`/`contacts_vs_goal_wall`: a sphere (the
  ball) gets no contact inside the window, letting it pass through; a box
  (car) falls straight through to the ordinary `contacts_vs_plane` against
  the wrapped plane, deliberately ignoring the window — a zero-regression
  choice for a car. `arena::standard_walls` drops its 2 back-wall
  `StaticPlane`s (now 7 planes instead of 9); new `arena::
  standard_goal_walls` returns them instead as 2 `StaticGoalWall`s,
  windowed at new constants `GOAL_HALF_WIDTH`/`GOAL_HEIGHT`. New `arena::
  standard_goal_cutout_fillets` rounds each window's 3 edges (two posts
  and a crossbar per goal, 6 `StaticQuarterPipe`s total, added to the same
  `curves` list), built via `StaticQuarterPipe::between_planes` from the
  real back-wall plane and a purely-geometric post/crossbar plane never
  itself added as a real collision wall. `PhysicsWorld` gains
  `goal_walls`/`with_goal_wall`, resolved for the ball *and* every car
  (unlike `curves`/`corner_fillets`). A car actually being deflected by
  any fillet or driving into a goal, and a modeled goal interior/net
  beyond the cutout, remain not modeled.
- `rb_physics_bullet::arena` (`RB-PHYSICS-001-FR-025`) — corner-wall
  floor/ceiling arch radius: a diagonal corner wall's own floor-seam and
  ceiling-seam fillets now use a new, distinctly larger
  `arena::CORNER_ARCH_RADIUS` (750 uu) instead of the cardinal walls' own
  `FILLET_RADIUS` (292 uu), matching real Rocket League's bigger, more
  swept corner-boost curve. All 16 `standard_corner_fillets` switch to
  `CORNER_ARCH_RADIUS` too, since `StaticCornerFillet::between_three_planes`
  needs one shared radius across all three planes it blends to still meet
  its adjoining edge fillets exactly where their axes cross. The 8
  cardinal-wall floor/ceiling seams, 8 vertical corner-edge fillets
  (FR-022), and 6 goal-cutout edge fillets (FR-024) are unaffected and
  keep `FILLET_RADIUS`. A compile-time assertion enforces
  `CORNER_ARCH_RADIUS > FILLET_RADIUS`.
- `rb_physics_bullet::arena` (`RB-PHYSICS-001-FR-026`) — goal
  post-crossbar corner fillets: new `arena::standard_goal_corner_fillets`
  rounds off the two compound corners per goal where a post's own
  vertical edge fillet meets the crossbar's own horizontal edge fillet (4
  total), via `StaticCornerFillet::between_three_planes` on the real back
  wall/post/crossbar planes — the same approach FR-023 used for the
  arena's own compound corners, reusing `FILLET_RADIUS` unchanged since
  both edge fillets meeting here already share one radius. The goal's
  other two corners, where a post meets the floor, get no such treatment
  (the window's bottom edge sits exactly at floor level already).
  `PhysicsWorld::standard_arena` wires the 4 new fillets in, bringing
  `corner_fillets` to 20 total.
- `rb_physics_bullet::collision` (`RB-PHYSICS-001-FR-027`) — car deflection
  by curved fillets: new `collision::box_vs_quarter_pipe`/
  `box_vs_corner_fillet` test a box's own 8 corners against the curved
  surface as zero-radius spheres, the same "test every corner" technique
  `box_vs_plane` already used for a flat plane; `contacts_vs_quarter_pipe`/
  `contacts_vs_corner_fillet` now dispatch a `Shape::Box` to these instead
  of `Vec::new()`. Closes the Non-goal repeated since FR-020: a car is now
  actually deflected by every curved fillet in this port, not just the
  ball. No `PhysicsWorld::step` changes needed — the car-side resolve
  calls already existed, just as silent no-ops. Documented as an
  approximation (a flush box face against a shallow curve can under-detect
  contact), not a full convex-vs-curved-surface narrow phase.
  `StaticGoalWall`/`contacts_vs_goal_wall` is unaffected — a goal wall
  isn't a curved fillet, so a car still can't drive into a goal.
- `rb_physics_bullet::collision` (`RB-PHYSICS-001-FR-028`) — a car can
  actually drive into a goal now: new `collision::box_vs_goal_wall` tests
  each of a box's 8 corners against `StaticGoalWall`'s window (a corner
  inside the window contributes no contact, mirroring
  `sphere_vs_goal_wall`'s pass-through rule per corner instead of once for
  the ball's single center point); `contacts_vs_goal_wall` now dispatches
  a `Shape::Box` to it instead of falling straight through to an
  unwindowed `contacts_vs_plane`. A car only partly lined up with the
  window gets a real partial block — the corners still outside it still
  register a contact. No `PhysicsWorld::step` changes needed, same as
  FR-027 — `resolve_goal_wall_contact` already ran for every car. Goal
  interior/net still not modeled — the goal opens onto open space for a
  car too, not a bounded volume.
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
- `rb_physics_bullet::world`'s
  `a_ball_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall`
  test flew its ball for a fixed 3.0s regardless of when it actually
  cleared the back wall — harmless with the old, smaller `FILLET_RADIUS`
  (the ball drifted into `body::StaticQuarterPipe`'s documented
  infinite-along-its-axis zone far past the goal and got only a mild
  correction there), but `RB-PHYSICS-001-FR-025`'s bigger
  `CORNER_ARCH_RADIUS` moved that same zone closer in and turned the brush
  into a solver-destabilizing correction that threw the ball back past the
  wall, failing the assertion. Shortened the test's flight duration to
  1.8s — still comfortably past the wall, short of the infinite-fillet
  zone.
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
