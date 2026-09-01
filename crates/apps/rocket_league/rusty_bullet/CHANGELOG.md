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
- `rb_physics_bullet::body`/`collision`/`arena` (`RB-PHYSICS-001-FR-029`) —
  a modeled goal interior: new `body::StaticBoundedWall` collides only
  *within* a rectangular bound (the opposite gate from `StaticGoalWall`'s
  window), with new `collision::sphere_vs_bounded_wall`/
  `box_vs_bounded_wall`/`contacts_vs_bounded_wall`. New
  `arena::standard_goal_back_walls` (2 plain, unbounded planes `GOAL_DEPTH`
  behind each real back wall — reachable only through the window, so
  unbounded is exact here), `standard_goal_side_walls` (4 bounded walls,
  reusing `goal_post_plane` unchanged) and `standard_goal_roofs` (2 bounded
  walls, reusing `goal_crossbar_plane` unchanged) close the "ball/car
  passes into open space" gap FR-024 through FR-028 all flagged. New
  `PhysicsWorld.bounded_walls`/`with_bounded_wall`, resolved for the ball
  and every car. Models a solid bounding volume, not a net mesh — no
  cloth/soft-body simulation added.
- `rb_physics_bullet::solver`/`world` (`RB-PHYSICS-001-FR-030`) — a
  combined multi-body solve: new `solver::resolve_dynamic_manifolds`
  resolves every ball-vs-car and car-vs-car contact manifold in a step
  together, sharing one `DeltaVelocity` accumulator per body index across
  every manifold that body takes part in (via new helper `delta_pair_mut`),
  instead of `PhysicsWorld::step` calling `resolve_contacts_between` once
  per pair and fully applying each pair's own result before the next
  pair's setup even read a body's velocity. Fixes the "3+ bodies mutually
  touching in the same step" approximation (e.g. a car pinned between the
  ball and another car) tracked since multi-car support was added. Static
  contacts (ground/walls/curves/goal geometry) are unaffected — each
  body's contact with static geometry never depended on another dynamic
  body, so only the dynamic-vs-dynamic path needed the fix.
- `rb_physics_bullet::drive` (`RB-PHYSICS-001-FR-031`) — a scoped
  constant-calibration audit (does NOT close `FR-005`'s real-data
  calibration, still blocked on `PHASE-0-EXIT`): sourced every
  uncalibrated placeholder constant against RocketSim, RLUtilities, and
  the RLBot community wiki. Corrected `JUMP_SPEED` (→ `875.0/3.0`) and
  `JUMP_HOLD_ACCELERATION` (→ `4375.0/3.0`) to their precise real values;
  added `UNBOOSTED_MAX_CAR_SPEED` (1410) as throttle's own speed cap,
  separate from the boosted `MAX_CAR_SPEED` (2300) — a real fix, since
  throttle alone previously could reach the boosted top speed. Confirmed
  several more constants already correct (`JUMP_HOLD_MAX_DURATION`,
  `BOOST_ACCELERATION`, `MAX_BOOST`, gravity, `GOAL_DEPTH`) and explicitly
  flagged the rest as audited-but-still-uncalibrated in their own doc
  comments, rather than silently leaving them ambiguous.
- `rb_physics_bullet::collision` (`RB-PHYSICS-001-FR-032`) — investigated a
  claimed corner-testing under-detection bug for `box_vs_quarter_pipe`/
  `box_vs_corner_fillet` (a box face resting flush against a shallow curve
  could have every corner clear while the face's middle already
  overlapped it) by building a genuine GJK closest-points replacement.
  Wiring it in broke two real end-to-end tests, because closest-point
  answers the wrong question for this contact — it's a containment
  question (is the box's farthest point from the axis/center at or beyond
  radius), and distance-from-a-line/point's maximum over a convex
  polytope is always at a corner, so the original per-corner technique is
  exact for this question, not an approximation. Reverted to the original
  `RB-PHYSICS-001-FR-027` implementation and deleted the GJK module
  entirely; corrected every doc comment across the crate and its spec
  that had inherited the unverified claim. No production-code change to
  the narrow phase itself.
- `rb_physics_bullet::net` (`RB-PHYSICS-001-FR-033`) — a genuine
  mass-spring net panel per goal, catching the ball. New `net::NetMesh`: a
  rectangular grid of point masses (`RigidBody::sphere`, tiny and light)
  with every perimeter point anchored to the rigid goal frame and every
  interior point free, connected by structural/shear springs (Hooke's law
  plus damping). Ball contact against a free point goes through a new
  `collision::sphere_vs_sphere` (this crate's first real sphere-vs-sphere
  test) plus the existing `solver::resolve_contacts_between` path — no new
  solver code. New `arena::standard_nets` builds one panel per goal,
  `NET_DEPTH` behind the real back wall and well in front of
  `RB-PHYSICS-001-FR-029`'s own rigid back-of-net plane (unchanged, still
  a car's real backstop, since a car isn't tested against the net at all).
  `PhysicsWorld` gains `nets`/`with_net`. Every new constant is an
  uncalibrated placeholder.
- `rb_physics_bullet::solver` (`RB-PHYSICS-001-FR-034`) — split impulse.
  Every contact's normal row now also solves a second, entirely separate
  "push" pseudo-velocity channel (`resolve_push_row`/
  `resolve_two_body_push_row`), fed only by that contact's own positional
  (penetration/ERP) error, never its velocity/restitution error.
  `ConstraintRow`/`TwoBodyRow` gained `rhs_penetration`/
  `applied_push_impulse` fields, splitting the old combined `rhs` term.
  After each manifold's iterations, the real velocity delta is applied to
  the body exactly as before, and the new push delta is applied directly
  to the body's position/orientation via a new `apply_push_delta` (built
  on the existing `integrate::integrate_transform`) — mirroring Bullet's
  own `btSolverBody::writebackVelocity`. Wired into `resolve_contacts`,
  `resolve_contacts_between`, and `resolve_dynamic_manifolds` with zero
  call-site changes anywhere outside `solver.rs`.
- `rb_physics_bullet::solver` (`RB-PHYSICS-001-FR-035`) — warm-starting,
  scoped to `resolve_dynamic_manifolds` only. A new `solver::ContactCache`
  carries a manifold's converged real-channel impulses from one call to
  the next, matched by each contact's approximate world position. A new
  `warm_start_two_body_row` applies each row's cached impulse directly to
  the manifold's shared `DeltaVelocity` accumulators before iterating
  (merely setting `applied_impulse` would do nothing on its own, since
  `GLOBAL_CFM` is always `0.0`). `resolve_dynamic_manifolds` gained a new
  `caches: &mut HashMap<(usize, usize), ContactCache>` parameter, rebuilt
  from only that call's manifolds each time. `PhysicsWorld` gains one
  persistent `dynamic_manifold_caches` field. Deliberately not wired into
  `resolve_contacts`/`resolve_contacts_between` — see that FR's own
  Non-goals.
- `rb_physics_bullet::arena`/`collision`/`net`/`solver`/`world`
  (`RB-PHYSICS-001-FR-036`) — a dedicated follow-up to `FR-031`'s own
  audit, resolving the two ambiguities it surfaced but deliberately didn't
  act on, using real source-level research (RocketSim's and RLUtilities'
  own source, and the current RLBot wiki, read directly). Every `92.75`
  ball-radius literal became `93.15`, not the previously-suspected `91.25`
  — the real games split the ball into a smaller inertia radius (`91.25`)
  and a distinctly larger collision radius (`93.15`, the mesh's own
  collision margin), and since this port's single unified radius field has
  no separate collision margin of its own, the collision radius is the
  correct single-constant analog. `arena::CEILING_Z` changed from `2044.0`
  to `2048.0`, confirmed to share the same reference point as RocketSim's
  `ARENA_HEIGHT`. Also corrected two mis-documented claims:
  `arena::CORNER_LENGTH` and `arena::GOAL_DEPTH` were wrongly described as
  uncalibrated placeholders — both are confirmed exact, so only their doc
  comments changed. `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS` remain
  untouched and still genuinely uncalibrated. No new tests; all 259
  pre-existing tests pass unchanged.
- `rb_physics_bullet::body`/`drive`/`world` (`RB-PHYSICS-001-FR-037`) —
  sleeping, closing the "no sleeping" half of `solver`'s own documented
  gap FR-035 left open. New `body::RigidBody` fields
  `is_sleeping: bool`/`sleep_timer: f32` and methods
  `update_sleep_state(&mut self, dt: f32)` (forcibly zeroes both
  velocities once they've stayed below new
  `LINEAR_SLEEP_VELOCITY_THRESHOLD`/`ANGULAR_SLEEP_VELOCITY_THRESHOLD`
  constants for `SLEEP_TIME_THRESHOLD` seconds, the actual fix for a
  bouncy resting contact never settling) and `wake(&mut self)` (the same
  reset, unconditionally). `PhysicsWorld::step` calls
  `update_sleep_state` for the ball and every car after every other
  contact resolves but before the transform integrates.
  `drive::apply_driven_forces` calls `car.wake()` unconditionally whenever
  a new `input_is_active` helper finds the car's input genuinely active,
  before that input's own force has had a chance to move it. 8 new tests
  (5 in `body.rs`, 3 in `world.rs`); all pre-existing tests pass unchanged.
- `rb_physics_bullet::net`/`world` (`RB-PHYSICS-001-FR-038`) — car-vs-net
  contact, closing this port's own former Non-goal that a car passes
  straight through a `net::NetMesh`'s spatial footprint untouched.
  `net::NetMesh::step` changed from a single `&mut RigidBody` (the ball
  alone) to `&mut [RigidBody]` (every body that can touch the net); its
  inner contact-resolution loop now iterates every body in the slice
  against each free point. No new collision code needed:
  `collision::contacts_between` already dispatches to `sphere_vs_box` for
  a car against a net point the same way it always has for ball-vs-car.
  `PhysicsWorld::step` reuses the same ball-plus-cars snapshot
  `solver::resolve_dynamic_manifolds` already resolved that step for the
  net-step call too. All of `net.rs`'s pre-existing tests updated only
  their call syntax (`std::slice::from_mut(&mut ball)`), not their own
  assertions. 3 new tests (2 in `net.rs`, 1 in `world.rs`); all
  pre-existing tests pass unchanged.
- `rb_physics_bullet::world` (`RB-PHYSICS-001-FR-039`) — wall-jump corner
  disambiguation, closing the "first wall in `self.walls`" simplification
  documented since FR-013 and made reachable in the standard arena by
  FR-019's diagonal corner walls. `PhysicsWorld::step`'s per-car
  wall-normal computation now sums every wall a car is touching this step
  and normalizes the result, instead of picking whichever wall comes
  first — a car touching two walls at a corner now pushes off diagonally,
  blending both, instead of firing along only one of them. A car touching
  exactly one wall is unaffected. No new collision code needed. 1 new
  `world.rs` test; all pre-existing tests pass unchanged.
- `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS` (`RB-PHYSICS-001-FR-040`) —
  a dedicated research pass, matching FR-036's own real-source-research
  method, looked for a real reference for both constants and found only
  one uncited RLBot wiki value ("wall bottom ramp radius: approx. 256, not
  circular") that doesn't distinguish the two constants, disclaims being
  circular, and shares its numeral with RLGym's unrelated `RAMP_HEIGHT`
  (a ramp's height, not a curve's radius) — deliberately not adopted. Both
  constants remain unchanged and genuinely uncalibrated; closing this for
  real needs actual extracted collision-mesh geometry. No new tests
  (documentation-only); all pre-existing tests pass unchanged.
- `solver::resolve_dynamic_manifolds` (`RB-PHYSICS-001-FR-041`) —
  investigated whether anything short of real recorded data could narrow
  FR-030's own documented extreme-mass-ratio "sandwiched"
  under-convergence gap. A naive global over-relaxation factor was tried
  and rejected (provably diverges for that exact case); each manifold's
  velocity-row impulse now scales by a parameter-free `1 / k` instead
  (`k` = the number of manifolds sharing a body this step) — narrowing
  FR-030's own result from ~89.5 to ~32 units/s at zero added iteration
  cost, with zero effect on the overwhelming majority single-manifold
  case. 2 new tests; all pre-existing tests pass unchanged.
- `collision::box_vs_box` (`RB-PHYSICS-001-FR-042`) — validated its
  edge-edge contact point and face-clipping degenerate fallback directly
  against Bullet's own `btBoxBoxDetector::dBoxBox` reference source.
  Confirmed this port's finite-segment edge-edge contact point is more
  rigorous than the reference's own unclamped-infinite-line one, and that
  this port's synthesize-rather-than-drop face-clipping fallback is a
  deliberate, favorable divergence from the reference's own drop-the-
  collision behavior. A candidate fix for the edge-edge tangent
  sign-selection heuristic was built and empirically tested but found
  genuinely mixed against a brute-force ground truth, so not adopted. No
  new tests (documentation-only); all pre-existing tests pass unchanged.
- `solver::combine_restitution`/`combine_friction`
  (`RB-PHYSICS-001-FR-043`) — this project's own spec claimed Bullet's
  default combine mode is `max`, without ever having checked; fetched and
  read `btManifoldResult`'s real source and found that wrong (the real
  default for both is an unclamped product, `a * b`). This port's average
  combine mode is kept, now for a correct reason: it preserves the
  identity `combine(a, a) == a`, which the reference's real product does
  not, and most bodies here currently share the same uncalibrated
  placeholder coefficient. Corrected the wrong claim in the spec,
  `solver.rs`, and `body.rs`. 2 new tests pin the identity-preserving
  behavior directly; all pre-existing tests pass unchanged.
- `docs/specifications/physics/RB-PHYSICS-001-physics-core-port.md`
  (`RB-PHYSICS-001-FR-044`) — this spec's own top-level Non-goals section
  still claimed split impulse wasn't implemented, contradicted by
  `RB-PHYSICS-001-FR-034`'s own already-shipped implementation. Corrected
  the stale bullet to a strikethrough-and-close note, matching the
  convention this section already uses for two other resolved Non-goals
  items. Zero production code changed; no new tests.
- `integrate::integrate_transform` (`RB-PHYSICS-001-FR-045`) — fetched and
  read Bullet's real `btRigidBody.cpp`/`.h`, `btTransformUtil.h`,
  `btQuaternion.h`, and `btScalar.h` and confirmed `apply_damping`,
  `integrate_velocities`, and `integrate_transform`'s reference claims all
  byte-for-byte accurate. Found this port's degenerate-quaternion epsilon
  (`1e-12`) numerically differs from Bullet's own `SIMD_EPSILON` (~5
  orders of magnitude larger) but is behaviorally equivalent, so not
  adopted. Found the check-then-normalize fallback branch matches Bullet's
  real choice to preserve the prior orientation rather than reset to
  identity on a degenerate result — a real distinction an unconditional
  `Quat::normalize` call would have gotten wrong. 1 new test pins this;
  all pre-existing tests pass unchanged.
- `body::Shape::local_inertia`/`RigidBody::update_inertia_tensor`,
  `mat3::Mat3::scaled_columns`/`Mat3::from_quat`
  (`RB-PHYSICS-001-FR-046`) — fetched and read Bullet's real
  `btSphereShape.cpp`, `btBoxShape.cpp`, `btRigidBody.cpp`/`.h`, and
  `btMatrix3x3.h` and confirmed the local-inertia formulas,
  `update_inertia_tensor`, and `scaled_columns` all byte-for-byte
  accurate. Found `Mat3::from_quat` hardcodes `s = 2` assuming a
  unit-length input, while the reference's own `setRotation`
  self-corrects for a non-unit-length one via `s = 2 / q.length2()` — not
  adopted, since this function's only production call site always
  receives an already-renormalized orientation. 1 new test pins this;
  all pre-existing tests pass unchanged.
- `collision::sphere_vs_plane`/`box_vs_plane`/`sphere_vs_box`/
  `sphere_vs_sphere` (`RB-PHYSICS-001-FR-047`) — fetched and read Bullet's
  real `btConvexPlaneCollisionAlgorithm.cpp`/`.h`,
  `btSphereBoxCollisionAlgorithm.cpp`,
  `btSphereSphereCollisionAlgorithm.cpp`, and `btManifoldPoint.h` and
  confirmed `sphere_vs_plane`/`sphere_vs_sphere` exact, and
  `sphere_vs_box`'s deep-penetration face selection confirmed to
  reproduce Bullet's own exact `+x, -x, +y, -y, +z, -z` face-check
  tie-break order. Found `box_vs_plane` computes all 4 corners exactly in
  one pass, where real Bullet's default configuration produces only one
  contact point per frame via a single GJK support query, relying on
  several frames of persistent-manifold accumulation to reach the same
  4-corner manifold — a favorable divergence, not adopted, in the same
  spirit as `box_vs_box`'s own FR-042 finding. 1 new test pins the exact
  tie-break-order match; all pre-existing tests pass unchanged.
- `solver::restitution_curve`/`plane_space`/`setup_rows`/`resolve_row`
  (`RB-PHYSICS-001-FR-048`) — fetched and read Bullet's real
  `btSequentialImpulseConstraintSolver.cpp`/`.h`, `btContactSolverInfo.h`,
  and `btVector3.h` and confirmed `plane_space` byte-for-byte exact,
  `restitution_curve` behaviorally exact (its `.max(0.0)` folds in a
  clamp real Bullet applies at its own call site instead), `setup_rows`
  exact against real `setupContactConstraint`/`setupFrictionConstraint`
  (correcting a stale citation to an unrelated function), `resolve_row`'s
  unified two-bound resolver behaviorally equivalent to Bullet's own two
  separate resolvers, and all 6 of `btContactSolverInfo`'s cited defaults
  exact. Found one genuine, significant divergence, not adopted: this
  port always derives both friction directions from a fixed,
  velocity-independent basis, where real Bullet's actual default aligns
  one direction with the tangential component of the current relative
  sliding velocity — flagged as open follow-up work for a dedicated
  future FR rather than fixed here. 1 new test pins the
  `restitution_curve`/call-site-clamp equivalence; all pre-existing tests
  pass unchanged.
- `arena::GOAL_HALF_WIDTH`/`GOAL_HEIGHT` reference confirmation
  (`RB-PHYSICS-001-FR-055`) — fetched the current RLBot wiki's "Useful
  Game Values" page directly (the same page `RB-PHYSICS-001-FR-036`'s own
  research used for `GOAL_DEPTH`) and confirmed both values exact against
  its own cited "Goal center-to-post"/"Goal height" numbers — no value
  change, a sourcing-status upgrade from "commonly-cited, unconfirmed" to
  "confirmed", the same non-behavioral outcome FR-036 reached for
  `GOAL_DEPTH`/`CORNER_LENGTH`. Also found and fixed a stale spec passage
  still describing `GOAL_DEPTH` as an unconfirmed "uncalibrated
  invention", contradicting FR-036's own already-shipped Requirements
  entry. No new tests; all pre-existing tests pass unchanged.
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
- `rb_physics_bullet::solver`'s friction-direction selection
  (`RB-PHYSICS-001-FR-049`) — closes the divergence
  `RB-PHYSICS-001-FR-048` found and left open: both friction directions
  were always derived from a fixed, velocity-independent `plane_space`
  basis, where real Bullet's actual default aligns friction direction 1
  with the tangential component of the current relative sliding velocity.
  A new `friction_directions` helper now does the latter, falling back to
  `plane_space` both for negligible tangential velocity (matching real
  Bullet's own `SIMD_EPSILON` threshold) and for a newly-found near-head-on
  catastrophic-cancellation edge case this crate's own panic-free
  `Vec3::normalize()` needed to handle but real Bullet's unguarded one
  doesn't. Wired into both `setup_rows` and `setup_two_body_rows`. 3 new
  tests, including a dedicated isotropic-friction regression test verified
  to fail under the old fixed-basis behavior; all pre-existing tests pass
  unchanged.
- `net::NetMesh::step`'s body-vs-net-point contact resolution
  (`RB-PHYSICS-001-FR-050`) — resolved every overlapping point
  independently and sequentially, an untested "net-point mass is tiny
  enough to not matter" assumption found false and, worse, genuinely
  order-dependent for a symmetric double-point impact (confirmed by a
  dedicated test), with a measured real-world residual of ~0.25 units/s
  out of a 2000 units/s impact. Adopted `solver::resolve_dynamic_manifolds`'s
  combined solve for every body-vs-point contact within a sub-step,
  reducing that residual roughly 15-fold to ~0.016 units/s; warm-starting
  deliberately left out of scope. 2 new tests; all pre-existing tests pass
  unchanged.
- `PhysicsWorld::step`'s static multi-surface contact resolution
  (`RB-PHYSICS-001-FR-051`) — resolved a body's contact against each
  static shape type (ground, wall, curve, corner fillet, goal wall,
  bounded wall) independently and sequentially, the same
  independent-pairwise gap `RB-PHYSICS-001-FR-030`/`RB-PHYSICS-001-FR-050`
  already proved under-converges. A dedicated test confirmed a ball wedged
  into a symmetric two-wall corner is genuinely order-dependent
  (mirror-image results depending on which wall resolves first). A new
  `solver::resolve_static_manifolds` generalizes `resolve_contacts` to
  combine every static-shape manifold a body touches into one shared
  solve; `step` was rewired to use it via a new `resolve_static_contacts`,
  replacing the old five-function-per-body call sequence
  (`resolve_plane_contact`/`resolve_curve_contact`/
  `resolve_corner_fillet_contact`/`resolve_goal_wall_contact`/
  `resolve_bounded_wall_contact`, all removed). 2 new tests, one confirmed
  to fail under the old sequential loop; all pre-existing tests pass
  unchanged.
- `PhysicsWorld::step`'s static-vs-dynamic combined-solve ordering
  (`RB-PHYSICS-001-FR-052`) — resolved a body's now-combined static
  contacts and its combined dynamic manifolds as two separate solves, one
  fully resolved and applied before the other's own setup for that same
  body ever read the result, the same independent-pairwise gap
  `RB-PHYSICS-001-FR-030`/`RB-PHYSICS-001-FR-050`/`RB-PHYSICS-001-FR-051`
  already proved under-converges. A dedicated test reused FR-051's own
  symmetric two-wall corner setup with one wall replaced by a very-heavy
  dynamic body, confirming the old two-call order is genuinely
  order-dependent. A new `solver::resolve_manifolds` folds a step's static
  and dynamic manifolds into one shared solve; `step` was rewired to use
  it, replacing the two separate calls with one. 2 new tests, one
  confirmed to fail under the old two-call sequence; all pre-existing
  tests pass unchanged.
- `solver::combine_friction`'s missing defensive clamp
  (`RB-PHYSICS-001-FR-053`) — `RB-PHYSICS-001-FR-043` fetched and read
  real Bullet's own `btManifoldResult::calculateCombinedFriction`/
  `calculateCombinedRestitution` source to correct this spec's wrong
  claim about the reference's default combine mode, but never separately
  examined one more detail in that same source: real Bullet's own
  `calculateCombinedFriction` additionally clamps its product result to
  `[-10.0, 10.0]`. Re-fetched and re-read `btManifoldResult.cpp` directly
  to confirm the clamp's exact mechanics, found it currently inert for
  every friction coefficient this crate itself ever sets (all positive
  placeholders in `0.1..=0.9`), and adopted it anyway for reference
  conformance since every static/dynamic body's own `friction` field is a
  public, unvalidated `f32`. `combine_friction` now clamps its average
  result to `[-10.0, 10.0]`, keeping the average formula FR-043 already
  decided to keep; `combine_restitution` stays unclamped, matching the
  reference's own choice. 1 new test; all pre-existing tests pass
  unchanged.
- `collision::box_vs_goal_wall`'s corner-testing overlap question
  (`RB-PHYSICS-001-FR-054`) — `RB-PHYSICS-001-FR-028`'s own doc comment
  left open whether a car's face resting flush against the goal window's
  own edge, every corner just clear of it while the face's middle already
  overlapped it, could be under-detected the way `RB-PHYSICS-001-FR-032`
  once suspected for a curved fillet. Resolved via a convex-hull
  argument: "every corner outside the (convex) window" is exactly
  equivalent to "the face doesn't fully fit through it," the correct
  block condition — no bug. The same investigation found the mirror image
  for `collision::box_vs_bounded_wall` *is* a genuine, currently
  unreachable under-detection gap (confirmed against this project's own
  car/ball sizes vs. the standard arena's own bound sizes) and documented
  it as a Non-goals item rather than fixing it. 2 new tests; all
  pre-existing tests pass unchanged.
- `drive::BOOST_ACCELERATION`'s missing ground/air split
  (`RB-PHYSICS-001-FR-056`) — this port's own single flat boost
  acceleration constant, and its own doc comments' explicit claim that
  boost "works identically airborne", were both found wrong by fetching
  RocketSim's own `RLConst.h` directly: the reference defines a
  distinctly higher `BOOST_ACCEL_AIR` (`3175/3` ≈ 1058.333) than
  `BOOST_ACCEL_GROUND` (`2975/3` ≈ 991.667, exactly matching this port's
  prior value) — a genuine split this port didn't model, understating
  every airborne boost by about 6.5%. Split into
  `BOOST_ACCELERATION_GROUND`/`BOOST_ACCELERATION_AIR`, wired
  `apply_driven_forces`'s existing `on_ground` parameter to select
  between them, and corrected every doc comment claiming the two were
  identical. 1 new test; all pre-existing tests pass unchanged.
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
