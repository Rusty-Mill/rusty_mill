# Project Status

- Last verified main commit: `2f5a3eb` (merge of [#157](https://github.com/baileyrd/rusty_bullet/pull/157))
- Verified at: 2026-09-01
- Current milestone: `PHASE-1-PHYSICS-CORE` (box-shaped car bodies, general 3x3 inertia, multi-contact resolution, ball-vs-car collision, car-vs-car collision, body-vs-arena-wall collision, ground-driving car input (throttle/steering), boost, handbrake, a variable-height ground jump, air control, a double jump (plain or a directional, flip-cancelable dodge), a wall jump (itself dodgeable and flip-cancelable the same way), a gentle landing auto-orientation assist, a modeled octagonal arena footprint plus ceiling (`PhysicsWorld::standard_arena`), curved fillets throughout the arena's vertical boundary deflecting the ball and, since FR-027, a car too (via corner-testing, confirmed exact for this containment-style contact by FR-032) — floor/ceiling seams for all 9 walls (cardinal and diagonal corner, the 4 corner walls' own seams distinctly larger than the cardinal walls' since FR-025), all 8 of the corner walls' own vertical edges (FR-022), and all 16 compound corners where a vertical-edge fillet meets a floor- or ceiling-seam fillet (FR-023, sized to match FR-025's bigger corner-wall arches) — and, since FR-024, an actual goal-mouth window (with its own 3 rounded edges) cut into each back wall, with a car now able to drive through it too since FR-028 (via the same per-corner approximation technique), since FR-026, the 4 compound corners per goal where a post's own fillet meets the crossbar's, and, since FR-029, a modeled bounded interior behind each goal window (a solid box) so a ball or car passing through settles instead of flying forever, and, since FR-033, a real mass-spring net panel per goal catching the ball specifically (scoped to the ball only at the time — since FR-038, a car is caught too), and, since FR-030, every ball-vs-car/car-vs-car contact manifold in a step is resolved together as one combined multi-body solve instead of independent pairwise ones, and, since FR-031, uncalibrated placeholder constants have been individually audited against the community reverse-engineering effort (some corrected, some confirmed, the rest explicitly flagged), and, since FR-032, the once-suspected corner-testing under-detection gap for a car vs. a curved fillet was rigorously investigated and found not to exist (a genuine GJK-based replacement was built, found to regress two real tests, and reverted — the honest outcome is a corrected doc comment, not new production code), and, since FR-034, every contact's penetration/positional correction runs on its own separate split-impulse "push" channel instead of folding into the body's real velocity, so resolving deep overlap no longer injects spurious velocity, and, since FR-035, `solver::resolve_dynamic_manifolds` (every ball-vs-car/car-vs-car manifold) warm-starts each call from the previous one's converged impulses instead of zero, converging measurably closer to the true answer for an under-converged manifold, and, since FR-036, the ball's collision radius (`92.75` to `93.15`) and `arena::CEILING_Z` (`2044.0` to `2048.0`) were corrected via real source-level research rather than left as open ambiguities, and, since FR-037, sleeping forcibly zeroes a body's velocity once it's stayed below a linear and an angular threshold for a sustained time, fixing the "bouncy resting contact never settles" limitation neither split impulse nor warm-starting alone could, and, since FR-038, `net::NetMesh::step` catches every car too, not just the ball, closing this port's own former Non-goal, and, since FR-039, a wall jump at a corner (a car touching two walls at once) pushes off along every touched wall's normal summed and normalized instead of picking whichever wall came first, and, since FR-040, a dedicated research pass looked for a real reference for `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS` and found only one uncited, self-disclaimed-non-circular, likely-conflated wiki value, deliberately not adopted (both constants remain genuinely uncalibrated), and, since FR-041, `solver::resolve_dynamic_manifolds` scales each manifold's velocity-row impulse by a parameter-free `1 / k` for a body shared by `k >= 2` manifolds this step, narrowing FR-030's own documented "sandwiched" under-convergence gap (a naive global over-relaxation factor was investigated and rejected, since it provably diverges for that exact case), and, since FR-042, `box_vs_box`'s edge-edge contact point and face-clipping degenerate fallback were validated directly against `btBoxBoxDetector::dBoxBox`'s own real source — this port's finite-segment edge-edge point derivation confirmed more rigorous than the reference's own unclamped-infinite-line one, its synthesize-rather-than-drop fallback confirmed a deliberate favorable divergence, and a candidate fix for the edge-edge sign-selection heuristic built and empirically tested but found genuinely mixed, not adopted, and, since FR-043, this spec's own claim that Bullet's default restitution/friction combine mode is `max` was checked directly against real source and found wrong (the real default is an unclamped product), with this port's own average combine mode re-justified for a correct reason (it preserves the identity `combine(a, a) == a`, which the reference's product doesn't), and, since FR-044, a stale "split impulse isn't implemented" Non-goals bullet (contradicted by FR-034's own already-shipped implementation) was corrected, and, since FR-045, `integrate.rs`'s own Bullet-reference claims were checked directly against real fetched source and confirmed accurate, with one finding worth keeping — its degenerate-quaternion fallback deliberately preserves the prior orientation rather than resetting to identity, matching Bullet's own real choice, and, since FR-046, `body.rs`/`mat3.rs`'s own Bullet-reference claims were likewise checked directly and confirmed accurate, with one similar finding worth keeping — `Mat3::from_quat` doesn't self-correct a non-unit-length input the way Bullet's own version does, safe only because its single call site always receives an already-renormalized orientation, and, since FR-047, `collision.rs`'s remaining closed-form shape pairings (`sphere_vs_plane`, `box_vs_plane`, `sphere_vs_box`, `sphere_vs_sphere`) were likewise checked directly against real fetched source — `sphere_vs_plane` and `sphere_vs_sphere` confirmed exact, `sphere_vs_box`'s deep-penetration face selection confirmed to reproduce Bullet's own exact tie-break check order, and `box_vs_plane` confirmed a deliberate, more rigorous divergence from Bullet's own real single-contact-per-frame-plus-persistence default, in the same spirit as FR-042's `box_vs_box` finding, and, since FR-048, `solver.rs`'s own `restitution_curve`/`plane_space`/`setup_rows`/`resolve_row` and `btContactSolverInfo`'s cited defaults were likewise checked directly and confirmed exact or confirmed-equivalent restructurings, with one genuine, significant finding kept open rather than fixed — this port always derives both friction directions from a fixed, velocity-independent basis, while Bullet's own real default aligns one direction with the actual relative sliding velocity, a physically meaningful difference deliberately left for a dedicated future FR, and, since FR-049, that divergence was closed: a new `friction_directions` helper in `solver.rs` aligns friction direction 1 with the tangential component of relative sliding velocity, matching Bullet's own real default, falling back to `plane_space` both for negligible tangential velocity and for a newly-found near-head-on catastrophic-cancellation edge case this crate's own panic-free `Vec3::normalize()` needed to handle, and, since FR-050, `net::NetMesh::step`'s own independent-pairwise body-vs-net-point contact resolution was found genuinely order-dependent for a symmetric double-point impact (not merely slow to converge), closed by adopting `solver::resolve_dynamic_manifolds`'s combined solve there too, reducing the measured real-world residual bias roughly 15-fold, and, since FR-051, the same independent-pairwise gap was found and closed one level up — `PhysicsWorld::step`'s own per-static-shape-type sequential resolution (ground, then each wall, curve, corner fillet, goal wall, bounded wall) was confirmed genuinely order-dependent for a body wedged into a symmetric two-wall corner, closed by a new `solver::resolve_static_manifolds` combining every static-shape manifold a body touches into one shared solve, replacing the old five-function-per-body call sequence, and, since FR-052, the same independent-pairwise gap was found and closed one level higher still — `PhysicsWorld::step` resolved a body's now-combined static contacts and its combined dynamic manifolds as two separate solves, confirmed genuinely order-dependent by reusing FR-051's own two-wall corner setup with one wall replaced by a very-heavy dynamic body, closed by a new `solver::resolve_manifolds` folding a step's static and dynamic manifolds into one shared solve, and, since FR-053, `solver::combine_friction`'s own result now clamps to `[-10.0, 10.0]` matching real Bullet's own `calculateCombinedFriction` (a detail FR-043's own reference read surfaced but never separately examined), currently inert for every friction coefficient this crate itself sets but adopted for reference conformance against every static/dynamic body's own unvalidated public `friction` field, and, since FR-054, the one question FR-028's own doc comment left open about `box_vs_goal_wall`'s corner testing was resolved (a convex-hull argument shows it collides exactly like an unwindowed plane when a face is bigger than the window and centered on it), while the same investigation found and documented, rather than fixed, a genuine mirror-image under-detection gap in `box_vs_bounded_wall` — confirmed unreachable given this project's own car/ball sizes against the standard arena's own bound sizes, and, since FR-055, `arena::GOAL_HALF_WIDTH`/`GOAL_HEIGHT` were confirmed exact against the current RLBot wiki's own cited goal dimensions (a sourcing-status upgrade, no value change), while a stale spec passage still describing `GOAL_DEPTH` as unconfirmed (contradicting FR-036's own already-shipped confirmation) was corrected, and, since FR-056, this port's own single flat boost-acceleration constant was found to be genuinely wrong (RocketSim's own real source defines a distinctly higher airborne value than grounded, which this port had collapsed into one) and fixed — a real behavioral change, not just a doc correction, closing this project's own earlier mistaken claim that boost "works identically airborne", and, since FR-057, a genuine missing constraint was found and fixed the same way — nothing previously bounded how fast sustained air control torque could spin a car, and RocketSim's own real `CAR_MAX_ANG_SPEED` hard limit (5.5 rad/s) is now enforced via a new `drive::clamp_angular_speed`, applied once per step right after velocity integration in both the production path and this crate's own test helper, and, since FR-058, throttle acceleration is scaled by a confirmed real speed-dependent taper curve (`drive::drive_speed_taper`) instead of applying flat right up to a hard cutoff, closing a gap `THROTTLE_ACCELERATION`'s own doc comment had named since it was introduced, and, since FR-059, a backward or side dodge now scales up (to 2.5x/1.9x `DODGE_SPEED`) as current speed rises, matching RocketSim's own confirmed real ratios (`drive::dodge_speed_scale`/`dodge_pitch_is_backward`) instead of a flat magnitude regardless of direction or speed, and, since FR-060, a stale open question about whether real auto-flip could map onto the landing auto-orientation assist was resolved by reading RocketSim's own real source (it can't — real auto-flip/auto-roll are both grounded and input-gated, unlike this port's airborne, input-free nudge), a documentation-only finding with no behavioral change, and, since FR-061, the ball now has hard caps on its own linear and angular speed (`world::BALL_MAX_SPEED = 6000.0`, `world::BALL_MAX_ANG_SPEED = 6.0`, enforced by a new `world::clamp_ball_velocity`), matching RocketSim's own confirmed real hard caps and closing a gap where the ball previously had no speed limit of any kind, unlike the car, and, since FR-062, a new `body::RigidBody::ball` constructor sets the ball's own confirmed real material properties (`restitution = 0.6`, `friction = 0.35`, `linear_damping = 0.03`) instead of the generic `sphere` placeholder, resolving the exact gap FR-061 had deferred (no dedicated ball-construction API to adopt `BALL_DRAG` against), and, since FR-063, a stale open question about which restitution/friction combine formula matches real Rocket League was resolved by reading RocketSim's own real source (real Rocket League doesn't use any generic per-body combine at all for its own named contact-pair types — it hardcodes distinct overrides per pair, including a car-vs-ball restitution of exactly zero and a car-vs-ball friction above 1.0), a documentation-only finding with no behavioral change, since this port's own `combine_restitution`/`combine_friction` architecture has no way to represent a per-pair-type override without a larger, separate change, and, since FR-064, a ground jump's variable-height hold acceleration now models real Rocket League's own mandatory `JUMP_MIN_TIME` (0.025s) minimum-hold window — a genuine behavioral fix, not just a doc correction — during which the acceleration keeps applying (scaled by `JUMP_PRE_MIN_ACCEL_SCALE = 0.62`) regardless of whether `jump` is still held, so even an instantaneous tap now gains a small amount of extra height, closing a gap `drive::JUMP_HOLD_MAX_DURATION`'s own doc comment had named since FR-031's original audit, and, since FR-065, real Rocket League's steering was found to be a wheeled-vehicle raycast/tire-slip model (`btVehicleRL`), not a torque at all — an architecture this port's own single-rigid-box car can't represent, the same category FR-063 established — with the confirmed real steer-angle-vs-speed curve's own shape also found to be the opposite of this port's own `speed_factor` (real turning ability is highest at a standstill, decreasing with speed; this port's own scales up with speed from zero), a documentation-only finding with no behavioral change since the real curve's wheel-angle-to-torque translation depends on tire-slip friction this port doesn't model, and, since FR-066, real Rocket League's own handbrake friction reduction was found to be genuinely anisotropic — two separate confirmed real curves, a constant `0.1` lateral factor and a near-constant `~0.9` longitudinal factor, applied to tire friction independently — where this port applies one shared `drive::HANDBRAKE_FRICTION_MULTIPLIER` to both directions alike; a striking coincidence (this port's own `0.1` exactly matches the real lateral-only factor) is not a confirmation, since the same `0.1` also wrongly crushes longitudinal grip real Rocket League keeps near `0.9`, a documentation-only finding with no behavioral change since genuinely splitting friction by direction would require threading a second, direction-specific coefficient through every one of `solver.rs`'s row-limit call sites, and, since FR-067, real Rocket League was found to have no distinct wall-jump mechanic or constant at all — `drive::WALL_JUMP_HORIZONTAL_SPEED`'s own "no public reference" gap turned out to be because real Rocket League's wall jump is the identical single grounded-jump impulse applied along the car's own up axis (which tips to match a touched wall via the wheel/suspension system this port's box car doesn't have), not a distinct horizontal-plus-vertical composite, a documentation-only finding with no behavioral change since this port's own two-component substitute remains a necessary compensation for that missing orientation mechanism, and, since FR-068, air control's yaw and roll torques now scale from pitch's own by real confirmed per-axis ratios (`AIR_CONTROL_YAW_SCALE = 95/130`, `AIR_CONTROL_ROLL_SCALE = 400/130`, from RocketSim's own `CAR_AIR_CONTROL_TORQUE = Vec(130, 95, 400)`) instead of all three axes sharing one flat magnitude — a genuine behavioral fix, not just a doc correction, since real air control's own mechanism was confirmed structurally identical to this port's own direct per-axis torque model, unlike steering or handbrake's own architecture mismatches, and, since FR-069, a flip's real spin was found to be a continuous per-axis torque applied every tick for a fixed 0.65s window (not an instantaneous angular-velocity kick), a documentation-only finding with no behavioral change since real Rocket League's own resulting spin rate depends on its own specific hitbox inertia tensor this port's placeholder car body doesn't match, and reproducing the real timed-torque shape would need new per-car elapsed-flip-time state FR-059's own Non-goals already flagged as out of scope, and, since FR-070, this port's own flip-cancel doc comment's "matching real Rocket League" claim was checked against real source and found inaccurate — real Rocket League's flip-cancel is driven by continuously holding pitch in the same direction as the flip's own pitch-torque component, scaling only that pitch-axis component proportionally, not this port's own jump-press-triggered, all-axis, outright zero, a documentation-only finding with no behavioral change since this port's dodge has no per-axis torque split to partially cancel and reproducing the real trigger/scope would need the same elapsed-flip-time state FR-059's own Non-goals already flagged as out of scope, and, since FR-071, real air control's own per-axis angular-velocity damping mechanism (subtracting `(angular velocity along an axis) * CAR_AIR_CONTROL_DAMPING[axis] * (1 - abs(analog input on that axis))` from the applied torque, releasing the stick giving full damping and holding it fully zeroing it) was found and documented, a thread FR-068's own Non-goals had left open without examining the mechanism, a documentation-only finding with no behavioral change since introducing this damping is a genuinely new mechanism this port has no existing quantity to scale a ratio from, unlike air control's own torque ratio, and, since FR-072, a diagonal dodge (both pitch and roll held) no longer comes out faster than an axis-aligned one — a genuine behavioral fix, not just a doc correction, adopting real Rocket League's own confirmed direction-normalization mechanism (`dodgeDir.safeNormalized()`) via a new `drive::normalize_dodge_direction`, closing a gap FR-059's own Non-goals had already flagged, and, since FR-073, a yaw-only stick press now fires a sideways dodge the same as a roll-only one would — a genuine behavioral fix folding real Rocket League's own confirmed `dodgeDir.y = yaw + roll` into the same combined roll-axis stick value, closing another gap FR-059's own Non-goals had already flagged, and, since FR-074, a near-axis-aligned diagonal dodge now snaps to a clean single-axis dodge instead of leaving a tiny perpendicular component — a genuine behavioral fix adopting RocketSim's own confirmed post-normalization small-component zeroing, correcting FR-073's own mis-scoping of this as "a separate, independent simplification", and, since FR-075, `DODGE_DEADZONE` is confirmed exact against RocketSim's own real dodge-cancellation threshold rather than "no public reference at all" as this spec previously claimed — an audit finding with no behavioral change, since this port's own trigger already matched the real one exactly once FR-073's yaw fold-in was in place — all implemented in `rb_physics_bullet` and wired into a real multi-car `PhysicsWorld`, and, since FR-076, `rb_physics_bullet` can now seed a `PhysicsWorld` from a recorded capture frame and simulate it forward using that capture's own recorded per-tick input (`PhysicsWorld::from_frame`/`world::simulate_recorded`), the prerequisite plumbing real-data constant calibration (FR-005) needs — surfacing along the way a real, previously-unnoticed ~44% width discrepancy in this crate's own long-standing car hitbox test placeholder against RocketSim's own real Octane dimensions, deliberately left uncorrected pending a dedicated future calibration FR, and, since FR-077, that capability is now wired into `rb_verify_cli` too (`score_capture_against_candidate`, `rb-verify --self`), scoring a real capture's own recorded outcome against a candidate simulated from its own recorded input from the first grounded, neutral frame onward, and the owner's own real-capture run produced this project's first genuine fidelity number (mean car position/rotation/velocity distance 4508.71 uu / 2.12 rad / 1421.73 uu/s over 2818 frames) — a very large divergence consistent with near-total trajectory decorrelation, not yet the right shape of evidence to calibrate FR-005's own constants from, so FR-005 itself still hasn't started, and, since FR-078, every existing test in the crate that models a real car (not an arbitrary shape used purely to test collision algorithms) now builds it from the confirmed real `CAR_HALF_EXTENTS` instead of the old, narrower placeholder FR-076 introduced but left untouched everywhere else — a test-suite hygiene correction, not new real-data-driven physics) — In Progress
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
  always populated, tested against a synthetic hand-authored fixture (now
  also verified against a real BakkesMod capture — see the
  `RB-VERIFY-002-FR-001` entry below).
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
- `RB-PHYSICS-001-FR-049` (velocity-aligned friction direction selection,
  implemented) — closes the genuine, significant divergence FR-048 found
  and left open: a new `friction_directions` helper in `solver.rs` now
  aligns friction direction 1 with the tangential component of the
  current relative sliding velocity, matching real Bullet's actual
  default, with direction 2 completing a right-handed basis via
  `dir1.cross(normal)`. Falls back to `plane_space`'s fixed basis both
  for negligible tangential velocity (matching real Bullet's own
  `SIMD_EPSILON` threshold) and for a second, genuinely new case found
  while implementing this: near-head-on collisions where catastrophic
  floating-point cancellation can leave a degenerate tangential residual
  that real Bullet's own unguarded `normalize()` would silently mishandle
  but this crate's own `Option`-returning `Vec3::normalize()` instead
  falls back gracefully from. Wired into both `setup_rows` and
  `setup_two_body_rows`. Confirmed via a dedicated isotropic-friction
  regression test, verified to fail under the old fixed-basis behavior.
  3 new tests; all 279 pre-existing tests pass unchanged. 282 total in
  `rb_physics_bullet` (+3 over FR-048's 279).
- `RB-PHYSICS-001-FR-050` (net-point contact combined-solve investigation,
  implemented) — `net::NetMesh::step` resolved every body-vs-net-point
  contact independently and sequentially, waving off `FR-030`'s own
  independent-pairwise gap as irrelevant because a net point's mass is
  "tiny enough" — an untested claim found false (`NET_POINT_MASS = 0.5` is
  half a typical ball's own mass). A dedicated single-shot test confirmed
  the old sequential loop is genuinely order-dependent for a symmetric
  double-point impact; a `NetMesh::step`-level test measured the real
  residual at ~0.25 units/s out of a 2000 units/s impact. Adopted
  `solver::resolve_dynamic_manifolds`'s combined solve for every
  body-vs-point contact within a sub-step, reducing that residual roughly
  15-fold to ~0.016 units/s; warm-starting deliberately left out of scope.
  2 new tests; all 282 pre-existing tests pass unchanged. 284 total in
  `rb_physics_bullet` (+2 over FR-049's 282).
- `RB-PHYSICS-001-FR-051` (static multi-surface contact combined-solve
  investigation, implemented) — `PhysicsWorld::step` resolved a body's
  contact against each static shape type (ground, then every wall, curve,
  corner fillet, goal wall, bounded wall) independently and sequentially,
  the same independent-pairwise gap FR-030/FR-050 already proved
  under-converges. A dedicated single-shot test confirmed a ball wedged
  into a symmetric two-wall corner is genuinely order-dependent
  (mirror-image results depending on which wall resolves first). A new
  `solver::resolve_static_manifolds` generalizes `resolve_contacts` to
  combine every static-shape manifold a body touches into one shared
  solve; `PhysicsWorld::step` was rewired to use it via a new
  `resolve_static_contacts`, replacing the old five-function-per-body call
  sequence. A `PhysicsWorld::step`-level test (a ball fired into a real
  two-wall corner) confirmed the fix at the public API, verified to fail
  under the old sequential loop first. 2 new tests; all 284 pre-existing
  tests pass unchanged. 286 total in `rb_physics_bullet` (+2 over FR-050's
  284).
- `RB-PHYSICS-001-FR-052` (static-vs-dynamic combined-solve ordering
  investigation, implemented) — `PhysicsWorld::step` resolved a body's
  now-combined static contacts (FR-051) and its combined dynamic
  manifolds (FR-030) as two separate solves, one fully resolved and
  applied before the other's own setup for that same body ever read the
  result — the same independent-pairwise gap FR-030/FR-050/FR-051 already
  proved under-converges, just at the boundary between the two existing
  combined solves. A dedicated single-shot test reused FR-051's own
  symmetric two-wall corner setup, replacing one wall with a very-heavy
  dynamic body (routed through the dynamic-manifold code path instead of
  the static one), and confirmed the old order is genuinely
  order-dependent. A new `solver::resolve_manifolds` folds a step's static
  and dynamic manifolds into one shared solve; `PhysicsWorld::step` was
  rewired to use it, replacing the two separate calls with one. A
  `PhysicsWorld::step`-level test (a ball fired into a real
  wall-and-heavy-car corner) confirmed the fix at the public API, verified
  to fail under the old two-call sequence first. 2 new tests; all 286
  pre-existing tests pass unchanged. 288 total in `rb_physics_bullet` (+2
  over FR-051's 286).
- `RB-PHYSICS-001-FR-053` (`combine_friction` defensive clamp, implemented)
  — `RB-PHYSICS-001-FR-043` fetched and read real Bullet's own
  `btManifoldResult::calculateCombinedFriction`/`calculateCombinedRestitution`
  source to correct this spec's wrong claim about the reference's default
  combine mode, but never separately examined one more detail in that same
  source: real Bullet's own `calculateCombinedFriction` additionally
  clamps its product result to `[-10.0, 10.0]`. Re-fetched and re-read
  `btManifoldResult.cpp` directly to confirm the clamp's exact mechanics,
  found it currently inert for every friction coefficient this crate
  itself ever sets (all positive placeholders in `0.1..=0.9`), and adopted
  it anyway for reference conformance — every static/dynamic body's own
  `friction` field is a public, unvalidated `f32`. `combine_friction` now
  clamps its average result to `[-10.0, 10.0]`, keeping the average
  formula FR-043 already decided to keep; `combine_restitution` stays
  unclamped, matching the reference's own choice. 1 new test; all 288
  pre-existing tests pass unchanged. 289 total in `rb_physics_bullet` (+1
  over FR-052's 288).
- `RB-PHYSICS-001-FR-054` (goal-wall/bounded-wall corner-testing overlap
  investigation, implemented) — closed the one question `RB-PHYSICS-001-FR-028`'s
  own doc comment left open: whether `box_vs_goal_wall`'s per-corner
  window test could under-detect a car's face resting flush against the
  window's own edge, every corner just clear of it while the face's
  middle already overlapped it — the same category of concern
  `RB-PHYSICS-001-FR-032` investigated for a curved fillet but explicitly
  didn't cover for a flat rectangle. Resolved via a convex-hull argument:
  "every corner outside the (convex) window" is exactly equivalent to
  "the face doesn't fully fit through it," the correct block condition —
  no bug, matching FR-032's own precedent via a distinct argument.
  Investigating `box_vs_bounded_wall` (`RB-PHYSICS-001-FR-029`) alongside
  it, since it shares the identical corner-testing technique with the
  opposite gate, found the mirror image is a genuine gap: a face larger
  than a bound and centered on it reports zero contacts despite genuinely
  resting on real material — confirmed unreachable given this project's
  own car/ball sizes against the standard arena's own bound sizes, so
  documented as a Non-goals item rather than fixed. 2 new tests; all 289
  pre-existing tests pass unchanged. 291 total in `rb_physics_bullet` (+2
  over FR-053's 289).
- `RB-PHYSICS-001-FR-055` (`GOAL_HALF_WIDTH`/`GOAL_HEIGHT` reference
  confirmation, stale doc correction, implemented) — fetched the current
  RLBot wiki's "Useful Game Values" page directly (the same page FR-036's
  own research used for `GOAL_DEPTH`) and confirmed `arena::GOAL_HALF_WIDTH`/
  `GOAL_HEIGHT` exact against its own cited "Goal center-to-post"/"Goal
  height" numbers — no value change, a sourcing-status upgrade only.
  Also found and fixed a stale "Open questions" passage that still
  described `GOAL_DEPTH` as an unconfirmed "uncalibrated invention",
  contradicting FR-036's own already-shipped Requirements entry and this
  spec's own Non-goals section. No new tests (pure constant-sourcing/doc
  correction, no behavioral change); all 291 pre-existing tests pass
  unchanged.
- `RB-PHYSICS-001-FR-056` (boost acceleration ground/air split,
  implemented) — `drive::BOOST_ACCELERATION` was a single flat constant
  applied identically grounded or airborne, and this port's own doc
  comments explicitly (and wrongly) claimed boost "works identically
  airborne". Fetched RocketSim's own `RLConst.h` directly and found the
  reference defines two distinct constants: `BOOST_ACCEL_GROUND =
  2975/3` (≈991.667, exactly matching this port's existing value) and a
  distinctly higher `BOOST_ACCEL_AIR = 3175/3` (≈1058.333, about 6.5%
  more) — a genuine ground/air split this port didn't model, so every
  airborne boost this crate ever applied understated real airborne boost
  strength. Split into `BOOST_ACCELERATION_GROUND`/`BOOST_ACCELERATION_AIR`,
  wired `apply_driven_forces`'s existing `on_ground` parameter to select
  between them, and corrected every doc comment that claimed the two
  were identical. 1 new test; all 291 pre-existing tests pass unchanged.
  292 total in `rb_physics_bullet` (+1 over FR-055's 291).
- `RB-PHYSICS-001-FR-057` (hard cap on car angular speed, implemented) —
  nothing previously bounded how fast sustained air control torque (or a
  dodge's own kick, or the landing-orientation assist) could spin a car,
  so holding full pitch/yaw/roll indefinitely spun a car arbitrarily
  fast, unlike real Rocket League. A second fetch of RocketSim's own
  `RLConst.h`, this time targeting every `drive.rs` constant this port's
  own doc comments flagged as having "no public reference at all,"
  surfaced `CAR_MAX_ANG_SPEED = 5.5f` (rad/s), a hard "can never exceed"
  ceiling this port had no equivalent for. Several other real constants
  the same fetch surfaced (dodge per-direction impulse scaling, auto-flip
  thresholds, a ramping powerslide model, a steering-torque mapping, and
  RocketSim's own per-axis `CAR_AIR_CONTROL_TORQUE`) were considered and
  explicitly not adopted — the torque-based ones repeat FR-031's own
  "false precision" finding (calibrated against RocketSim's own car
  mass/inertia tensor, which this port's placeholder body doesn't match),
  while `CAR_MAX_ANG_SPEED` bounds the *result* (a rad/s quantity)
  instead, so it transfers cleanly regardless. Added
  `drive::MAX_CAR_ANGULAR_SPEED`/`drive::clamp_angular_speed` (a genuine
  clamp, unlike `MAX_CAR_SPEED`'s force-gating), wired in right after
  `integrate::integrate_velocities` in both `world.rs`'s production path
  and `drive.rs`'s own test helper. Also noted, as a coincidence, that
  the pre-existing uncalibrated `DODGE_ANGULAR_SPEED` placeholder is
  numerically equal to this same 5.5 value. 3 new tests; all 292
  pre-existing tests pass unchanged. 295 total in `rb_physics_bullet` (+3
  over FR-056's 292).
- `RB-PHYSICS-001-FR-058` (real speed-dependent throttle taper,
  implemented) — `THROTTLE_ACCELERATION`'s own doc comment had named
  this exact gap since it was introduced: full flat acceleration right up
  to a hard cutoff at `UNBOOSTED_MAX_CAR_SPEED`, not a genuine taper.
  Fetching RocketSim's own `Car.cpp` (not just `RLConst.h`'s constants)
  surfaced the real mechanism: drive force is scaled by a confirmed
  3-point piecewise-linear curve (`{0, 1.0}, {1400, 0.1}, {1410, 0.0}`),
  not applied flat. The curve's underlying magnitude constant
  (`THROTTLE_TORQUE_AMOUNT`) is expressed in Bullet-internal units that
  don't transfer cleanly (repeating FR-031's/FR-057's own "false
  precision" finding), but the curve's *shape* is a pure, unitless ratio
  that does — the same reasoning FR-057 used for `MAX_CAR_ANGULAR_SPEED`.
  Added `drive::DRIVE_SPEED_TAPER_BREAKPOINTS`/`drive_speed_taper`,
  replaced the hard cutoff with the real taper (still evaluated against
  this port's own pre-existing signed, direction-aware speed — not
  RocketSim's own direction-agnostic input, left out of scope), and
  corrected doc comments describing this as unmodeled.
  `THROTTLE_ACCELERATION`'s own peak magnitude remains an uncalibrated
  placeholder; only the curve's shape is now confirmed and modeled. 2 new
  tests; all 295 pre-existing tests pass unchanged. 297 total in
  `rb_physics_bullet` (+2 over FR-057's 295).
- `RB-PHYSICS-001-FR-059` (real forward-speed-dependent dodge impulse
  scaling, implemented) — `RB-PHYSICS-001-FR-031`'s own audit had already
  found real Rocket League's dodge has "direction/speed-dependent
  scaling" but couldn't adopt it without the actual formula. Fetching
  RocketSim's own `Car.cpp` (the same file/technique FR-058 used)
  surfaced the real mechanism: a dodge's base impulse scales per-axis by
  `((maxSpeedScale - 1) * forwardSpeedRatio) + 1`, where `maxSpeedScale`
  is `1.0` for a forward dodge (no change), `2.5` for a backward dodge
  (opposing current velocity), or `1.9` for any side dodge. Adopted the
  confirmed real *ratios* only, via two new functions
  (`drive::dodge_speed_scale`/`dodge_pitch_is_backward`), wired into both
  the ground-dodge and wall-jump-dodge blocks — deliberately not the real
  base magnitude (RocketSim's `FLIP_INITIAL_VEL_SCALE = 500.f` vs this
  port's own unchanged `DODGE_SPEED = 1400.0`), since the real
  forward-dodge scale of exactly `1.0` means `DODGE_SPEED` already stands
  in for that case — the same "shape confirmed, magnitude not" split
  FR-058 established. Also not adopted: RocketSim's own diagonal-dodge
  direction normalization and its continuous-torque-over-time spin model,
  both left for a future requirement. 5 new tests; all 297 pre-existing
  tests pass unchanged — every existing dodge test dodges from a standing
  start, where the new scale evaluates to `1.0` regardless of direction,
  a zero-regression-risk property confirmed by inspection before
  implementation. 302 total in `rb_physics_bullet` (+5 over FR-058's
  297).
- `RB-PHYSICS-001-FR-060` (landing auto-orientation vs. real auto-flip/
  auto-roll, audit finding — documentation only) — `RB-PHYSICS-001-FR-057`'s
  own Non-goals had left open whether real Rocket League's auto-flip could
  map onto `drive::LANDING_AUTO_UPRIGHT_TORQUE` "without further
  investigation." Fetching RocketSim's real `Car.cpp` (the same
  file/technique FR-058/FR-059 used) resolved it: real Rocket League has no
  mechanic matching "continuously nudge an airborne car upright with no
  player input" at all — it has two distinct, real, grounded, input-gated
  systems instead (auto-flip: a jump-triggered turtle-recovery flip past a
  roll threshold; auto-roll: a throttle-triggered ground-alignment torque),
  neither airborne nor input-free. Corrected the `drive` module's doc
  comments, this spec's stale Open Questions bullet, and FR-057's own
  Non-goals bullet accordingly. No behavioral change, no new tests
  (documentation-only, matching FR-044's own precedent); all 302
  pre-existing tests pass unchanged.
- `RB-PHYSICS-001-FR-061` (hard caps on ball linear/angular speed,
  implemented) — the ball had no linear or angular speed cap of any kind
  (unlike the car, which has had a hard angular-speed ceiling since
  FR-057). Fetching RocketSim's own `RLConst.h`/`Ball.cpp` (matching
  FR-057/FR-060's own method) found two confirmed real hard caps —
  `BALL_MAX_SPEED = 6000.f`, `BALL_MAX_ANG_SPEED = 6.f` — enforced by a
  hard clamp after collision resolution, at the end of the physics tick.
  Both are pure velocity caps, transferring cleanly regardless of this
  port's own ball calibration, the same category FR-057 established.
  Added `world::BALL_MAX_SPEED`/`BALL_MAX_ANG_SPEED` and a new
  `world::clamp_ball_velocity` (generalizing `drive::clamp_angular_speed`'s
  own shape to linear and angular speed), wired into `PhysicsWorld::step`
  right after this step's contact resolution — matching real RocketSim's
  own placement more precisely than the car's own earlier-in-pipeline
  clamp. Deliberately not adopted: `BALL_DRAG = 0.03f`, since real
  RocketSim sets it once at ball construction as a per-match
  mutator-config default, not a hardcoded system invariant — left for a
  future requirement. 4 new tests; all 302 pre-existing tests pass
  unchanged — no existing test set the ball's speed or angular speed
  anywhere near either cap, an explicit zero-regression-risk property
  confirmed by inspection before implementation. 306 total in
  `rb_physics_bullet` (+4 over FR-060's 302).
- `RB-PHYSICS-001-FR-062` (real ball material properties via a new
  `RigidBody::ball` constructor, implemented) — FR-061's own Non-goals had
  deferred adopting `BALL_DRAG` for lack of a dedicated ball-construction
  API (`sphere` gives every caller an identical generic `0.5`/`0.5`/`0.0`
  placeholder). Fetching RocketSim's own `RLConst.h` (matching
  FR-057/FR-060/FR-061's own method) confirmed `BALL_RESTITUTION = 0.6f`,
  `BALL_FRICTION = 0.35f`, and `BALL_DRAG = 0.03f` — none a torque/force
  calibrated against a specific mass/inertia, so all three transfer
  cleanly. Added `body::RigidBody::ball(radius, mass, position)`, new
  additive API alongside `sphere`/`car_box`: identical for
  `radius`/`mass`/`position`, but sets `restitution = 0.6`, `friction =
  0.35`, `linear_damping = 0.03` instead of the generic defaults; `sphere`
  itself unchanged. Deliberately not adopted: `BALL_MASS_BT = CAR_MASS_BT
  / 6.f`, since this project has no canonical "real" car construction
  site yet to keep a `1:6` ratio against — left for a future requirement.
  3 new tests; all 306 pre-existing tests pass unchanged. 309 total in
  `rb_physics_bullet` (+3 over FR-061's 306).
- `RB-PHYSICS-001-FR-063` (real Rocket League uses per-contact-pair-type
  restitution/friction overrides, not a per-body combine — audit finding,
  documentation only) — FR-043 had left open which formula matches real
  Rocket League for `solver::combine_restitution`/`combine_friction`.
  Fetching RocketSim's own `RLConst.h` (matching FR-057/FR-060/FR-061/
  FR-062's own method) found the real answer isn't a different formula:
  real Rocket League hardcodes a distinct value per named contact-pair
  type — `CARWORLD_COLLISION_FRICTION/RESTITUTION = 0.3f`/`0.3f`,
  `CARCAR_COLLISION_FRICTION/RESTITUTION = 0.09f`/`0.1f`,
  `CARBALL_COLLISION_FRICTION/RESTITUTION = 2.0f`/`0.0f` — overriding
  whatever a generic per-body combine would produce. Most strikingly, a
  car hitting the ball has zero restitution-driven bounce in real Rocket
  League regardless of either body's own material (this port's own
  combine currently averages the ball's real `0.6` against the car's
  generic `0.5` to `~0.55` for that exact pairing), and car-vs-ball
  friction is above `1.0`, a value no per-body combine could produce.
  Corrected `combine_restitution`/`combine_friction`'s own doc comments
  and this spec's stale Open Questions bullet. Not adopted: implementing
  real per-pair-type overrides, since those functions' own two-`f32`-in
  signature has no way to know which kind of pair produced its inputs —
  left for a future, dedicated requirement. No behavioral change, no new
  tests (documentation-only, matching FR-044/FR-060's own precedent); all
  309 pre-existing tests pass unchanged.
- `RB-PHYSICS-001-FR-064` (real mandatory minimum-hold window for a ground
  jump's variable-height acceleration) — `drive::JUMP_HOLD_MAX_DURATION`'s
  own doc comment had named this exact gap since FR-031's original audit:
  real Rocket League scales its jump-hold acceleration down during a
  `JUMP_MIN_TIME` (0.025s) window rather than applying it flat, an
  unmodeled "two-phase ramp". Fetching RocketSim's own `Car.cpp`
  (`_UpdateJump`, matching FR-058/FR-059's own real-implementation-file
  method) confirmed the exact mechanism: the hold force keeps applying,
  scaled by `JUMP_PRE_MIN_ACCEL_SCALE = 0.62f`, for `JUMP_MIN_TIME`
  seconds regardless of whether `jump` is still held — even an
  instantaneous tap gets a small amount of extra height in real Rocket
  League. Added `drive::JUMP_MIN_TIME`/`JUMP_PRE_MIN_ACCEL_SCALE` and
  reworked `apply_driven_forces`'s hold-acceleration check to derive
  elapsed time since the press from the existing `jump_hold_time_remaining`
  state instead of adding a second field, so no caller needed to change.
  A genuine behavioral fix, not a doc correction. 3 new tests pin the
  mandatory window's own scaled acceleration, its immunity to an early
  release, and its on-schedule closure even when jump is never held; all
  309 pre-existing tests pass unchanged, bringing the crate to 312.
- `RB-PHYSICS-001-FR-065` (real steering is a wheeled-vehicle raycast
  model, not a torque, with an inverted speed-vs-turning-ability curve —
  audit finding, documentation only) — `drive::STEER_TORQUE` had no
  public reference at all. Fetching RocketSim's own `Car.cpp`
  (`_UpdateWheels`, matching FR-058/FR-059/FR-064's own method) found
  real Rocket League's steering isn't a direct yaw-torque model: a
  wheel's steer angle (from a confirmed `STEER_ANGLE_FROM_SPEED_CURVE`)
  feeds Bullet's own raycast vehicle system (`btVehicleRL`), whose
  per-wheel lateral tire friction is what actually turns the car — an
  architecture this port's single-rigid-box car has no way to represent,
  the same category FR-063 established. The confirmed curve's own shape
  is also the opposite of this port's own `speed_factor`: real turning
  ability is highest at a standstill and decreases with speed, while
  this port's `speed_factor` is zero at a standstill and scales up with
  speed. Corrected `STEER_TORQUE`'s and `MAX_CAR_SPEED`'s own doc
  comments and the `speed_factor` call site's comment; also fixed
  adjacent stale text in the spec's own Open Questions section that
  still claimed `AIR_CONTROL_TORQUE`/`JUMP_HOLD_MAX_DURATION`/
  `JUMP_HOLD_ACCELERATION` had no public reference, contradicting
  FR-057's and FR-031's own already-shipped findings. Not adopted as a
  fix: the real curve maps speed to a wheel angle whose translation to
  yaw torque depends on tire-slip friction this port doesn't model,
  leaving no principled way to carry even the curve's shape onto this
  port's own direct-torque model. No behavioral change, no new tests;
  all 312 pre-existing tests pass unchanged.
- `RB-PHYSICS-001-FR-066` (real handbrake friction reduction is
  anisotropic, not a single uniform multiplier — audit finding,
  documentation only) — `drive::HANDBRAKE_FRICTION_MULTIPLIER` had no
  public reference at all. Continuing FR-065's own `_UpdateWheels`
  investigation of RocketSim's real `Car.cpp` found real Rocket League's
  handbrake friction reduction is genuinely anisotropic: a confirmed
  `HANDBRAKE_LAT_FRICTION_FACTOR_CURVE` (a constant `0.1` factor at every
  speed) and a separate confirmed `HANDBRAKE_LONG_FRICTION_FACTOR_CURVE`
  (`0.5` at a standstill, `0.9` at and above 1 uu/s) are applied to
  lateral and longitudinal tire friction independently, not one shared
  multiplier. This port's own pre-existing `HANDBRAKE_FRICTION_MULTIPLIER
  = 0.1` happens to match the real lateral-only factor exactly — a
  striking coincidence, not a confirmation, since this port applies that
  same `0.1` to its single isotropic friction scalar, which the
  ground-contact solver reads identically for every direction, so it
  also wrongly crushes longitudinal grip to a tenth where real Rocket
  League keeps it near `0.9`. Not adopted as a fix: `solver::
  friction_directions` already computes two separate tangent directions
  per contact (since FR-049), but both directions currently share one
  combined-friction scalar when their row limits are computed, so
  genuinely splitting handbrake's own factor by direction would require
  threading a second, direction-specific coefficient through every one
  of `solver.rs`'s several row-limit call sites plus a way for those
  call sites to know a specific body is currently handbraking — the
  same architecture-mismatch category FR-063/FR-065 already established.
  Also fixed adjacent stale text in the spec's own Open Questions section
  that still claimed `HANDBRAKE_FRICTION_MULTIPLIER` had no public
  reference at all. No behavioral change, no new tests; all 312
  pre-existing tests pass unchanged.
- `RB-PHYSICS-001-FR-067` (real Rocket League has no distinct wall-jump
  mechanic or constant at all — audit finding, documentation only) —
  `drive::WALL_JUMP_HORIZONTAL_SPEED` had no public reference at all.
  Fetching RocketSim's real `Car.cpp` (`_UpdateJump`) found real Rocket
  League's `_UpdateJump` applies exactly one impulse, `GetUpDir() *
  mutatorConfig.jumpImmediateForce` (the same real value this port's own
  `JUMP_SPEED` already matches), gated only on `isOnGround`, itself
  defined purely by wheel-contact count with no floor-vs-wall distinction
  at all; a dedicated search of `RLConst.h` found no `WALL_JUMP`-named
  constant anywhere. Since FR-065 already confirmed real cars ride
  Bullet's own raycast vehicle system, a car driving on a wall has its own
  orientation continuously tipped by wheel/suspension contact forces to
  match that wall, so `GetUpDir()` already points along the wall's
  outward normal by the time a wall jump fires — real Rocket League's
  "wall jump" is the identical single grounded-jump impulse, not a
  distinct horizontal-plus-vertical composite, closing a thread FR-031's
  original audit only briefly noted without confirming the exact
  mechanism. Not adopted as a fix: this port's car has no wheels,
  raycasting, or surface-tracking orientation system at all (the same
  architecture gap FR-065 found for steering) — applying only `JUMP_SPEED`
  straight up on a wall touch would produce no push-off at all in this
  port's own model, so its own two-component composite substitute remains
  deliberate and necessary. Also fixed adjacent stale text in the spec's
  own Open Questions section that still claimed
  `WALL_JUMP_HORIZONTAL_SPEED` had no public reference at all. No
  behavioral change, no new tests; all 312 pre-existing tests pass
  unchanged.
- `RB-PHYSICS-001-FR-068` (real per-axis air-control torque ratio,
  implemented) — all three axes (pitch/yaw/roll) shared one flat
  `drive::AIR_CONTROL_TORQUE` magnitude. FR-031's own audit had already
  found real air-control torque coefficients exist but didn't adopt them,
  since they're absolute torques calibrated against real Rocket League's
  own specific mass/inertia tensor. Fetching RocketSim's real `Car.cpp`
  (`_UpdateAirTorque`) found the real mechanism is structurally identical
  to this port's own — a direct per-axis torque scaled by analog input —
  unlike steering or handbrake's own architecture mismatches, with
  `RLConst.h` confirming `CAR_AIR_CONTROL_TORQUE = Vec(130, 95, 400)`
  (pitch-yaw-roll order). Because the mechanism matches, the confirmed
  per-axis ratio (unlike the real absolute values) is adoptable the same
  way FR-058's throttle taper and FR-059's dodge scale ratios are: added
  `AIR_CONTROL_YAW_SCALE = 95.0/130.0` and `AIR_CONTROL_ROLL_SCALE =
  400.0/130.0`, wired into `apply_driven_forces`'s yaw/roll torque
  application; `AIR_CONTROL_TORQUE` itself (pitch's own magnitude) is
  unchanged, still uncalibrated. A genuine behavioral fix, not a doc
  correction: yaw now produces measurably less angular velocity than
  pitch, and roll measurably more, for equal analog input. 2 new tests
  pin the exact expected angular velocity in closed form; all 312
  pre-existing tests pass unchanged, bringing the crate to 314.
- `RB-PHYSICS-001-FR-069` (real dodge-spin mechanism, documentation-only) —
  continuing the investigation FR-031's own audit first opened (which
  already found real reference constants, `FLIP_TORQUE_X=260`,
  `FLIP_TORQUE_Y=224`, `0.65`s, but not the mechanism behind them).
  Fetching RocketSim's real `Car.cpp` (`_UpdateDoubleJumpOrFlip` and
  `_UpdateAirTorque`) found a flip's spin is a continuous per-axis torque,
  not `drive::DODGE_ANGULAR_SPEED`'s own instantaneous angular-velocity
  kick: a per-axis `flipRelTorque` is recorded once at flip start, then
  applied every physics tick for as long as `isFlipping = hasFlipped &&
  flipTime < FLIP_TORQUE_TIME` holds (a hard 0.65s cutoff, no decay or
  ramp). Not adopted as a fix: real Rocket League's own resulting spin
  rate depends on its own specific hitbox inertia tensor this port's
  placeholder car body doesn't match (the same "false precision" reasoning
  FR-031 already applied), and reproducing the real timed-torque shape
  would need new per-car elapsed-flip-time state threaded through
  `PhysicsWorld`, a redesign FR-059's own Non-goals already flagged as out
  of scope. Corrected `DODGE_ANGULAR_SPEED`'s own doc comment, the module
  doc's dodge section, the "commonly-cited constants" paragraph, and the
  adjacent stale Open Questions bullet. No behavioral change, no new
  tests; all 314 pre-existing tests pass unchanged.
- `RB-PHYSICS-001-FR-070` (real flip-cancel mechanism, documentation-only) —
  `FR-069`'s own fetch of `_UpdateAirTorque` surfaced a `pitchTorqueScale`
  factor scoped out at the time as "an additional speed- or state-dependent
  scale... didn't fully characterize." Fetching RocketSim's real `Car.cpp`
  again closed that thread: real Rocket League's flip-cancel is driven by
  continuously holding pitch in the same direction as the flip's own
  pitch-torque component, scaling only that pitch-axis component by
  `1 - abs(controls.pitch)` every tick — not this port's own jump-press
  trigger that zeros every axis outright. This port's own flip-cancel doc
  comment had claimed to match real Rocket League without that claim ever
  being checked against real source; a sideways (roll-only) dodge has no
  pitch-torque component, so real Rocket League can't pitch-cancel it at
  all, unlike this port's own direction-independent cancel. Not adopted as
  a fix: this port's dodge has no per-axis torque split to partially cancel
  (the same architecture gap FR-069 already found for the dodge's own
  spin), and reproducing the real continuous-hold trigger and pitch-only
  scope would need the same per-axis torque and elapsed-flip-time state
  FR-059's own Non-goals already flagged as out of scope. Corrected the
  `drive` module's flip-cancel doc comment and added a forward citation
  from FR-016's own entry. No behavioral change, no new tests; all 314
  pre-existing tests pass unchanged.
- `RB-PHYSICS-001-FR-071` (real air-control damping mechanism,
  documentation-only) — FR-068's own Non-goals had already found
  RocketSim's `CAR_AIR_CONTROL_DAMPING = Vec(30, 20, 50)` exists but left
  it as "a separate, independent addition left for a future requirement"
  without examining the mechanism. Fetching RocketSim's real `Car.cpp`
  again (the same fetch FR-070 used for `pitchTorqueScale`) found the full
  mechanism: for each axis, real air control subtracts a damping torque
  `(angular velocity along that axis) * CAR_AIR_CONTROL_DAMPING[axis] *
  (1 - abs(analog input on that axis))` from the applied torque before
  scaling by inertia — releasing the stick gives full damping strength,
  continuously bleeding off spin; holding it fully zeroes the damping,
  granting full torque authority. Not adopted as a fix: unlike
  `AIR_CONTROL_TORQUE`'s own pitch/yaw/roll ratio, this port has no
  existing damping quantity to apply a ratio to — introducing one is a
  genuinely new mechanism, not a multiplier transfer — and its absolute
  coefficients are calibrated against real Rocket League's own specific
  inertia tensor, the same "false precision" reasoning that already keeps
  `AIR_CONTROL_TORQUE` a placeholder. Corrected the `drive` module's
  air-control doc comment and `AIR_CONTROL_ROLL_SCALE`'s own doc comment,
  and added a forward citation from FR-068's own Non-goals. No behavioral
  change, no new tests; all 314 pre-existing tests pass unchanged.
- `RB-PHYSICS-001-FR-072` (normalized diagonal-dodge direction,
  implemented) — FR-059's own Non-goals had already found and flagged a
  genuine gap: this port summed each dodge axis' own full-strength
  `(pitch, roll)` contribution independently, so a diagonal dodge came out
  `sqrt(2)`-ish times faster than an axis-aligned one, unlike real Rocket
  League. Fetching RocketSim's real `Car.cpp` (`_UpdateDoubleJumpOrFlip`)
  confirmed the real mechanism: `dodgeDir = btVector3(-pitch, yaw + roll,
  0).safeNormalized()`, normalized to unit length before any further
  speed-based scaling. Because normalizing a direction vector needs no
  new machinery this port lacks, it transfers cleanly the same way
  FR-058/FR-059/FR-068's own adopted ratios do. Added
  `drive::normalize_dodge_direction`, wired into both the ground-dodge
  and wall-jump-dodge code paths — the per-axis `DODGE_DEADZONE` trigger
  and `dodge_pitch_is_backward`'s sign check still read raw stick values;
  only the scaled magnitude changes. This port's own sign convention is
  kept and yaw isn't folded in, both already-documented, separate
  simplifications. A genuine behavioral fix, not a doc correction:
  updated the two existing diagonal-dodge tests to assert the corrected
  magnitude and added 3 new tests for `normalize_dodge_direction`
  directly; all pre-existing tests pass unchanged, bringing the crate to
  317.
- `RB-PHYSICS-001-FR-073` (fold yaw into the dodge direction, implemented)
  — FR-059's own Non-goals (and FR-072's own doc comment) had already
  found and flagged this port's dodge/wall-jump-dodge direction reads
  `pitch`/`roll` only, never `yaw`, unlike real Rocket League's own
  `dodgeDir = (-pitch, yaw + roll, 0)`. Confirming via RocketSim's real
  `Car.cpp` that `controls.yaw` feeds nowhere else in the function, and
  that this port already reads `input.yaw` in the same function for air
  control, folding it into the dodge's roll-axis stick value (`roll +
  yaw`, each clamped individually first) at both the ground-dodge and
  wall-jump-dodge call sites needed no new machinery — a pure additive
  combination of an already-available input, the same "pure operation, no
  new architecture" transfer FR-058/FR-059/FR-068/FR-072's own adopted
  findings share. A genuine behavioral fix, not a doc correction: a
  yaw-only stick press now fires a sideways dodge the same as a roll-only
  one would. Added 3 new tests (a yaw-only dodge, a yaw-and-roll
  cancellation, and a yaw-only wall-jump-dodge); all pre-existing tests
  pass unchanged, bringing the crate to 320.
- `RB-PHYSICS-001-FR-074` (snap a near-axis-aligned dodge to a pure single
  axis, implemented) — FR-073's own Non-goals had flagged RocketSim's
  post-normalization small-component zeroing as "a separate, independent
  simplification," a mis-scoping this requirement corrects: it's a further
  pure post-processing step on `normalize_dodge_direction`'s own
  already-computed normalized pair, needing no new machinery, exactly like
  normalization itself (FR-072). Re-confirmed via RocketSim's real
  `Car.cpp`: after `dodgeDir.safeNormalized()`, any component whose
  magnitude falls below `0.1` is zeroed, not re-normalized afterward.
  Added `drive::DODGE_DIRECTION_SNAP_THRESHOLD = 0.1` (a distinct constant
  from `DODGE_DEADZONE` despite sharing the same real value, since they
  serve different real purposes) and wired the zeroing into
  `normalize_dodge_direction`'s own return path — both dodge call sites
  already route through it, so no call-site changes were needed. A
  genuine behavioral fix: a near-axis-aligned diagonal stick input now
  snaps to a clean single-axis dodge instead of a slightly diagonal one.
  Added 2 new tests pinning the snap behavior at both sides of the
  threshold; all pre-existing tests pass unchanged, bringing the crate to
  322.
- `RB-PHYSICS-001-FR-075` (confirm `DODGE_DEADZONE` matches RocketSim's own
  real cancellation threshold, audit finding) — this spec's own Open
  Questions had claimed `DODGE_DEADZONE` "still has no public reference at
  all," and FR-074's own Non-goals (mirroring FR-073's identical earlier
  claim) separately framed RocketSim's all-or-nothing dodge-cancellation
  check as "a real but separate architectural difference" from this
  port's own trigger. Both were wrong: RocketSim's own confirmed check
  (already quoted verbatim during FR-072/FR-073/FR-074's own
  investigations) fires iff `abs(yaw + roll) >= 0.1 || abs(pitch) >= 0.1`;
  since FR-073 already folds yaw into this port's own `dodge_roll`, this
  port's own trigger is the identical boolean decision once
  `DODGE_DEADZONE == 0.1` — the same real value, differing only in an
  unobservable strict-vs-non-strict boundary comparison. Corrected
  `DODGE_DEADZONE`'s own doc comment, the module doc, this spec's stale
  Open Questions bullet, and FR-073's/FR-074's own Non-goals framing. No
  code change: this port's dodge trigger already matched real Rocket
  League exactly. No new tests; all 322 pre-existing tests pass unchanged.
- `RB-VERIFY-002-FR-001` (BakkesMod-side capture plugin, built and run for
  real) — the one step blocked on the owner's own Windows/BakkesMod/game
  environment is done: built with MSVC (VS2022 Build Tools) + CMake against
  the owner's own installed `BakkesModSDK` copy, loaded into a real Rocket
  League + BakkesMod session, and run in freeplay. The first real capture
  (9,358 lines) proved the hook (`Function TAGame.Car_TA.SetVehicleInput`)
  fires correctly and the ball's live physics state records correctly, but
  surfaced a genuine bug: enumerating cars via `ServerWrapper::GetPRIs()` +
  `PriWrapper::GetCar()` recorded the same frozen spawn-point transform and
  all-zero input on every line, since a PRI's `Car` back-reference is never
  updated in freeplay (PRI is for scoreboard/stat tracking, which freeplay
  has none of) — confirmed by the ball's own recorded velocity spiking
  mid-session while the "car" entry never moved. Fixed by switching to
  `ServerWrapper::GetCars()`, the game's own live spawned-car-actor list. A
  second real capture (2,818 lines, ~23.5s) confirmed both ball and car
  state update correctly with real, varied controller input (1,612 of 2,818
  ticks with non-zero throttle/steer); every line schema-validated exactly
  against ADR-0005, and the file parsed end-to-end via `rb_capture_ingest`
  with every car entry carrying `Some` input in chronological order (via a
  scratch integration test, not kept in the repo — the capture itself is
  the owner's own personal play data). `RB-VERIFY-002-FR-002` is now
  verified against a real capture too, not just the synthetic fixture. Both
  of `RB-VERIFY-002`'s own former open questions (hook-name and
  format-ergonomics) are resolved. Still open: the stricter manual
  BakkesMod-overlay single-timestamp cross-check (see Blocked, same shape
  as `RB-VERIFY-001`'s equivalent item) and NFR-002 (recording overhead,
  unmeasured).
- `PHASE-0-EXIT` — its literal exit gate ("pipeline runs end-to-end and
  produces a divergence score on ≥1 real replay and ≥1 real BakkesMod
  capture") is now met: `rb_verify_cli` was run against the vendored real
  replay fixture and the real BakkesMod capture from
  `RB-VERIFY-002-FR-001` above (`frames compared: 343, mean ball distance:
  3640.81 uu, car pairs compared: 343, mean car position/rotation/velocity
  distance: 4714.78 uu / 2.31 rad / 2127.93 uu/s`), with no errors and ball
  scoring, car scoring, and timestamp-tolerant alignment all engaged. This
  closes all four `PHASE-0-*` roadmap units. As before, the number itself
  is not a meaningful fidelity measurement (the replay and capture are
  unrelated matches) — that was never this gate's own criterion, and
  closing it for real needs a Phase 1 candidate physics engine that
  doesn't exist yet (see `RB-PHYSICS-001`'s own `FR-005`, now unblocked in
  the sense that real data finally exists to calibrate against, but not
  itself started).
- `RB-PHYSICS-001-FR-076` (candidate-engine plumbing, implemented):
  `rb_physics_bullet` can now seed a `PhysicsWorld` from a recorded
  `PhysicsFrame` (`PhysicsWorld::from_frame`) and simulate it forward
  using a recorded per-tick controller-input sequence
  (`world::simulate_recorded`) — the exact next step `world::simulate`'s
  own doc comment already named as the thing to do "once `RB-VERIFY-002`
  capture data exists" — the prerequisite plumbing `FR-005`'s real-data
  calibration needs. Along the way, fetched RocketSim's own real car
  mass/hitbox and ball mass directly from source (`body::CAR_MASS`,
  `body::CAR_HALF_EXTENTS`, `body::BALL_MASS`, new
  `RigidBody::standard_car`/`standard_ball` constructors), surfacing a
  real, previously-unnoticed ~44% width discrepancy in this crate's own
  long-standing car hitbox test placeholder — deliberately left
  uncorrected at every existing test call site (a separate calibration FR
  of its own, not this one). 13 new unit tests (335 `rb_physics_bullet`
  tests total, up from 322); full workspace `fmt`/`clippy`/`test` green.
  Known limitation, unchanged from the original scoping: `PhysicsWorld`
  still has no setter for a car's mid-air jump/dodge state, so a seed
  frame needs to be a grounded, neutral moment.
- `RB-PHYSICS-001-FR-077` (wire candidate engine into `rb_verify_cli`,
  implemented, verified against a real capture): `rb_verify_cli` gains
  `score_capture_against_candidate`, seeding a `PhysicsWorld` from a
  capture's own first grounded, neutral frame (`is_grounded_and_neutral`,
  proxying for `FR-076`'s unset hidden jump/dodge state being accurate
  there) and scoring a candidate simulated from that capture's own
  recorded input against the capture's own recorded outcome — this
  project's first fidelity comparison with a genuine physical reason to be
  small if the physics core is accurate. A new `rb-verify --self
  <capture-file>` CLI mode exposes it. 3 new unit tests (happy path
  against the synthetic capture fixture, missing-file, and
  no-qualifying-frame `Malformed` cases); full workspace `fmt`/`clippy`/
  `test` green (388 tests). The owner then ran `cargo run -p rb_verify_cli
  -- --self test2.jsonl` against the real capture from
  `RB-VERIFY-002-FR-001` (2,818 frames) on their own machine, producing
  this project's first genuine fidelity number:
  ```
  frames compared:    2818
  mean ball distance: 2206.08 uu
  max ball distance:  5673.98 uu
  car pairs compared: 2818
  mean car position/rotation/velocity distance: 4508.71 uu / 2.12 rad / 1421.73 uu/s
  max  car position/rotation/velocity distance: 8798.56 uu / 3.14 rad / 3643.64 uu/s
  ```
  A large divergence — for scale, the standard arena's own half-width is
  `4096.0` and half-length `5120.0`, and `Quat::angle_to`'s range is `[0,
  π]` (confirmed by this run's own max rotation distance of `3.14`), so a
  mean car rotation distance of `2.12` is past even what a uniformly
  random orientation would average — consistent with near-total
  trajectory decorrelation over the run's own ~23-second span, not a
  small, bounded fidelity gap. Expected rather than alarming: physics
  simulation is chaotic (any modeling error compounds over dozens of
  seconds of free simulation from one seed frame) and this port's own
  extensively self-documented gap list (uncalibrated placeholder
  constants, no tire-slip steering model, no per-axis air-control
  damping, anisotropic handbrake friction unmodeled, among others)
  guarantees real modeling error exists. See `RB-PHYSICS-001`'s own
  Interpretation note under FR-077 for the full reasoning, including what
  this single number does *not* yet establish (gradual vs. abrupt
  divergence) and why `RB-PHYSICS-001-FR-005` (real-data constant
  calibration) still hasn't started — this result isn't yet the right
  shape of evidence to calibrate individual constants from.
- `RB-PHYSICS-001-FR-078` (car hitbox calibration, implemented, verified):
  every existing `car_box`-style test helper across `rb_physics_bullet`
  (`body.rs`/`collision.rs`/`drive.rs`/`net.rs`/`solver.rs`/`world.rs`)
  that modeled a real car was switched from the old placeholder
  half-extents (`Vec3::new(60.0, 30.0, 18.0)`) to the confirmed real
  `body::CAR_HALF_EXTENTS` `FR-076` introduced but left every pre-existing
  call site untouched. An arbitrary shape unrelated to a real car (a unit
  cube, a symmetric pair of identical boxes, a tiny corner-testing probe)
  was deliberately left alone. Rather than hand-recomputing every
  downstream literal for an anisotropic dimension change (X +0.4%, Y
  +44.5%, Z +7.4%, unlike `FR-036`'s uniform scalar radius substitution),
  each test's own duplicate-literal dependency was refactored to read the
  actual half-extents it constructed its own car from, then the suite was
  run to find exactly which assertions still needed a real recompute —
  only resting-height thresholds (`position.z` settling on the car's own
  half-extent, `18.0` → `CAR_HALF_EXTENTS.z`) genuinely needed one. Two
  solver-level tests' doc comments citing specific measured pinch
  velocities were re-measured and confirmed unchanged (a purely 1D,
  mass/velocity-driven collision doesn't depend on the absolute
  half-extent value). No new tests — all 335 pre-existing
  `rb_physics_bullet` tests pass unchanged; full workspace `fmt`/
  `clippy`/`test` green (388 tests).
- `RB-VERIFY-003-FR-004` (divergence-growth diagnostic, implemented,
  sanity-checked): a new `rb_domain::divergence::score_windows`
  partitions the same nearest-timestamp-matched pairs the existing
  whole-run `score` uses (both now share a private `matched_pairs`/
  `score_pairs` pipeline) into consecutive `window_secs`-wide time
  buckets and scores each independently, so a single run's divergence can
  be read window-by-window instead of collapsed into one mean/max pair —
  the follow-up `RB-PHYSICS-001-FR-077`'s own real-capture run
  recommended, to tell gradual compounding error apart from an abrupt
  early derailment before `FR-005` calibrates against it. A new
  `rb_verify_cli::score_capture_growth` and `rb-verify --self-growth
  <capture-file> [window-secs] [max-timestamp-delta-secs]` CLI mode
  expose it, sharing a new private `seed_and_simulate` helper with the
  existing `score_capture_against_candidate`/`--self` mode. 4 new
  `rb_domain::divergence` tests (14 total) and 3 new `rb_verify_cli`
  tests (9 total); full workspace `fmt`/`clippy`/`test` green (395
  tests). Manually run once against the synthetic capture fixture,
  confirming the CLI mode runs end-to-end — the real run this diagnostic
  exists for, against `FR-077`'s own `test2.jsonl`, is still pending the
  owner's own machine (see Next).

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
- `RB-VERIFY-002`'s manual BakkesMod-overlay single-timestamp cross-check
  (one physics value pinned against what BakkesMod's own overlay/logging
  reports for that same instant) — same shape as, and still open for the
  same reason as, `RB-VERIFY-001`'s equivalent item above: needs the owner
  to do it locally, since this sandbox has no way to verify an exact
  remembered timestamp.

## Next

1. (Optional, owner-side, non-blocking) The manual BakkesMod-overlay
   single-timestamp cross-checks for `RB-VERIFY-001`/`RB-VERIFY-002` (see
   Blocked).
2. Running the now-implemented `RB-VERIFY-003-FR-004` divergence-growth
   diagnostic (`rb-verify --self-growth`) against `FR-077`'s own real
   capture (`test2.jsonl`) on the owner's machine — the run that would
   actually show whether that run's divergence grew gradually (many
   small modeling errors compounding, pointing at broad constant
   calibration) or abruptly (one specific early mechanic mismatch
   derailing the whole run, pointing at a targeted fix instead). Only
   sanity-checked so far against the synthetic capture fixture (see
   Validation); recommended before `RB-PHYSICS-001-FR-005` (real-data
   constant calibration) starts, since blind curve-fitting against a
   fully-decorrelated trajectory isn't sound.

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (395 tests: 27 in `rb_domain` (incl. 4
  new `score_windows` tests, `RB-VERIFY-003-FR-004`), 335 in
  `rb_physics_bullet`, 14 in `rb_replay_ingest` (incl. real-fixture
  integration test), 10 in `rb_capture_ingest` (incl. synthetic-fixture
  test), 9 in `rb_verify_cli` (incl. `score_capture_against_candidate`'s
  and `score_capture_growth`'s happy-path runs against the synthetic
  capture fixture), plus doc-tests)
- `cargo run -p rb_replay_ingest --bin corpus_check` (local only, not CI):
  40/40 real owner replays parsed cleanly, 2026-08-28
- `cargo run -p rb_verify_cli --bin rb-verify -- <replay> <capture>`
  (manual, 2026-08-28, default 0.02s timestamp tolerance): `frames
  compared: 6, mean ball distance: 0.25 uu, max ball distance: 0.25 uu,
  car pairs compared: 6, mean car position/rotation/velocity distance:
  2816.42 uu / 2.36 rad / 1307.87 uu/s` against the real replay fixture +
  (now time-aligned) synthetic capture fixture.
- `cargo run -p rb_verify_cli --bin rb-verify -- --self test2.jsonl`
  (manual, owner's machine, 2026-09-04, default 0.02s timestamp
  tolerance, `RB-PHYSICS-001-FR-077`'s own real-capture run): `frames
  compared: 2818, mean ball distance: 2206.08 uu, max ball distance:
  5673.98 uu, car pairs compared: 2818, mean car position/rotation/
  velocity distance: 4508.71 uu / 2.12 rad / 1421.73 uu/s, max car
  position/rotation/velocity distance: 8798.56 uu / 3.14 rad / 3643.64
  uu/s` — this project's first genuine fidelity number (candidate
  actually simulated from the real capture's own recorded input, not an
  unrelated match); see FR-077's own Completed entry and
  `RB-PHYSICS-001`'s Interpretation note for what this large a divergence
  does and doesn't establish.
- `cargo run -p rb_verify_cli --bin rb-verify -- --self-growth
  crates/rb_capture_ingest/fixtures/example.capture.jsonl` (manual,
  2026-09-04, default `window_secs = 1.0`, `RB-VERIFY-003-FR-004`): `t=
  11.78s frames= 5 ball mean/max= 0.75/ 2.17 uu car mean pos/rot/vel=
  58.75 uu / 0.05 rad / 600.40 uu/s` — a single window, since the
  fixture's own 5 frames all fall within one second; confirms the new
  `--self-growth` CLI mode runs end-to-end. Not the diagnostic's real
  purpose — running it against `FR-077`'s own real capture is still
  pending the owner's own machine (see Next).

## Risks and decisions needed

- `RB-RESEARCH-O001` (build vs. integrate physics) — **resolved**, see
  ADR-0004.
- `RB-RESEARCH-O002` (binary reverse engineering) — needs explicit owner
  sign-off after legal/practical review before any work starts, and needs
  the owner's own machine/game install since this sandbox has neither.
  Owner: baileyrd.
- `RB-RESEARCH-O003` (capture tooling scope) — **resolved**, see ADR-0005
  (one-off script, JSON-Lines format).
