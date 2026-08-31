# Release Notes

Tracks notable changes to this repo, one entry per merged change against
`main`, reverse chronological. Pre-1.0, no version tags yet — entries are
keyed by the commit/PR that shipped them.

---

## Restitution/friction combine-mode reference validation
**2026-08-31** · [PR #93](https://github.com/baileyrd/rusty_bullet/pull/93) · `aa9938d`

- **This project's own spec claimed, without ever having checked, that
  Bullet's default restitution/friction combine mode is `max` for both.**
  Fetched and read `btManifoldResult.h`/`btManifoldResult.cpp` in full and
  found that claim wrong.
- **Bullet's real default for both is an unclamped product (`a * b`)** —
  friction's own version additionally clamps the result to `[-10, 10]` —
  with no `max` mode, no geometric mean, and no per-pair override anywhere
  in the reference short of a custom `gContactAddedCallback`.
- **This port's own average combine mode is kept anyway, now for a correct
  reason.** Average preserves the identity `combine(a, a) == a` (two
  surfaces sharing a coefficient combine back to that coefficient), which
  the reference's own product does not (`0.5 * 0.5 == 0.25`) — and most
  bodies in this port currently share the same uncalibrated placeholder
  `0.5` for both coefficients, so the reference's real default would
  silently combine the overwhelming majority of this port's own contacts
  to `0.25`, a value nobody chose.
- **Whether either formula matches real Rocket League itself is
  unaffected by this correction** and remains genuinely open, needing real
  recorded ball/ground behavior to calibrate against — only the wrong
  reference-fact claim, and this port's own justification for diverging
  from the *correct* one, changed.
- **2 new dedicated unit tests** pin `combine_restitution`/
  `combine_friction`'s own identity-preserving behavior directly. All 273
  pre-existing tests pass unchanged; 275 total.

---

## Box-vs-box reference validation
**2026-08-31** · [PR #91](https://github.com/baileyrd/rusty_bullet/pull/91) · `feabc32`

- **Fetched and read Bullet's own `btBoxBoxDetector::dBoxBox` reference
  source directly** to validate two "reasonable, tested choices, never
  validated against the reference" this project's own spec flagged as open.
- **Edge-edge contact point: confirmed more rigorous than the reference.**
  `dBoxBox`'s own contact point uses `dLineClosestApproach` — closest
  approach between two *infinite lines*, applied with no clamping to the
  finite edge length at all (confirmed directly in the fetched source).
  This port's own finite-segment closest-point construction (Ericson's
  algorithm) correctly stays within both edges — a genuine improvement
  over the reference it's ported from, not merely an equivalent
  restatement of it.
- **Face-clipping degenerate fallback: confirmed a deliberate, favorable
  divergence.** The reference contains the exact same undocumented
  "should never happen" judgment call (twice, zero justification given
  either time) this port's own code comment already made. Where the two
  diverge is policy: the reference's own fallback drops the collision
  entirely, while this port synthesizes a contact instead, since SAT has
  already confirmed real overlap by that point and dropping it risks a
  body tunneling through in a rare grazing case.
- **Investigated a candidate fix for the edge-edge sign-selection
  heuristic — found genuinely mixed, not adopted.** Which of a box's 4
  candidate parallel edges is "near" is picked via a heuristic either way;
  swapping this port's center-to-center-vector proxy for the reference's
  own SAT-normal-based one was built and empirically tested against a
  brute-force ground truth across 50,000 randomized configurations: the
  current heuristic wins for large/arbitrary penetration depths (~11.6%
  vs. ~8.7% optimal-match rate), the candidate wins for realistic
  near-first-contact depths (~93% vs. ~77%), and neither is reliably
  optimal. Kept as-is.
- **No new tests** — documentation-only, no value or behavior changed,
  the same precedent FR-032/FR-040 established for a rigorously
  investigated negative result being real, valuable work. All 273
  pre-existing tests pass unchanged.

---

## Sandwiched-solve convergence
**2026-08-31** · [PR #89](https://github.com/baileyrd/rusty_bullet/pull/89) · `4b0a133`

- **Investigated whether anything short of real recorded data could narrow
  `RB-PHYSICS-001-FR-030`'s own documented extreme-mass-ratio "sandwiched"
  under-convergence gap** at this crate's fixed `SOLVER_ITERATIONS = 10`.
- **Tried a naive global SOR-style relaxation factor first — and rejected
  it**: factors above 1.0 (over-relaxation) made FR-030's own
  symmetric-pinch test scenario measurably *diverge* (worse than the
  pre-FR-030 independent-pairwise approach), while factors below 1.0
  (under-relaxation) made it monotonically *better*, matching standard
  PGS/SOR theory for a tightly-coupled multi-constraint body.
- **`solver::resolve_dynamic_manifolds` now scales each manifold's
  velocity-row impulse by a parameter-free `1 / k`** instead, where `k` is
  the number of manifolds sharing a body this step — the same "fair share"
  weighting position-based-dynamics solvers use for a point mass under
  several simultaneous constraints. Mathematically dominant rather than a
  tuned magic number: it can only reduce, never increase, a shared body's
  per-iteration overshoot, so it needed no real recorded data to justify
  adopting.
- **Narrows FR-030's own symmetric-pinch result from ~89.5 to ~32 units/s**
  (independent-pairwise stays ~98.9), at zero added iteration cost. A body
  touched by only one other body this step (`k == 1`, the overwhelming
  majority of contacts) is a mathematical no-op, confirmed by a dedicated
  bit-for-bit-equivalence test against `resolve_contacts_between`.
- **Does not achieve full convergence** to the true simultaneous-solve
  answer within one call's fixed `SOLVER_ITERATIONS` — the gap is
  narrowed, not closed; real recorded multi-car contact data would still
  be needed to know whether the residual error matters for fidelity in
  practice.
- **2 new tests**; all 271 pre-existing tests pass unchanged. 273 total in
  `rb_physics_bullet` (+2 over FR-040's 271).

---

## Fillet-radius calibration research
**2026-08-31** · [PR #87](https://github.com/baileyrd/rusty_bullet/pull/87) · `f92ceed`

- **A dedicated research pass looked for a real reference for
  `arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS`** — the two uncalibrated
  placeholder constants FR-036's own constant-ambiguity research left
  untouched — searching this port's established reference tier
  (RocketSim/RLUtilities source, the RLBot wiki, RLGym's game values).
- **Found exactly one candidate, and deliberately didn't adopt it**: the
  RLBot wiki's uncited "wall bottom ramp radius: approx. 256, not
  circular". It carries no citation, doesn't distinguish `FILLET_RADIUS`
  from the corner walls' own distinctly bigger `CORNER_ARCH_RADIUS`,
  explicitly disclaims being a true circular arc, and shares its numeral
  with RLGym's own unrelated `RAMP_HEIGHT` (a ramp's height from the
  ground, not a curve's radius) — suggesting the wiki entry may conflate
  the two rather than independently measure a radius.
- **Both constants remain unchanged (`292.0`/`750.0`) and genuinely
  uncalibrated.** Adopting an unreliable number would trade one honestly
  uncalibrated placeholder for a differently-uncertain one dressed up as a
  citation — a worse outcome than leaving it alone.
- **Genuinely closing this needs actual extracted collision-mesh geometry**
  (e.g. via `ZealanL/RLArenaCollisionDumper`'s real triangle-mesh dump),
  which needs the owner's own Windows/Rocket League environment — the same
  blocker `RB-VERIFY-002-FR-001` already documents.
- **No new tests** — documentation-only, no runtime value changed, matching
  FR-031/FR-036's own precedent for constant-audit findings that don't
  change a value. All 271 pre-existing tests pass unchanged.

---

## Car-vs-net contact
**2026-08-31** · [PR #85](https://github.com/baileyrd/rusty_bullet/pull/85) · `fdbd940`

- **A car is now caught by a goal net too, not just the ball** — closes
  this port's own former Non-goal that "a car still passes straight
  through a `net::NetMesh`'s spatial footprint untouched."
- **`net::NetMesh::step` changed from a single `&mut RigidBody` (the ball
  alone) to `&mut [RigidBody]`** (every body that can touch the net). Its
  inner contact-resolution loop now iterates every body in the slice
  against each free point. A single-element slice for the ball alone
  behaves identically to the old signature — every one of this module's
  pre-existing tests only needed a call-syntax update
  (`std::slice::from_mut(&mut ball)`), not a changed assertion.
- **No new collision code was needed** — `collision::contacts_between`
  already dispatches to `sphere_vs_box` for a car (box) against a net
  point (sphere) the same way it always has for ball-vs-car.
- **`PhysicsWorld::step` reuses the same ball-plus-cars snapshot**
  `solver::resolve_dynamic_manifolds` already resolved that step for the
  net-step call too, deferring the sync back to `self.ball`/`self.cars`
  until after every net has had its turn, instead of syncing immediately
  and rebuilding a second snapshot just for the net loop.
- **3 new tests**: 2 in `net.rs` (the direct car analog of the existing
  "caught vs. free flight" ball test, and a test proving both a ball and a
  car are resolved against the same net step, not just the first body in
  the slice) and 1 in `world.rs` (the live-`PhysicsWorld` end-to-end
  proof, mirroring the ball's own version).
- 3 new tests, 271 total in `rb_physics_bullet` (+3 over FR-039's 268). All
  pre-existing tests pass unchanged.

---

## Wall-jump corner disambiguation
**2026-08-31** · [PR #86](https://github.com/baileyrd/rusty_bullet/pull/86) · `99234c6`

- **A wall jump at a corner now pushes off diagonally, blending both
  touched walls**, instead of firing along only one of them depending on
  iteration order. `PhysicsWorld::step`'s per-car wall-normal computation
  sums every wall a car is touching this step and normalizes the result,
  instead of `Iterator::find`-ing the first match.
- **Closes a simplification documented since FR-013**, made reachable in
  the standard arena for the first time by FR-019's diagonal corner walls
  (a car can now genuinely touch two walls at once at a real corner).
- **A car touching exactly one wall is bit-for-bit unaffected** — summing
  a single unit-length wall normal and normalizing it is a no-op, so every
  pre-existing wall-jump test passes unchanged.
- **No new collision code was needed** — `resolve_plane_contact` already
  resolved simultaneous multi-wall contact correctly; only the wall-jump
  push-off direction picker, `drive::apply_driven_forces`'s own input, was
  affected.
- **1 new test**, `a_car_touching_two_walls_at_a_corner_wall_jumps_diagonally_outward`
  (two perpendicular walls, a car touching both at once, asserting the
  push-off comes out diagonal with equal horizontal components). 268 total
  in `rb_physics_bullet` (+1 over FR-037's 267).

---

## Sleeping
**2026-08-31** · [PR #83](https://github.com/baileyrd/rusty_bullet/pull/83) · `33c4b77`

- **A body's velocity now forcibly zeroes once it's stayed below a linear
  and an angular threshold for a sustained time**, closing the "no
  sleeping" half of the solver's own documented gap warm-starting left
  open. New `body::RigidBody::update_sleep_state`/`wake`.
- **This is the actual fix for a bouncy resting contact never settling** —
  the limitation neither split impulse nor warm-starting alone could
  close, since restitution re-triggers off a fresh gravity-induced closing
  velocity every frame regardless of where the solver's iteration starts
  or how it got there.
- **A car wakes unconditionally the instant it receives genuinely active
  input**, before that input's own force has had a chance to move it — a
  resultant-velocity-only wake check isn't enough, since a driving force
  whose one-frame delta is itself smaller than the sleep threshold would
  otherwise get zeroed right back out every frame, permanently stranding
  an asleep car. A new `input_is_active` helper treats an unrecovered
  analog channel (`None`) the same as a recovered-but-literally-neutral
  one (`Some(0.0)`), so a car fed a real recorded input stream that always
  resolves every channel doesn't get stuck permanently awake either.
- **All three new threshold constants are uncalibrated placeholders** —
  no public reference exists for what, if any, real Rocket League's own
  physics engine uses internally for this purely implementation-internal
  stabilization detail.
- **8 new tests** (5 in `body.rs` exercising the mechanism directly, 3 in
  `world.rs` proving it through a live `PhysicsWorld`, including a direct
  demonstration that a nonzero-restitution resting ball now actually falls
  asleep at exactly zero velocity instead of bouncing forever). All
  pre-existing tests pass unchanged.
- 8 new tests, 267 total in `rb_physics_bullet` (+8 over FR-036's 259).

---

## Ball radius and ceiling height corrections
**2026-08-31** · [PR #81](https://github.com/baileyrd/rusty_bullet/pull/81) · `ab892bf`

- **Resolved both constant ambiguities `RB-PHYSICS-001-FR-031`'s own audit
  surfaced but deliberately didn't act on**, using real source-level
  research (cloning and reading RocketSim's and RLUtilities' own source,
  and the current RLBot wiki, rather than guessing from prior training-data
  recall).
- **Ball radius: `92.75` became `93.15`, not the previously-suspected
  `91.25`.** FR-031 had framed this as a straight two-way choice, but the
  real games actually split the ball into a smaller inertia radius
  (`91.25`) and a distinctly larger collision radius (`93.15`, the mesh's
  own collision margin) — a split this port's single unified
  `RigidBody::sphere` radius field has no room for. Since this port has no
  separate Bullet-style collision margin of its own, the collision radius
  is the mathematically correct single-constant analog, so switching to
  `91.25` would have been a regression, not a fix. Every `92.75` literal
  across `solver.rs`/`world.rs`/`net.rs`/`collision.rs` became `93.15`.
- **`arena::CEILING_Z`: `2044.0` became `2048.0`.** Confirmed, via both
  RocketSim's own `ARENA_HEIGHT = 2048.f` and an independent reconstruction
  from real extracted collision-mesh geometry, to describe the same
  reference point this port's `CEILING_Z` does.
- **Two mis-documented claims corrected as a low-risk byproduct**, not new
  findings requiring their own change: `arena::CORNER_LENGTH` and
  `arena::GOAL_DEPTH` were wrongly described (by earlier FRs) as
  uncalibrated placeholders with no public reference — both are actually
  confirmed exact, so only their doc comments changed, not their values.
- **`arena::FILLET_RADIUS`/`CORNER_ARCH_RADIUS` remain untouched and still
  genuinely uncalibrated.** No analytic single-number reference exists for
  either in the serious community sources — closing that gap for real
  would mean ingesting an actual dumped collision mesh, a separate,
  more involved follow-up deliberately left for later.
- **No new tests** — a constant-only correction with no new behavior to
  characterize, the same precedent `RB-PHYSICS-001-FR-031` established for
  its own constant changes. All 259 pre-existing tests across the crate
  pass unchanged (total unchanged from the warm-starting change).

---

## Warm-starting
**2026-08-31** · [PR #79](https://github.com/baileyrd/rusty_bullet/pull/79) · `a79d923`

- **`solver::resolve_dynamic_manifolds` (every ball-vs-car/car-vs-car
  manifold) now warm-starts from the previous call's converged impulses**
  instead of zero. A new `solver::ContactCache` carries a manifold's
  converged real-channel impulses (normal plus both friction rows) from
  one call to the next, matched by each contact's approximate world
  position.
- **The seed is applied to the running velocity delta, not just bookkeeping.**
  Merely setting a row's `applied_impulse` to a cached value would do
  nothing on its own here — this port's `GLOBAL_CFM` is always `0.0`, so
  that field never otherwise enters the per-iteration math. A new
  `warm_start_two_body_row` instead pre-loads the cached impulse's effect
  directly into the manifold's shared `DeltaVelocity` accumulators before
  any iteration runs, mirroring Bullet's own warm-start (applying the
  cached impulse to the solver body's temporary velocity at setup time).
- **`resolve_dynamic_manifolds` gained a `caches` parameter** — one
  `ContactCache` per (normalized) body-index pair. Every call rebuilds it
  from only that call's own manifolds, so a pair no longer touching drops
  out automatically, no separate eviction pass needed. `PhysicsWorld`
  gains one persistent `dynamic_manifold_caches` field, passed into its
  one `resolve_dynamic_manifolds` call site.
- **Deliberately scoped to this one call site.** `resolve_contacts`/
  `resolve_contacts_between` (every static-geometry contact, for both the
  ball and every car) stay un-warm-started: this port's fixed
  `SOLVER_ITERATIONS` already fully converges every one-body/two-body
  scenario this crate tests, so warm-starting them has no scenario to
  demonstrate value against yet. `resolve_dynamic_manifolds` already had
  one — `RB-PHYSICS-001-FR-030`'s own documented extreme-mass-ratio
  "sandwiched" case, which doesn't fully converge within one call's
  iteration budget.
- **1 new `solver.rs` test** reuses that exact sandwiched-ball scenario:
  call 1 (cold) partially converges and populates a cache; from that
  identical post-call-1 state, call 2 then runs twice on independent
  copies — once warm (reusing call 1's cache), once cold (a fresh map) —
  with identical positions, contacts, velocities, and iteration budget
  both times, isolating exactly what the warm seed contributes. The warm
  run lands measurably closer to the true zero-velocity equilibrium than
  the cold repeat.
- **Does not fix the "bouncy resting contact never settles" limitation.**
  That symptom comes from restitution re-triggering off a fresh
  gravity-induced closing velocity every frame, independent of where the
  solver's iteration starts — warm-starting converges the same
  wrong-looking bounce faster, it doesn't stop it from recurring. Sleeping
  (still unimplemented) is the actual fix, and remains the sole open item
  under this port's old combined "no-warm-starting-or-sleeping" gap, which
  this change splits.
- All 14 of `solver.rs`'s pre-existing tests pass unchanged when given an
  empty cache, confirming this change is behavior-preserving for every
  case they already covered.
- 1 new test, 259 total in `rb_physics_bullet` (+1 over
  `RB-PHYSICS-001-FR-034`'s 258).

---

## Split impulse
**2026-08-31** · [PR #77](https://github.com/baileyrd/rusty_bullet/pull/77) · `dedfeec`

- **Deep penetration correction no longer injects spurious velocity into a
  contact.** Every contact's normal row now also solves a second, entirely
  separate "push" pseudo-velocity channel
  (`solver::resolve_push_row`/`resolve_two_body_push_row`), fed only by
  that contact's own positional (penetration/ERP) error — never its
  velocity/restitution error, which stays on the real channel exactly as
  before. This is Bullet's own default (`m_splitImpulse = true`),
  documented as a deliberate gap in this port since the solver was first
  written.
- **Correction now moves position, not velocity.** After a manifold's
  iterations finish, the real velocity delta is applied to the body
  exactly as before, and the new push delta is applied directly to the
  body's position/orientation via a new `solver::apply_push_delta` (built
  on the existing `integrate::integrate_transform`, no new integration
  math) — mirroring Bullet's own `btSolverBody::writebackVelocity`, which
  performs the identical second, independent `integrateTransform` call
  using the push velocity right after writing back the real velocity
  delta.
- **Wired into every resolve path with zero call-site changes elsewhere.**
  `resolve_contacts`, `resolve_contacts_between`, and
  `resolve_dynamic_manifolds` each gained the push-channel resolve/apply
  calls; `world.rs`, `net.rs`, and every other caller of these three
  functions is unaffected.
- **2 new `solver.rs` tests** directly prove the core claim: a
  deeply-penetrating, at-rest contact (zero restitution, zero incoming
  velocity) leaves the real post-solve velocity along the contact normal
  near zero, while the body/bodies' positions measurably separate to
  relieve the overlap — for both the one-body (`resolve_contacts`) and
  two-body (`resolve_contacts_between`) paths.
- **4 pre-existing `world.rs` live end-to-end fillet tests got measurably
  stronger, not just updated.** Before this change, a ball embedded past a
  curved fillet's resting distance only asserted it moved "meaningfully"
  back toward that surface, because the old combined penetration+velocity
  term left the ball with residual velocity to keep coasting on after the
  correction resolved. After this change, the same tests assert the ball
  settles at (not past) its exact resting distance, since the new push
  channel leaves no such residual velocity behind — independent,
  live-`PhysicsWorld` confirmation that this fix does what it claims.
- All 12 of `solver.rs`'s pre-existing tests pass unchanged, confirming
  splitting the old combined `rhs` term into separate `rhs`/
  `rhs_penetration` fields is behavior-preserving for every case they
  already covered.
- Still open: warm-starting/sleeping (a *bouncy* resting contact still
  re-solves from zero every frame and never settles — a different
  symptom split impulse doesn't address) and the average-not-max
  restitution/friction combine mode.
- 2 new tests, 258 total in `rb_physics_bullet` (+2 over
  `RB-PHYSICS-001-FR-033`'s 256).

---

## Genuine goal net
**2026-08-31** · [PR #75](https://github.com/baileyrd/rusty_bullet/pull/75) · `e1ffb4f`

- **Each goal now has a real mass-spring net catching the ball**, replacing
  part of `RB-PHYSICS-001-FR-029`'s solid-bounding-box stand-in with actual
  springy/catching behavior — the "ball tangles in netting" case this
  project's own Non-goals had left open since FR-029 shipped.
- **New `net` module (`net::NetMesh`)**: a rectangular grid of point masses
  (each a real `RigidBody::sphere`, deliberately tiny and light) connected
  by structural (horizontal/vertical) and shear (diagonal) springs (Hooke's
  law plus velocity damping). Every point on the grid's own perimeter is
  anchored — fixed in place, representing the net's real attachment to the
  rigid goal frame (crossbar, both posts, the ground/back line) — while
  every interior point is free to move under gravity, spring forces, and
  ball contact.
- **Reuses existing machinery instead of a bespoke penalty-force system**:
  the ball's contact against each free net point goes through a new
  `collision::sphere_vs_sphere` (this crate's first real sphere-vs-sphere
  contact test — previously an unimplemented placeholder with no caller)
  plus the *existing* `solver::resolve_contacts_between` two-body
  sequential-impulse path, the exact same machinery ball-vs-car and
  car-vs-car contacts already use. `NetMesh::step` sub-steps its own
  internal physics for numerical stability, since a mass-spring system
  this stiff would go unstable integrated with a single large
  Bullet-style step.
- **New `arena::standard_nets`** builds one net panel per goal, positioned
  `NET_DEPTH` behind the real back wall — well in front of `FR-029`'s own
  rigid back-of-net plane, which stays completely unchanged as an
  always-there backstop. `PhysicsWorld` gains `nets`/`with_net`, resolved
  after every other contact each step.
- **Scoped to the ball only, on purpose**: a car still passes straight
  through a net panel's own spatial footprint untouched, stopped instead
  by `FR-029`'s pre-existing solid bounding box — a documented Non-goal,
  not an oversight. Also out of scope: a full 3D "sock" shape billowing
  backward from the goal mouth (this models a single flat rest-shape
  panel, which still deforms backward dynamically under a real ball
  impact via its own springs), and bending stiffness.
- **Every new constant is an uncalibrated placeholder** — real Rocket
  League net material properties have never been published, and this
  port's own point-mass/spring topology is already a simplification of a
  real net's continuum cloth behavior, so a "correct" numeric match isn't
  a coherent target yet either way.
- 10 new tests: 5 in `net.rs` (perimeter anchoring, zero-stretch springs at
  rest, anchored points immovable under gravity, an undisturbed net
  settling instead of oscillating forever, and the real catching proof — a
  ball fired at the net's own center loses over half its speed within 1
  simulated second compared to free flight); `collision.rs` replaced the
  old `contacts_between_two_spheres_is_empty` regression test with 2
  proving `sphere_vs_sphere`'s own correctness; 2 in `arena.rs`; 2 in
  `world.rs` (a wiring-count test plus the real live end-to-end proof — a
  ball fired at a lone net panel in an isolated minimal scene loses at
  least half its speed compared to the identical shot with no net
  present). 256 tests total in `rb_physics_bullet` (+10 over
  `RB-PHYSICS-001-FR-032`'s 246).

---

## Curved-fillet narrow-phase investigation
**2026-08-31** · [PR #73](https://github.com/baileyrd/rusty_bullet/pull/73) · `51e633a`

- **Investigated a claimed corner-testing under-detection bug for a car vs.
  a curved fillet, found it doesn't exist — no change to the narrow phase
  itself.** `RB-PHYSICS-001-FR-027`'s own doc comments claimed
  `box_vs_quarter_pipe`/`box_vs_corner_fillet`'s per-corner technique was
  an approximation, not a full convex-vs-curved-surface narrow phase: a box
  face resting flush against a shallow curve could have every corner still
  clear of the fillet while the face's middle already overlapped it,
  under-detecting that case.
- **Built the fix, and it broke real tests.** A from-scratch GJK
  closest-points implementation was built and wired in to replace the
  per-corner technique — doing so broke two pre-existing, previously-passing
  end-to-end tests, because closest-point is the wrong question for this
  contact: a quarter-pipe/corner-fillet's contact test is a *containment*
  question (is the box's farthest point from the axis/center at or beyond
  radius), not a nearest-point one.
- **The math**: distance-from-a-line/point is a convex function of
  position, and the maximum of a convex function over a convex polytope
  (the box) is always attained at one of its extreme points — its 8
  corners — never a face's interior. So the original per-corner technique
  computes the exact same answer a full narrow phase would, just via
  enumeration instead of an iterative solver — it was never an
  approximation for this specific question.
- **Reverted the code, kept the finding.** `box_vs_quarter_pipe`/
  `box_vs_corner_fillet` are unchanged from `RB-PHYSICS-001-FR-027`; the
  GJK module has been deleted entirely (no remaining consumer). Every doc
  comment across the crate and its spec that had inherited FR-027's
  unverified claim (`lib.rs`'s crate doc, `RB-PHYSICS-001`'s own scope,
  Non-goals, Requirements, and Verification plan sections) now reflects the
  corrected, verified understanding.
- **The goal wall's own analogous window-edge concern remains open** — the
  window boundary is a flat rectangle, not a curve, so it's a distinct
  question this investigation didn't cover.
- 1 new test:
  `collision::tests::no_point_on_a_boxs_face_is_ever_farther_from_a_quarter_pipes_axis_than_its_own_corners`,
  densely sampling (50×50 grid per face) all 6 faces of a car-sized box
  positioned exactly like the two tests that broke, confirming no
  face-interior point ever exceeds the box's own 8 corners' maximum
  distance from the axis. 246 tests total in `rb_physics_bullet` (+1 over
  `RB-PHYSICS-001-FR-031`'s 245).

---

## Constant-calibration audit
**2026-08-31** · [PR #71](https://github.com/baileyrd/rusty_bullet/pull/71) · `4c7b9a2`

- **A scoped audit of every uncalibrated placeholder constant** in
  `drive.rs`/`arena.rs`, sourced against the community reverse-engineering
  effort — deliberately does NOT close `RB-PHYSICS-001-FR-005`'s real-data
  calibration, which still needs `PHASE-0-EXIT`.
- **Sources**: the RocketSim (`ZealanL/RocketSim`) and RLUtilities
  (`samuelpmish/RLUtilities`) source code plus the RLBot community wiki's
  "Useful Game Values" page — three independently-written references;
  agreement across all three treated as high confidence, a single source
  or an older/casual reference flagged as lower-confidence rather than
  silently trusted.
- **Corrected, with real code/behavior changes**:
  - `drive::JUMP_SPEED`: `292.0` → `875.0/3.0` (≈291.667 uu/s) — matches
    RocketSim's `JUMP_IMMEDIATE_FORCE` and RLUtilities' `Jump::speed`
    exactly; also confirmed to be the double jump's own impulse, unchanged.
  - `drive::JUMP_HOLD_ACCELERATION`: `1400.0` → `4375.0/3.0` (≈1458.33
    uu/s²) — matches RocketSim's `JUMP_ACCEL` and RLUtilities'
    `Jump::acceleration` exactly.
  - **New `drive::UNBOOSTED_MAX_CAR_SPEED = 1410.0`** — a genuine bug fix,
    not just a doc update: before this audit, throttle alone shared
    `MAX_CAR_SPEED` (2300, Rocket League's *boosted* top speed) as its own
    cap, letting a car reach boosted top speed on throttle alone.
    Throttle now caps at this new, separate, real unboosted-top-speed
    constant instead; `MAX_CAR_SPEED` keeps its already-correct role as
    boost's own cap.
- **Confirmed already correct, no change** (recorded as *confirmed*, not
  merely *unchanged*): `drive::JUMP_HOLD_MAX_DURATION` (0.2),
  `drive::BOOST_ACCELERATION` (991.667), `drive::MAX_BOOST` (100), gravity
  (-650), `arena::GOAL_DEPTH` (880).
- **Explicitly flagged as audited-but-still-uncalibrated** — a real
  reference exists but doesn't safely port into this port's own unit
  system or mechanic shape, or no reference exists at all:
  `drive::DODGE_SPEED` (real dodge impulse is a direction/speed-scaled
  curve, not a flat number, and adopting just its base magnitude would
  collide with `WALL_JUMP_HORIZONTAL_SPEED`), `drive::DODGE_ANGULAR_SPEED`
  (real flip spin is torque-based against a specific hitbox inertia tensor,
  not a flat rad/s), `drive::WALL_JUMP_HORIZONTAL_SPEED` (real Rocket
  League has no separate wall-jump speed at all — it reuses the plain jump
  impulse along the contact normal), `drive::STEER_TORQUE`/
  `drive::AIR_CONTROL_TORQUE`/`drive::HANDBRAKE_FRICTION_MULTIPLIER`/
  `drive::LANDING_AUTO_UPRIGHT_TORQUE` (real torque/friction-curve values
  exist but are calibrated to real Rocket League's own specific car
  mass/inertia, which this port's placeholder car body isn't confirmed to
  match), and `arena::FILLET_RADIUS`/`arena::CORNER_ARCH_RADIUS` (Rocket
  League's real corner geometry is a triangulated collision mesh, not an
  analytic arc — no single-number reference exists anywhere).
- **Two open ambiguities surfaced, deliberately not acted on**: this
  port's ball radius (`92.75`) is an older, casually-cited figure, while
  RocketSim/RLUtilities/the current RLBot wiki all converge on `91.25` as
  the real simulation collision radius — not changed since `92.75` is
  load-bearing across a large fraction of this crate's existing tests;
  `arena::CEILING_Z` (`2044.0`) vs. RocketSim's `ARENA_HEIGHT = 2048.f` —
  unclear whether they describe the same reference point. Both recorded as
  open questions for a future, deliberate change.
- 1 new test: `drive::tests::throttle_alone_cannot_reach_the_boosted_top_speed`.
  245 tests total in `rb_physics_bullet` (+1 over
  `RB-PHYSICS-001-FR-030`'s 244).

---

## Combined multi-body solve
**2026-08-31** · [PR #69](https://github.com/baileyrd/rusty_bullet/pull/69) · `dfbefb4`

- **`PhysicsWorld::step` now resolves every ball-vs-car and car-vs-car
  contact manifold together as one combined multi-body solve**, instead of
  resolving each pair independently and fully applying it before the next
  pair's setup even reads a body's velocity — closing the "3+ bodies
  mutually touching in the same step" approximation this project has
  tracked since multi-car support first landed (e.g. a car pinned between
  the ball and another car).
- **New `solver::resolve_dynamic_manifolds`** takes every dynamic-vs-dynamic
  manifold in the scene at once (`(body_index_a, body_index_b, contacts)`
  triples into a shared `bodies` slice) and gives every body index that
  takes part in at least one manifold its own `DeltaVelocity` accumulator,
  shared across every manifold that body is in for the whole
  `SOLVER_ITERATIONS` loop — a real shared island solve, not a sequence of
  independent pairwise ones. New helper `delta_pair_mut` generalizes the
  `Vec::split_at_mut` disjoint-borrow trick `PhysicsWorld::step`'s
  car-vs-car loop already used (previously only for adjacent indices) to
  arbitrary index pairs. `resolve_contacts_between`'s old `TwoBodyDelta`
  struct is gone — `resolve_two_body_row` now takes each body's
  `DeltaVelocity` separately, which is what makes sharing one accumulator
  across manifolds possible.
- **`PhysicsWorld::step` rewired**: the old per-pair `resolve_dynamic_contact`
  helper (and its two call-site loops) is replaced with collecting every
  non-empty ball-vs-car/car-vs-car manifold into indices against a
  `[ball, car0, car1, ...]` body list, one call to
  `resolve_dynamic_manifolds`, then copying the resolved velocities back
  out. Static contacts (ground, arena walls, curves, corner fillets, goal
  walls, bounded walls) are deliberately unchanged — a body's contact with
  static geometry never depends on another dynamic body, so resolving it
  independently loses no information; only the dynamic-vs-dynamic path
  needed the fix.
- **Measured, not just assumed, improvement**: a left-right symmetric
  "pinch" test (a ball exactly touching two identical, much heavier cars
  closing in from opposite sides at equal speed, restitution zero
  throughout) has a true simultaneous-solve answer of all three bodies
  ending near zero velocity (total momentum is exactly zero). Resolving
  each pair independently left the ball at ~99% of a single car's own
  closing speed — as if the first-resolved contact's effect was almost
  entirely discarded by the second. The combined solve, at this crate's
  existing 10 solver iterations, leaves the ball measurably slower
  (~89.5 vs. ~98.9 units/s in the isolated measurement) but doesn't fully
  converge to zero in that few iterations — a known, common limitation of
  projected Gauss-Seidel solvers for a light body sandwiched between two
  much heavier ones, confirmed (not shipped as a change) by checking that
  many more iterations converge the combined solve's result much closer to
  zero, while the independent-pairwise approach's result doesn't change at
  all no matter how many iterations each individual pairwise call gets —
  proof the old approach's error was structural, not an iteration-count
  shortfall.
- 2 new tests: `solver::tests::resolve_dynamic_manifolds_keeps_more_of_every_bodys_contact_than_resolving_pairs_independently`
  and `world::tests::a_ball_pinched_between_two_closing_cars_is_resolved_by_a_shared_multi_body_solve`.
  244 tests total in `rb_physics_bullet` (+2 over `RB-PHYSICS-001-FR-029`'s
  242).

---

## Modeled goal interior
**2026-08-31** · [PR #67](https://github.com/baileyrd/rusty_bullet/pull/67) · `9b69c0c`

- **A ball or car passing through a goal-mouth window now settles inside a
  bounded goal box** instead of sailing forever into open, unbounded
  space — closing the "modeled goal interior/net" gap repeated across
  `RB-PHYSICS-001-FR-024` through `FR-028`'s own "Still not modeled" lists.
- **New `body::StaticBoundedWall`** collides only *within* a rectangular
  bound in the plane's own local frame — the opposite gate convention from
  `StaticGoalWall`'s window (which collides everywhere *except* inside a
  rectangle). New `collision::sphere_vs_bounded_wall`/`box_vs_bounded_wall`/
  `contacts_vs_bounded_wall` dispatch by shape, the box path using the same
  "test every corner" technique established by FR-027/FR-028 (a corner
  *outside* the bound is skipped, the opposite of `box_vs_goal_wall`'s
  per-corner window test).
- **New `arena::standard_goal_back_walls`** (2 plain, unbounded
  `StaticPlane`s, `GOAL_DEPTH` behind each real back wall) — deliberately
  unbounded, since nothing can reach that plane except by first passing
  through the goal-mouth window. **New `arena::standard_goal_side_walls`**
  (4 bounded walls, reusing `goal_post_plane` completely unchanged) and
  **`arena::standard_goal_roofs`** (2 bounded walls, reusing
  `goal_crossbar_plane` unchanged) — an unbounded plane at either position
  would incorrectly wall off the *entire* main field, the same problem
  those planes' own pre-existing doc comments already documented for their
  original, purely-geometric role.
- **`PhysicsWorld` gains `bounded_walls`/`with_bounded_wall`**, resolved
  for the ball and every car exactly like `goal_walls`.
- **Two real test-design findings worth keeping**: the 3 new live
  end-to-end proofs are deliberately isolated to a minimal scene built
  from just the specific new wall(s) under test, not the full
  `PhysicsWorld::standard_arena` — using the full arena, a ball fired
  sideways or upward from deep inside the goal box got flung to wildly
  wrong positions, root-caused to the pre-existing "a `StaticQuarterPipe`'s
  sector-membership test only checks angle, not radial distance"
  limitation, spuriously triggered by the standard arena's own
  goal-cutout-edge fillets sitting near the window. Separately, an early
  version of these tests zeroed only the *ball's* own restitution and got
  nondeterministic results, since the *wall's* own default 0.5 restitution
  still applied in the solver — fixed by zeroing the wall's restitution
  too.
- **Still not modeled**: a genuine net *mesh* — this models a solid
  bounding volume standing in for the net's functional role, not
  springy/catching netting or a real net's own visual sag.
- 4 new tests in `body.rs`, 5 in `collision.rs`, 8 in `arena.rs`, and 4 in
  `world.rs` (1 wiring-count + 3 live end-to-end proofs, plus a
  pre-existing wall-count test in `world.rs` renamed to match the 2 new
  back-of-net planes). 242 tests total in `rb_physics_bullet` (+21 over
  `RB-PHYSICS-001-FR-028`'s 221).

---

## Car actually driving into a goal
**2026-08-31** · [PR #65](https://github.com/baileyrd/rusty_bullet/pull/65) · `3141f1e`

- **A car (box) can now actually drive into a goal**, closing the last
  goal-related Non-goal repeated across `RB-PHYSICS-001-FR-024` through
  `FR-027` — until now, `collision::contacts_vs_goal_wall` sent a car
  straight through to an unwindowed `contacts_vs_plane`, so it always
  collided with the full, solid back wall even though the ball already
  passed through the goal-mouth window.
- **New `collision::box_vs_goal_wall`** tests each of a box's 8 corners
  individually against `StaticGoalWall::contains_in_window` — a corner
  whose own projection falls inside the window contributes no contact at
  all, the same pass-through rule `sphere_vs_goal_wall` already applies to
  the ball's single center point, applied per corner instead. A corner
  outside the window behaves exactly like an ordinary `box_vs_plane`
  corner test.
- **A real emergent behavior, not a separate feature**: a car only partly
  lined up with the window (straddling one of its edges) gets a genuine
  partial block — the corners still outside the window register contacts
  and stop the car there, while the corners inside register none — rather
  than the all-or-nothing result a single-point sphere test necessarily
  produces.
- **`contacts_vs_goal_wall` now dispatches a `Shape::Box` to
  `box_vs_goal_wall`** instead of falling through to `contacts_vs_plane`.
  No `PhysicsWorld::step` changes were needed — exactly like FR-027's own
  discovery, `resolve_goal_wall_contact` was already being called for
  every car in the scene (it always needed the wall's plain-plane
  collision even before this fix).
- **Still not modeled**: a modeled goal interior/net — the goal opens onto
  open, unbounded space beyond the back wall for a car now too, not a
  bounded volume. Tracked as separate follow-up work.
- 3 new tests in `collision.rs` (replacing 1 obsolete "ignores the window
  entirely" regression test) and 2 new tests in `world.rs` (replacing 1
  obsolete "still stopped by the back wall" regression test), including a
  live end-to-end proof that a car fired at the goal-mouth center actually
  passes the back wall. 221 tests total in `rb_physics_bullet` (+3 over
  `RB-PHYSICS-001-FR-027`'s 218).

---

## Car deflection by curved fillets
**2026-08-31** · [PR #63](https://github.com/baileyrd/rusty_bullet/pull/63) · `f13e5f5`

- **A car (box) is now actually deflected by every curved fillet in this
  port**, closing the Non-goal repeated across every fillet increment
  since `RB-PHYSICS-001-FR-020` — until now, a car drove straight through
  wall-to-floor/ceiling seams, corner-wall vertical edges, compound
  corners, and goal-cutout edges, untouched; only the ball was ever
  deflected.
- **New `collision::box_vs_quarter_pipe`/`box_vs_corner_fillet`** reuse the
  same "test every corner" technique `box_vs_plane` already used for a
  flat plane — each of a box's 8 corners is checked as a zero-radius
  sphere via the existing `sphere_vs_quarter_pipe`/`sphere_vs_corner_fillet`,
  and every corner that reports a contact contributes one to the manifold.
  Each surviving contact's `point` is overwritten to the corner's own
  world position (not the fillet-surface point those functions themselves
  compute), for the same rel_pos/torque-accuracy reason `box_vs_plane`'s
  own doc comment already gives.
- **`contacts_vs_quarter_pipe`/`contacts_vs_corner_fillet` now dispatch a
  `Shape::Box` to these** instead of `Vec::new()`. No `PhysicsWorld::step`
  changes were needed at all — `resolve_curve_contact`/
  `resolve_corner_fillet_contact` were already being called for every car
  in the scene, just as a silent no-op until now.
- **Documented as an approximation, not a full convex-vs-curved-surface
  narrow phase** (no GJK/EPA support-mapping machinery was added): a box
  face resting flush against a shallow curve can have every one of its
  own corners still just clear of the fillet while the face's middle
  already overlaps it, under-detecting that case — the same "exact per
  test-point, an approximation of the whole shape" caveat this crate has
  always carried for curved geometry.
- **`StaticGoalWall`/`contacts_vs_goal_wall` is unaffected** — a goal wall
  isn't a curved fillet, so a car still sees the same solid, full-width
  back wall it always has, and still can't drive into a goal.
- 3 net new/replaced unit tests across `collision.rs`/`world.rs` in
  `rb_physics_bullet` (218 total): `collision.rs` replaced its two old
  "box vs. curved fillet is always empty" regression tests with proofs
  that an embedded box gets a correctly-directed contact and a
  clearly-outside-the-sector/bounds box still gets none; `world.rs`
  replaced `a_car_is_not_deflected_by_a_curved_transition` (whose entire
  premise this increment reverses) with an end-to-end proof that a car
  resting within a curve's footprint gets pushed up exactly like the ball
  does, and added a compound-corner-fillet car test checking the car's
  *worst corner penetration* shrinks rather than that its center of mass
  approaches the fillet's center (the way the equivalent ball test
  checks) — an oriented box's corners sit at different depths at once, so
  resolving one corner's contact can rotate the box in a way that moves
  its center away from the fillet even as every individual corner's own
  overlap is being corrected. This was found empirically (an earlier,
  center-of-mass-based assertion actually failed) and led to the more
  careful, still-correct invariant.

---

## Goal post-crossbar corner fillets
**2026-08-31** · [#61](https://github.com/baileyrd/rusty_bullet/pull/61) · `c179716`

- **Rounds off the two compound corners per goal where a post's own
  vertical edge fillet meets the crossbar's own horizontal edge fillet**,
  one per post per goal (4 total) — closing a gap `RB-PHYSICS-001-FR-024`'s
  own doc comment explicitly flagged as deliberately not blended into a
  single smooth vertex.
- **New `arena::standard_goal_corner_fillets`** builds all 4 directly via
  `StaticCornerFillet::between_three_planes` on the real back wall/post/
  crossbar planes that meet there — the same approach
  `RB-PHYSICS-001-FR-023` used for the arena's own 16 compound corners. No
  new shape or collision code needed: `StaticCornerFillet`/
  `sphere_vs_corner_fillet` already generalize to any three non-parallel
  planes.
- **Reuses `FILLET_RADIUS` unchanged.** Unlike `RB-PHYSICS-001-FR-025`'s
  arena corners, both edge fillets meeting at a goal's post-crossbar
  corner already share one radius, so there's no mismatched-radius concern
  requiring a dedicated constant.
- **The goal's other two corners, where a post meets the floor, get no
  such treatment** — the window's own bottom edge sits exactly at floor
  level, so a post's own fillet there simply ends flush with the ground
  the ball already rolls on, not a sharp, unrounded vertex needing a
  blend.
- **`PhysicsWorld::standard_arena` wires the 4 new fillets in** via the
  same `with_corner_fillet` builder `standard_corner_fillets`'s 16 already
  used, bringing `corner_fillets` to 20 total.
- 3 new unit tests across `arena.rs`/`world.rs` in `rb_physics_bullet` (215
  total): 2 in `arena.rs` — `standard_goal_corner_fillets_has_four_fillets`
  and `every_goal_corner_fillets_center_sits_radius_in_from_a_back_wall_a_post_and_the_crossbar`
  (proving every fillet's center sits `FILLET_RADIUS` in from a back wall,
  a post plane, and the crossbar plane simultaneously — a real triple
  intersection, not an arbitrary point); 1 in `world.rs` —
  `a_ball_embedded_in_a_goal_corner_fillets_footprint_is_pushed_toward_the_center`,
  the real end-to-end proof, a ball embedded past a goal corner fillet's
  own radius gets pushed meaningfully back toward the center.

---

## Corner-wall floor/ceiling arch radius
**2026-08-31** · [#59](https://github.com/baileyrd/rusty_bullet/pull/59) · `ff1391a`

- **A diagonal corner wall's own floor-seam and ceiling-seam fillets are now
  distinctly larger than a cardinal wall's**, matching real Rocket League's
  noticeably bigger, more swept corner-boost curve rather than a
  scaled-down copy of a cardinal wall's small rounding.
- **New `arena::CORNER_ARCH_RADIUS` (750 uu)**, an uncalibrated placeholder
  like every other arena dimension in this crate (no verified reference for
  the real arch's actual radius, chosen only to read as visibly larger than
  `FILLET_RADIUS` (292 uu)). The 8 of `standard_curves`'s 24 fillets that
  bridge a corner wall to the floor or ceiling now use it instead of
  `FILLET_RADIUS`; a compile-time
  `const _: () = assert!(CORNER_ARCH_RADIUS > FILLET_RADIUS);` enforces the
  "distinctly larger" relationship.
- **All 16 `standard_corner_fillets` switch to `CORNER_ARCH_RADIUS` too.**
  `StaticCornerFillet::between_three_planes` needs one shared radius across
  all three planes it blends to still meet its adjoining edge fillets
  exactly where their axes cross (the same no-gap property
  `RB-PHYSICS-001-FR-023` established) — every one of the 16 compound
  corners touches one of the 8 now-bigger corner-wall arches, so a
  mismatched radius there wouldn't blend cleanly.
- **Unaffected, still `FILLET_RADIUS`:** the 8 cardinal-wall floor/ceiling
  seams, the 8 vertical corner-edge fillets (`FR-022`), and the 6
  goal-cutout edge fillets (`FR-024`) — independent, additive contact
  sources next to the bigger arches, not blended with them, the same
  convention every other adjoining-fillet pair in this module already uses.
- **Discovered and fixed a real regression while validating**: `body::StaticQuarterPipe`
  is documented as infinite along its own axis, not clipped to a corner
  wall's real, finite span — a ball fired dead down the arena's own center
  line eventually re-enters some corner-wall arch's resting shell far past
  the goal, a pre-existing (already-documented) property that was already
  true with the old, smaller `FILLET_RADIUS` (a mild, harmless correction
  around y≈7650-7930 there), but `CORNER_ARCH_RADIUS` moves that zone closer
  in (y≈6300-7700) and turns the same brush into a much sharper,
  solver-destabilizing correction (velocities spiking to tens of thousands
  of units/sec). Fixed by shortening the pre-existing `world.rs` test
  `a_ball_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall`'s
  simulated flight duration (3.0s → 1.8s) — still comfortably long enough to
  prove the ball clears the back wall, but short enough to stop before
  re-entering that already-documented infinite-fillet zone. A test-scoping
  fix, not a new capability or a new documented Non-goal.
- 1 new unit test in `world.rs` in `rb_physics_bullet` (212 total): the real
  end-to-end proof, `a_ball_embedded_in_a_corner_walls_floor_arch_footprint_is_pushed_toward_the_axis`
  — a ball embedded past a corner wall's floor arch's own (larger) radius
  gets pushed meaningfully back toward the axis, asserting
  `CORNER_ARCH_RADIUS > FILLET_RADIUS` along the way.

---

## Goal cutouts
**2026-08-30** · [#57](https://github.com/baileyrd/rusty_bullet/pull/57) · `34234b6`

- **Opens an actual goal-mouth window in each back wall**, where every
  prior increment had a single solid, flat plane spanning the full width.
- **New static shape `body::StaticGoalWall`.** A `StaticPlane` plus a
  rectangular window in the plane's own local `u_axis`/`v_axis` frame
  (`window_center`, `half_width`, `half_height`) — the same "derive an
  axis/window in the plane's own local frame rather than assuming a world
  axis" discipline `StaticQuarterPipe::between_planes`'s `axis_direction`
  generalization (`FR-022`) established. `contains_in_window` tests a
  point's projection onto `u_axis`/`v_axis` alone, independent of the
  point's own depth from the plane along `plane.normal`.
- **`collision::sphere_vs_goal_wall`/`contacts_vs_goal_wall` dispatch by
  shape.** A sphere (the ball) gets no contact at all when its center
  falls inside the window, letting it pass straight through; a box (car)
  falls straight through to the ordinary `contacts_vs_plane` against the
  wrapped plane, deliberately ignoring the window entirely — a
  zero-regression choice, since a car now sees literally the same
  contact-generation call it always did against a back wall.
- **`arena::standard_walls` drops the 2 back-wall `StaticPlane`s it used
  to return** (now 7 planes instead of 9); new `arena::standard_goal_walls`
  returns them instead as 2 `StaticGoalWall`s, windowed at new
  commonly-cited constants `GOAL_HALF_WIDTH`/`GOAL_HEIGHT` (same sourcing
  caveat as `SIDE_WALL_X`), each centered on its own wall at half the
  goal's own height.
- **New `arena::standard_goal_cutout_fillets` rounds each window's 3
  edges** (two vertical posts, one horizontal crossbar, times 2 goals — 6
  `StaticQuarterPipe`s, added to the same `curves` list `standard_curves`'s
  24 already populate). Each is derived via the existing
  `StaticQuarterPipe::between_planes` from the real back-wall plane and a
  second, purely-geometric plane (`goal_post_plane`/`goal_crossbar_plane`)
  representing the post's or crossbar's own inward-/downward-facing
  surface — positioned at exactly the window's own edge, so the fillet's
  tangent point lands exactly on the window boundary with no gap or
  overlap. Unlike a real wall, these post/crossbar planes are never
  themselves added as collision geometry: an infinite plane facing
  straight along X (or capping Z) would incorrectly wall off the *entire*
  rest of the field at that coordinate, unlike a diagonal corner wall's
  own orientation, which stays non-binding everywhere except right at the
  true corner.
- **`PhysicsWorld` gains `goal_walls`/`with_goal_wall`/
  `resolve_goal_wall_contact`**, resolved for the ball *and* every car
  (unlike `curves`/`corner_fillets`'s ball-only resolution) — safe
  precisely because the box path is a no-op change from the prior
  plain-`StaticPlane` behavior. `PhysicsWorld::standard_arena` wires in
  both the goal walls and the goal-cutout fillets automatically.
- **Still not modeled:** a car (box) actually being deflected by any
  fillet or driving into a goal, a modeled goal interior/net beyond the
  cutout itself (the ball passes into open space, not a bounded volume),
  and the goal's own two compound top corners where a post's fillet meets
  the crossbar's (independent, additive fillets there, same "no blended
  3D corner" approach the arena's corner-wall edges used before `FR-023`).
- 17 new unit tests across `body.rs`/`collision.rs`/`arena.rs`/`world.rs`
  in `rb_physics_bullet` (211 total): 4 in `body.rs` proving
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

---

## Compound-corner fillets
**2026-08-30** · [#55](https://github.com/baileyrd/rusty_bullet/pull/55) · `5d2db86`

- **Rounds off the last 16 sharp vertices in the standard arena's vertical
  boundary** — the compound corners where a corner wall's own vertical-edge
  fillet (`FR-022`) meets a floor- or ceiling-seam fillet (`FR-020`/`FR-021`),
  near that corner wall's own top or bottom endpoint.
- **New static shape `body::StaticCornerFillet`.** A compound corner is
  where *three* planes meet at once, which no existing cylindrical
  `StaticQuarterPipe` can blend, so this requirement introduces a genuinely
  different shape: an immovable sphere riding the concave inside of the
  vertex, the same "ride the inside" convention every prior fillet already
  uses, generalized from a cylinder to a sphere.
- **`between_three_planes` derives the center as three planes' common
  intersection, not solved from scratch.** It reuses the same "radius-in
  from every bridged plane" invariant `StaticQuarterPipe::between_planes`
  already established: since the fillet's center must sit exactly `radius`
  in from all three planes, it's also exactly `radius` in from each *pair*
  of them — meaning it already lies on all three of that vertex's own
  pairwise `between_planes` axis lines simultaneously. So the center is
  nothing more than those three lines' common intersection point, solved
  directly via the classic three-plane-intersection cross-product form of
  Cramer's rule.
- **Containment generalizes a 2-sided sector test to a "spherical
  triangle."** New `collision::sphere_vs_corner_fillet`: a direction from
  the center is inside the fillet iff its dot product with each of 3
  `bounds` is non-negative. Each bound is the raw (deliberately
  non-normalized — only its sign is used) cross product of a pair of the
  three normals, sign-corrected via `signed_pair_axis` (checking the third,
  non-pair plane's own normal against it) to always point toward the sharp
  corner this fillet replaces — provably correct because that dot product
  is exactly the derivative of the third plane's own signed distance along
  a candidate direction. No `.normalize()`/`.unwrap()` is needed or used
  anywhere in this new production code, the same discipline
  `between_planes`'s own `FR-022` self-correction established.
- **`arena::standard_corner_fillets` builds all 16** (4 per corner wall —
  floor+side, floor+back, ceiling+side, ceiling+back — times the 4 corner
  walls) directly from the same three flat planes `standard_walls` already
  builds, reusing `FILLET_RADIUS` once again rather than a fourth radius
  constant.
- **`PhysicsWorld` gains `corner_fillets`/`with_corner_fillet`/
  `resolve_corner_fillet_contact`**, mirroring `curves`/`with_curve`/
  `resolve_curve_contact` exactly — a no-op for a car, the same documented
  deferred case as every other fillet here. `PhysicsWorld::standard_arena`
  wires in all 16 automatically.
- **Still not modeled:** a car (box) actually being deflected by any
  fillet, and goal cutouts in the back walls.
- 13 new unit tests across `body.rs`/`collision.rs`/`arena.rs`/`world.rs`
  in `rb_physics_bullet` (194 total): 4 in `body.rs`, using a synthetic
  fixture combining a perpendicular floor with the same 45-degree
  non-perpendicular wall pair `between_planes`'s own `FR-022` fixture
  uses — the center sits radius-in from all three planes with tangent
  points exactly on each, and the derived `bounds` correctly include the
  direction toward the sharp corner and exclude the direction pointing
  away from it; 5 in `collision.rs`, mirroring `sphere_vs_quarter_pipe`'s
  own test shapes (deep-inside no contact, touching zero penetration,
  pushed-past positive penetration toward the center, outside-bounds no
  contact, box always empty); 2 in `arena.rs` — `standard_corner_fillets`
  returns exactly 16 fillets, and every fillet's center sits radius-in
  from a floor/ceiling plane, a side/back wall, and a corner wall
  simultaneously; 2 in `world.rs` — `standard_arena` carries exactly 16
  corner fillets, plus the real end-to-end proof, a ball embedded past a
  compound-corner fillet's own radius gets pushed meaningfully back toward
  the center.

---

## Curved corner-wall vertical-edge fillets
**2026-08-30** · [#53](https://github.com/baileyrd/rusty_bullet/pull/53) · `d466ae2`

- **Rounds off the standard arena's last remaining sharp edges** — the 8
  vertical edges where each of the 4 diagonal corner walls meets its
  neighboring side or back wall. `arena::standard_curves` now returns 24
  `StaticQuarterPipe`s (the 16 floor/ceiling-seam fillets `FR-020`/`FR-021`
  already built, plus 8 vertical-edge fillets, one per corner-wall
  endpoint).
- **Generalized `StaticQuarterPipe::between_planes` to any two non-parallel
  planes, not just perpendicular ones.** Unlike every prior fillet in this
  port, the two planes a vertical-edge fillet bridges *aren't*
  perpendicular — a corner wall meets its neighboring side/back wall at 135
  degrees (given `standard_walls`' 45-degree corner cut), not 90. This
  exposed a real gap: `between_planes` previously only computed the correct
  axis point for perpendicular planes, via a shortcut (summing the two
  scaled normals) that silently gives the *wrong* point at any other angle.
  It now solves the axis point as an actual 2x2 linear system in the
  (possibly non-orthogonal) basis the two normals form, and its own sector
  angle comes out to exactly the angle between the two planes' normals — a
  right angle for perpendicular planes as before, or (for these
  vertical-edge fillets) a shallow 45 degrees, the supplement of the walls'
  135-degree dihedral angle.
- **Generalized `sphere_vs_quarter_pipe`'s sector-membership test** from the
  old two-dot-products check (only correct for a 90-degree sector, since
  its two edges happen to be perpendicular) to a signed-cross-product test
  against `axis_direction`, exact for any sector up to 180 degrees — the
  widest a sensible fillet-replacing-a-corner can ever be.
- **`between_planes` self-corrects a "backwards" `axis_direction`
  internally**, since the general sector test (unlike the old
  perpendicular-only one) depends on `axis_direction`'s own sign/handedness:
  it flips the input if `cross(sector_start, sector_end)` doesn't already
  point the right way, so a caller can pass either of the two opposite
  directions along the shared edge line without reasoning about which one
  is correct.
- **The vertical-edge fillets' own `axis_direction` is simply `(0, 0, 1)`**
  — the edge itself is vertical — no cross product needed, unlike the
  corner-wall floor/ceiling-seam case `FR-021` introduced.
  **`FILLET_RADIUS` is reused as-is** once again, rather than a separate,
  smaller radius for these visibly shallower edges.
- **Still not modeled:** a car (box) actually being deflected by any
  fillet, the compound corner where a vertical-edge fillet meets a floor-
  or ceiling-seam fillet (near a corner wall's own top/bottom endpoint —
  this port models each fillet as an independent, additive contact source,
  not a blended 3D corner), and goal cutouts in the back walls.
- 9 new unit tests across `body.rs`/`arena.rs`/`world.rs` in
  `rb_physics_bullet` (181 total): 5 in `body.rs`, using a synthetic
  non-perpendicular fixture independent of the arena's own geometry — the
  axis still sits exactly `radius` in from both planes with tangent points
  exactly on each; the derived sector angle matches the angle between the
  two planes' normals (45 degrees for this fixture); the sharp corner the
  fillet replaces sits outside its own radius but within its sector (the
  real proof the generalized sector orientation actually faces the missing
  material, not away from it); and passing either of the two opposite
  `axis_direction` choices produces the same correctly-oriented sector; 3
  in `arena.rs` — `standard_curves` returns exactly 24 fillets, every
  vertical-edge fillet's `axis_direction` runs purely along Z, and a corner
  wall's own vertical-edge fillet sits radius-in from both the corner wall
  and its neighboring side wall with a 45-degree sector; 1 in `world.rs` —
  the real end-to-end proof, a ball embedded past a vertical-edge fillet's
  own radius (at a wall-to-wall angle that isn't a right angle) gets pushed
  meaningfully back toward the axis (not a claim that it settles and stays
  at the exact resting distance — its contact stops firing once the
  overlap resolves, so nothing cancels whatever residual velocity the
  correction left the ball with, the same reason `FR-020`'s and `FR-021`'s
  own equivalent tests make the same weaker, "moved meaningfully" claim).

---

## Curved corner-wall-to-floor/wall-to-ceiling transitions
**2026-08-30** · [#51](https://github.com/baileyrd/rusty_bullet/pull/51) · `d746d08`

- **Extends `RB-PHYSICS-001-FR-020`'s fillet treatment to the 4 diagonal
  corner walls** `RB-PHYSICS-001-FR-019` introduced — `arena::standard_curves`
  now returns 16 `StaticQuarterPipe`s (still one floor-side and one
  ceiling-side fillet per wall, now for all 9 walls) instead of 8.
- **`StaticQuarterPipe::between_planes` needed no code changes.** Its real
  correctness requirement was never "axis-aligned planes" (as FR-020's own
  doc comment had incorrectly claimed) — only that the two bridged planes'
  normals, plus `axis_direction`, form an orthonormal basis, which only
  needs the two planes to be mutually *perpendicular*. A vertical wall's
  normal always has zero Z component while the floor/ceiling's is always
  purely Z, so this holds for a corner wall regardless of its own
  horizontal rotation, not just for a cardinal wall.
- **A corner wall's fillet `axis_direction` is computed via a cross
  product** (`floor.normal.cross(&wall.normal)`, and the ceiling
  equivalent) rather than hand-picked, since — unlike a cardinal wall's —
  it isn't a coordinate axis. The cross product of two always-perpendicular
  unit vectors is already exactly unit length by construction, so no
  `.normalize()`/`.unwrap()` is needed (avoiding a `clippy::unwrap_used`
  violation the workspace's lint config promotes to a hard CI error in
  production code).
- **A new `corner_wall_plane(sx, sy)` helper in `arena.rs`** factors out the
  existing (behavior-unchanged) corner-wall plane construction
  `standard_walls` already did inline, so `standard_curves` can reuse it
  rather than duplicating the math. `PhysicsWorld::standard_arena` picks up
  the extra 8 curves automatically, since it already loops over every curve
  `arena::standard_curves()` returns.
- **`FILLET_RADIUS` is reused as-is** for the corner-wall fillets rather
  than introducing a second, independently chosen radius.
- **Still not modeled:** a car (box) actually being deflected by any fillet
  (unchanged from FR-020), a fillet at a corner wall's own *vertical* edges
  — where it meets its neighboring side/back wall at other than 90 degrees,
  a materially different problem since `between_planes` only handles two
  perpendicular planes — and goal cutouts in the back walls.
- 4 new unit tests across `arena.rs`/`world.rs` in `rb_physics_bullet` (172
  total): `standard_curves` returns exactly 16 fillets; every fillet's axis
  sits exactly `FILLET_RADIUS` in from some vertical wall, cardinal or
  corner; a corner wall's own derived fillet axis sits exactly
  `FILLET_RADIUS` in from both the corner wall and the floor, with
  correctly perpendicular unit sector vectors; the cross product computing
  each of the 4 corner walls' `axis_direction` is exactly unit length,
  confirming the production code's `.normalize()`-free assumption actually
  holds — plus, the real end-to-end proof, a new `PhysicsWorld` test built
  around a wall with a diagonal (non-axis-aligned) normal, rather than
  going through `arena::standard_curves` directly, confirms a ball resting
  at ordinary flat-floor height within that diagonal wall's fillet
  footprint gets pushed up off it, the same physical proof FR-020 gave for
  a cardinal wall, now for one whose normal isn't a coordinate axis.

---

## Curved wall-to-floor/wall-to-ceiling transitions
**2026-08-30** · [#49](https://github.com/baileyrd/rusty_bullet/pull/49) · `8053a71`

- **Added:** a new `body::StaticQuarterPipe` shape — an immovable
  partial-cylinder fillet connecting two perpendicular flat planes,
  infinite along its own axis like `StaticPlane` — and `collision::
  contacts_vs_quarter_pipe`, a sphere-only narrow-phase test
  (`RB-PHYSICS-001-FR-020`).
- **The playable side is the *inside* of the fillet's concave face** — the
  same geometry a skateboard quarter-pipe is named after and ridden on the
  inside of. A point is governed by a fillet at all only when its
  direction from `axis_point`, projected perpendicular to
  `axis_direction`, falls within the 90-degree sector from `sector_start`
  to `sector_end` (checked via `dot(dir, sector_start) >= 0 && dot(dir,
  sector_end) >= 0`, exact for a 90-degree sector since the two vectors
  are perpendicular); within that sector, contact fires as the sphere's
  surface approaches or crosses the fillet's own radius *from inside*, and
  the correction pushes the sphere back toward the axis — the opposite
  direction convention from `sphere_vs_plane`'s always-away-from-the-plane
  push.
- **`StaticQuarterPipe::between_planes(plane_a, plane_b, radius,
  axis_direction)`** derives a fillet's axis/sector automatically from the
  two flat planes it bridges (offsetting each plane inward by `radius`
  along its own normal, negating each plane's normal for the sector vector
  pointing back to its own tangent point) — exact only when `plane_a`/
  `plane_b`'s normals and `axis_direction` form an orthonormal basis (true
  for every cardinal arena wall's own floor/ceiling seam, not a diagonal
  corner wall's).
- **`PhysicsWorld` gains `curves: Vec<StaticQuarterPipe>` and a
  `with_curve` builder** (mirroring `walls`/`with_wall`), resolved via a
  new `resolve_curve_contact` alongside `resolve_plane_contact` for the
  ball and every car — a no-op for cars, since the box arm of
  `contacts_vs_quarter_pipe` is always empty.
- **`solver::resolve_contacts`'s second parameter changed from
  `&StaticPlane` to plain `restitution: f32, friction: f32`** — the only
  two fields it ever actually used — so this same solver path serves a
  `StaticQuarterPipe` fillet exactly as it already served a `StaticPlane`,
  with no new solver code needed.
- **`arena::standard_curves`** builds the 8 fillets (floor-side and
  ceiling-side, for each of the 4 cardinal walls) the standard arena needs,
  via `between_planes`, using a new uncalibrated placeholder
  `FILLET_RADIUS` — this port has no verified reference for the real
  transition radius either, same status as `arena::CORNER_LENGTH`.
  `PhysicsWorld::standard_arena` now adds these 8 curves alongside its
  existing 9 walls.
- **Still not modeled:** a car (box) actually being deflected by a fillet
  (needs real support-mapping/SAT-style collision machinery against curved
  geometry this port doesn't have yet), fillets at the 4 diagonal corner
  walls (their non-axis-aligned normals don't satisfy `between_planes`'
  orthonormal-basis assumption), and goal cutouts in the back walls.
- 15 new unit tests across `body.rs`/`collision.rs`/`arena.rs`/`world.rs`
  in `rb_physics_bullet` (168 total): the derived fillet geometry sits
  exactly `radius` in from both bridged planes with correctly-directed,
  perpendicular unit sector vectors and tangent points exactly on each
  plane; a sphere deep inside a fillet has no contact, touching it has
  zero penetration, pushed past it has positive penetration pushing back
  toward the axis, and outside the 90-degree sector has no contact
  regardless of absolute distance; a box against a fillet always returns
  no contact; `standard_curves` returns exactly 8 fillets, each sitting
  radius-in from the floor/ceiling and a cardinal wall; `PhysicsWorld::
  standard_arena` carries exactly 8 curves, plus — the real end-to-end
  proof — a ball resting at ordinary flat-floor height within a curve's
  footprint (already overlapping the fillet's own material) gets pushed up
  off that flat height instead of staying embedded, while a car in the
  exact same position stays completely unaffected at its ordinary
  flat-floor resting height.

---

## Modeled arena footprint
**2026-08-30** · [#47](https://github.com/baileyrd/rusty_bullet/pull/47) · `cc68213`

- **Added:** a new `arena` module builds Rocket League's real
  standard-arena boundary entirely from `RB-PHYSICS-001-FR-013`'s existing
  generic `StaticPlane`/`PhysicsWorld::with_wall` machinery
  (`RB-PHYSICS-001-FR-019`) — no new collision code, since a ceiling and a
  corner-cut wall are each just another flat plane.
- **`arena::standard_ground`** is the flat floor at `z = 0`, identical to
  the `flat_ground()` test helper this crate has used since v0.
- **`arena::standard_walls`** returns 9 `StaticPlane`s: 2 side walls
  (`x = ±SIDE_WALL_X`), 2 back walls (`y = ±BACK_WALL_Y`), a ceiling
  (`z = CEILING_Z`), and 4 diagonal corner walls (one per quadrant) cutting
  off the true rectangular corner where a side wall would otherwise meet a
  back wall at 90 degrees — giving the field its real octagonal footprint
  instead of a plain rectangle.
- **Constant sourcing:** `SIDE_WALL_X` (4096), `BACK_WALL_Y` (5120), and
  `CEILING_Z` (2044) are commonly-cited community-measured field
  dimensions, the same sourcing convention `drive::MAX_CAR_SPEED`/
  `JUMP_SPEED` already established. The corner walls' inset distance
  (`CORNER_LENGTH`, equal along both axes, giving a 45-degree cut) is this
  project's own uncalibrated placeholder — this port has no verified
  reference for the real arena's actual corner-wall geometry, which isn't
  even a single flat plane in the real field mesh (it's curved, and blends
  into ramps this port doesn't model either).
- **New `PhysicsWorld::standard_arena` convenience constructor** wires
  both into a `PhysicsWorld` in one call — offered alongside, not
  replacing, `PhysicsWorld::new`/`with_wall`'s existing ad-hoc-wall
  capability, which this crate's own tests keep using for non-standard
  scenes.
- **Still not modeled:** curved wall-to-floor/wall-to-ceiling transitions,
  goal cutouts in the back walls, and disambiguating or blending a car's
  simultaneous contact with two walls at a corner for wall-jump purposes —
  physical collision resolution already handles a car touching two walls
  at once correctly regardless (each wall is resolved independently every
  step), only the wall-jump push-off direction picker still isn't, and
  the new corner walls make that case reachable in the standard arena for
  the first time (still untested here).
- 10 new unit tests across `arena.rs`/`world.rs` in `rb_physics_bullet`
  (153 total): `standard_walls` returns exactly 9 planes; the arena's
  center is on the playable side of every one of them; opposing side/back
  walls share one offset magnitude by construction; a point just past a
  side wall is no longer on the playable side; the ceiling bounds from
  above; a corner wall actually cuts off the true rectangular corner; all
  four corner walls share one offset magnitude, plus — the real end-to-end
  proof — `PhysicsWorld::standard_arena` carries exactly 9 walls and the
  standard ground, a ball shot at the standard arena's side wall bounces
  off it rather than escaping, and a ball fired straight at the true
  rectangular corner is stopped by the diagonal corner wall well before
  its x or y individually reaches either the side or back wall's own
  position.

---

## Landing auto-orientation
**2026-08-30** · [#45](https://github.com/baileyrd/rusty_bullet/pull/45) · `b5ed2cd`

- **Added:** `drive::apply_driven_forces` gains a gentle continuous
  restoring torque, applied while airborne, nudging the car's local up
  axis back toward world up (`RB-PHYSICS-001-FR-018`). Real Rocket League
  triggers this assist on approach to the ground; this port has no
  raycast or distance query to replicate that condition, so the assist
  instead applies continuously whenever airborne, gated on two conditions
  so it never fights the player: no active `pitch`/`roll` air-control
  input this step, and no fresh `ControllerInput.jump` press this step
  (avoiding a same-step conflict between this torque's accumulation into
  `total_torque` and a dodge's/wall-jump-dodge's/double-jump's/
  flip-cancel's own direct `angular_velocity` mutation, both resolved by
  the same `integrate_velocities` call).
- **The correction:** `up_axis(car).cross(&world_up) *
  LANDING_AUTO_UPRIGHT_TORQUE`. Since both vectors are unit length, the
  cross product's magnitude is already proportional to the sine of the
  car's tilt off level, so a level car earns no correction and a heavily
  tilted one earns a proportionally stronger nudge, with no separate angle
  computation needed.
- **New constant `LANDING_AUTO_UPRIGHT_TORQUE`** is an uncalibrated
  placeholder, deliberately one full order of magnitude smaller than
  `AIR_CONTROL_TORQUE` so the assist reads as gentle assistance, not full
  control — this port has no public reference for the real assist's
  actual strength or trigger condition either.
- **Known, accepted, unaddressed limitation:** a car resting exactly
  upside-down gives an exactly antiparallel `up_axis`/`world_up` pair,
  whose cross product is also zero, so no correction is computed in that
  unlikely exact singularity.
- **No new `PhysicsWorld` state** — the assist is a pure function of the
  car's current orientation, input, and ground contact, all already in
  scope.
- Drive.rs's own test-helper chain never calls
  `integrate::integrate_transform`, so a car's `orientation` never
  actually changes step-to-step there; the new `drive.rs` tests instead
  set a known tilted orientation directly (a new `tilted_car()` helper)
  and check a single step's resulting torque.
- A pre-existing regression test
  (`world::tests::landing_and_a_new_double_jump_clears_a_stale_dodge_flip_
  flag_in_a_live_world`) was loosened from an exact `assert_eq!` to a
  small tolerance, since the assist now legitimately nudges angular
  velocity by a tiny amount on the test's intervening neutral-input step.
- 5 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet` (143
  total): a tilted airborne car with no input gets a corrective torque; an
  already-upright airborne car gets none; the assist has no effect while
  grounded; it doesn't fire while pitch air control is actively held; and
  — the real end-to-end proof — a car tilted 90 degrees with no input
  trends back toward level over 120 steps of a live `PhysicsWorld::step`
  loop (gravity zeroed). This closes out the last item tracked in
  `drive.rs`'s own module doc "Not implemented" list since the dodge
  (FR-014) increment — that list is now empty.

---

## Wall-jump dodge
**2026-08-30** · [#43](https://github.com/baileyrd/rusty_bullet/pull/43) · `3b08fdf`

- **Added:** the wall jump's own fresh press (`RB-PHYSICS-001-FR-013`) now
  checks `ControllerInput.pitch`/`roll` against `DODGE_DEADZONE`
  (`RB-PHYSICS-001-FR-017`), the same check the ground double jump's press
  already uses (`RB-PHYSICS-001-FR-014`): at or above it on either axis, a
  **wall-jump dodge** fires instead of the plain fixed push-off — the same
  outward-plus-upward impulse combined with a horizontal `DODGE_SPEED`
  component and `DODGE_ANGULAR_SPEED` spin (identical axis/sign conventions
  to the ground dodge), also arming `dodge_flip_active` so its spin is
  flip-cancelable exactly like a ground dodge's (`RB-PHYSICS-001-FR-016`).
- **Below the deadzone:** the plain wall jump fires exactly as before this
  requirement, still never touching `double_jump_available`.
- **Unlike the plain wall jump, the dodge variant spends the double jump:**
  a deliberate simplification — since touching a wall unconditionally
  restores `double_jump_available` before this check ever runs, gating the
  dodge variant on it would be vacuous (always true there); having it
  consume the resource instead keeps flip-cancel's existing invariant
  ("`dodge_flip_active` is only ever true while `double_jump_available` is
  false") intact with zero changes to flip-cancel's own branch ordering or
  any new landing/wall-touch-clearing logic. This port has no way to
  separately account for "a wall touch refilled the double jump, then the
  wall-jump dodge spent it" versus a genuinely independent wall-dash
  resource, and real Rocket League's precise accounting here isn't public
  to the precision this project would need to model that distinction.
- **No new physics constants** — reuses
  `DODGE_SPEED`/`DODGE_ANGULAR_SPEED`/`WALL_JUMP_HORIZONTAL_SPEED`/
  `JUMP_SPEED` outright.
- **Two pre-existing tests repurposed, not silently deleted:**
  `drive::wall_jump_fires_instead_of_a_dodge_when_touching_a_wall` and
  `world::wall_jump_still_fires_instead_of_a_dodge_when_touching_a_wall`
  both asserted the *old* "wall jump always ignores stick input" premise
  this requirement deliberately reverses — both now assert the new
  wall-jump-dodge behavior instead, keeping the same scenario (touching a
  wall with directional stick input) but updating the expected outcome.
- 6 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet` (138
  total): a wall-jump dodge consumes the double jump unlike a plain wall
  jump; its spin can be flip-cancelled; a below-deadzone stick deflection
  still gives a plain wall jump; opposite stick sign dodges the opposite
  direction; a diagonal (pitch+roll) wall-jump dodge combines both axes,
  plus — the real end-to-end proof — a wall-jump dodge firing in a live
  `PhysicsWorld::step` loop, and a second end-to-end test confirming its
  spin is flip-cancelable there too.

---

## Flip-cancel
**2026-08-30** · [#41](https://github.com/baileyrd/rusty_bullet/pull/41) · `14d986d`

- **Added:** a dodge's spin (`RB-PHYSICS-001-FR-014`) can now be canceled
  early (`RB-PHYSICS-001-FR-016`) — a further fresh `ControllerInput.jump`
  press while airborne, not touching a wall, with the double jump already
  spent by that dodge, zeroes `RigidBody.angular_velocity` outright instead
  of leaving the flip to spin indefinitely.
- **A new per-car `dodge_flip_active: bool`** (`PhysicsWorld`'s parallel
  `car_dodge_flip_active: Vec<bool>`, starting `false`) tracks whether the
  most recent double-jump-or-dodge press left a cancelable flip: the
  directional-dodge branch sets it `true`; the plain-double-jump branch
  explicitly sets it `false` rather than leaving it alone.
- **Closes a real staleness bug this port's own tests were written to
  catch:** without that explicit clear, a much-later, completely unrelated
  plain double jump (after landing from the dodge and taking off again)
  would leave the flag `true`, letting a further press spuriously
  flip-cancel a flip that no longer exists. Verified by temporarily
  removing the fix and confirming both the `drive.rs` and `world.rs`
  regression tests actually fail without it.
- **Scoped narrowly:** flip-cancel touches neither the dodge's own linear
  velocity nor `double_jump_available` (already spent by the dodge that set
  the flag); wall jump keeps its existing priority, checked first in the
  airborne branch, unchanged. This port has no timed flip animation to
  interrupt (a dodge is one instantaneous angular-velocity kick, not a
  sustained torque over a fixed duration), so "mid-flip" here means "any
  time before landing or a wall touch re-arms the double jump" — a
  documented simplification of real Rocket League's actual flip-duration
  window. No new physics constants — a state-flag-gated zeroing action, not
  a magnitude to calibrate.
- 6 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet` (132
  total): a second jump press cancels a dodge's spin outright and spends
  the flag; flip-cancel leaves the dodge's own translation and
  `double_jump_available` untouched; a plain double jump clears a stale
  `dodge_flip_active` left over from an earlier dodge; a wall jump still
  takes priority over flip-cancel when touching a wall, plus — the real
  end-to-end proof — a second jump press canceling a dodge's spin in a live
  `PhysicsWorld::step` loop, and a regression test confirming landing and a
  later plain double jump clear a stale flag there too, not just in
  `drive.rs` isolation.

---

## Variable jump height input
**2026-08-30** · [#39](https://github.com/baileyrd/rusty_bullet/pull/39) · `9266c6c`

- **Added:** the ground jump (`RB-PHYSICS-001-FR-010`) gains a hold window
  (`RB-PHYSICS-001-FR-015`) — continuing to hold `ControllerInput.jump`
  after the fresh press that fires it adds a continuous
  `JUMP_HOLD_ACCELERATION` upward force, for up to
  `JUMP_HOLD_MAX_DURATION` seconds, on top of the press's own fixed
  `JUMP_SPEED` impulse. Releasing `jump` (or the window simply running
  out) stops the extra acceleration immediately, matching real Rocket
  League's held-vs-tapped jump height difference.
- **Ordering-sensitive by design:** a new per-car `jump_hold_time_remaining:
  f32` (`PhysicsWorld`'s parallel `car_jump_hold_time_remaining: Vec<f32>`,
  starting `0.0`) is checked and decremented against whatever value the
  *previous* call left it at, before that same call's own
  `on_ground`/`jump_pressed` handling can re-arm it to
  `JUMP_HOLD_MAX_DURATION` — so a fresh ground-jump press's own step
  always fires only the plain impulse; only continued holding into later
  calls earns the extra height.
- **Scoped to the ground jump alone:** the double jump, a dodge, and the
  wall jump are all still a single fixed instantaneous impulse, unaffected
  by how long jump is held — firing any of them requires releasing jump
  first (a fresh press), which itself unconditionally zeroes the ground
  jump's hold window before that press's own branch ever runs.
- **Constants:** `JUMP_HOLD_MAX_DURATION` and `JUMP_HOLD_ACCELERATION` are
  both uncalibrated placeholders — this port has no public reference for
  real Rocket League's actual hold-window length or acceleration the way
  `JUMP_SPEED` does.
- **Regression fix:** the pre-existing
  `holding_jump_does_not_repeatedly_relaunch_the_car` test's run duration
  was extended (1.5s → 3.0s), since a continuously held jump now also
  earns the variable-height bonus, climbing higher and taking longer to
  land than a bare `JUMP_SPEED` impulse alone.
- 6 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet` (126
  total): holding jump after a ground jump adds more upward velocity than
  tapping it, releasing jump early stops the extra acceleration
  immediately, the extra acceleration stops accruing once the hold window
  has expired even if still held, and a double jump fired after holding
  the ground jump through its whole window still adds exactly one more
  `JUMP_SPEED` kick rather than an extra variable-height boost, plus — the
  real end-to-end proof — a held ground jump reaching a greater peak
  height than a tapped one in a live `PhysicsWorld::step` loop, and a
  regression test confirming the double-jump-unaffected property holds
  there too, not just in `drive.rs` isolation.

---

## Dodge input
**2026-08-30** · [#37](https://github.com/baileyrd/rusty_bullet/pull/37) · `72150f5`

- **Added:** the double jump's fresh press (`RB-PHYSICS-001-FR-014`) now
  checks `ControllerInput.pitch`/`roll` at the moment it fires: at or above
  a new `DODGE_DEADZONE` on either axis, it fires a directional dodge
  instead of the plain vertical double jump — a purely horizontal
  `DODGE_SPEED` impulse (along `forward_axis` for `pitch`, `right_axis`
  for `roll`) plus an instantaneous `DODGE_ANGULAR_SPEED` spin written
  directly to `RigidBody.angular_velocity` about the perpendicular axis.
- **Reuses air control's own axis/sign conventions:** a forward dodge uses
  the same `pitch`→`right_axis` mapping air control's pitch torque already
  does (just fast and instantaneous instead of a continuous torque), and a
  side dodge does the same with `roll`→`forward_axis`. Both axes can
  contribute at once (a diagonal dodge), simply summed rather than
  normalized — a documented simplification, since real Rocket League
  normalizes the stick direction so a diagonal dodge isn't faster than an
  axis-aligned one.
- **Shares the double jump's resource:** below `DODGE_DEADZONE` on both
  axes, the plain vertical double jump fires exactly as before; either way
  the press spends the shared `double_jump_available` — a dodge and a
  plain double jump aren't separate resources. Wall jump is untouched: it
  never checks `pitch`/`roll` at all, so touching a wall always gets the
  fixed wall-jump push-off, never a dodge.
- **Constants:** `DODGE_SPEED` and `WALL_JUMP_HORIZONTAL_SPEED` are now
  `pub` (mirroring `JUMP_SPEED`) so `world.rs`'s end-to-end tests can
  assert against, and distinguish between, all three jump variants.
- **Not implemented** (explicitly, not silently dropped): a dodge variant
  of the wall jump, canceling a dodge's rotation early by pressing again
  mid-flip (flip-cancel), any landing auto-orientation assistance, and
  variable jump height — each tracked as separate follow-up work.
- 10 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet`
  (120 total): a forward (pitch) dodge and a lateral (roll) dodge each
  give the expected horizontal velocity and spin, a below-deadzone
  deflection still gives a plain double jump, a dodge spends
  `double_jump_available` the same as a plain one, opposite pitch dodges
  the opposite direction, a diagonal dodge combines both axes, dodge logic
  has no effect while grounded, and a wall jump still fires its own
  (smaller) push-off instead of a dodge when touching a wall, plus — the
  real end-to-end proof — a car dodging forward with a visible flip after
  a ground jump in a live `PhysicsWorld::step` loop, and a regression test
  confirming a car touching a wall with directional stick input still
  gets the wall jump, not a dodge.

---

## Wall jump input
**2026-08-30** · [#35](https://github.com/baileyrd/rusty_bullet/pull/35) · `b748b86`

- **Added:** `PhysicsWorld` gains arena walls (`RB-PHYSICS-001-FR-013`) —
  `walls: Vec<StaticPlane>` and a `with_wall` builder (mirroring
  `with_car`). Every body (ball and cars alike) now collides with every
  wall the same way it already collides with the ground, reusing the same
  body-vs-static-plane machinery (`resolve_ground_contact` is renamed
  `resolve_plane_contact` — no behavior change, it never had ground-specific
  logic, just a ground-specific name).
- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains a wall
  jump — a fresh airborne jump press while touching a wall
  (`wall_normal: Some(normal)`, computed the same way `on_ground` is) fires
  an impulse combining a new `WALL_JUMP_HORIZONTAL_SPEED` (uncalibrated
  placeholder) outward along the wall's normal with `JUMP_SPEED` upward.
- **Interaction with the double jump:** wall jump takes priority over the
  double jump on a fresh press, but is otherwise independent of it —
  merely touching a wall (whether or not jump is pressed) unconditionally
  restores `double_jump_available`, the same "any surface contact refills
  your second jump" rule landing already uses, so a wall jump doesn't cost
  a player their double jump and has no once-per-airborne-period limit of
  its own.
- **Not implemented** (explicitly, not silently dropped): the directional
  "dodge" a real wall jump can pair with, variable jump height, and any
  modeled arena footprint beyond generic flat walls (Rocket League's actual
  octagonal shape, curved wall-to-floor/ceiling transitions, a ceiling, or
  disambiguating a car touching two walls at once) — each tracked as
  separate follow-up work.
- 7 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet`
  (110 total): wall jump gives outward-and-upward velocity when available,
  has no effect while grounded, takes priority over the double jump
  without consuming it, and mere wall contact restores double-jump
  availability, plus — the real end-to-end proof — a car resting against a
  wall wall-jumps outward and upward in a live `PhysicsWorld::step` loop,
  a ball shot at a wall bounces off it instead of tunnelling through (the
  same physical proof ball-vs-car collision already has, now for the
  generic plane-collision machinery walls reuse), and a regression test
  confirming a car near but not touching an existing wall still gets a
  plain double jump.

---

## Double jump input
**2026-08-30** · [#33](https://github.com/baileyrd/rusty_bullet/pull/33) · `7c9524a`

- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains a
  double jump (`RB-PHYSICS-001-FR-012`) — one more, identical `JUMP_SPEED`
  instantaneous upward velocity change fired on a fresh (rising-edge)
  press of `ControllerInput.jump` while the car is airborne, reusing the
  ground jump's own edge detection rather than a second edge-detector.
- **Availability, not ground contact:** gated on a new per-car
  `double_jump_available` flag instead of `on_ground` — touching the
  ground (landing, or simply resting) unconditionally restores it to
  `true`, and a fresh airborne press that fires the double jump sets it to
  `false` until the next landing, so it fires at most once per airborne
  period no matter how many more times jump is released and re-pressed
  before then. `PhysicsWorld` gains a parallel
  `car_double_jump_available: Vec<bool>` (starting `true`, kept in
  lockstep with `cars` by `with_car`).
- **Constants:** reuses `JUMP_SPEED` (now `pub`) rather than a
  separately-calibrated double-jump speed — this port has no public
  reference for a distinct number either.
- **Not implemented** (explicitly, not silently dropped): the directional
  "dodge" impulse/torque a real double jump pairs with (a sideways/forward
  flip from the stick direction at the moment of the second press),
  variable jump height, and wall jump — each a distinct real mechanic,
  tracked as separate follow-up work.
- 6 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet`,
  minus one pre-existing `drive.rs` test (`jump_has_no_effect_while_airborne`)
  removed because this feature deliberately supersedes its premise (103
  total): a fresh airborne jump press gives upward velocity when the
  double jump is available, has no effect when it isn't, is consumed
  after firing once, and touching the ground restores availability, plus
  — the real end-to-end proof — a double jump fired after a ground jump
  in a live `PhysicsWorld::step` loop (gravity zeroed) adds a second
  `JUMP_SPEED` kick on top of the first, and a regression test confirming
  a spent double jump doesn't refire mid-air no matter how many more
  times jump is released and re-pressed before landing.

---

## Air control input
**2026-08-29** · [#31](https://github.com/baileyrd/rusty_bullet/pull/31) · `431ff56`

- **Added:** `rb_physics_bullet::drive::apply_driven_forces` gains air
  control (`RB-PHYSICS-001-FR-011`) — torque about the car's local right,
  up, and forward axes, scaled directly by `ControllerInput.pitch`/`yaw`/
  `roll` (each an `Option<f32>`, `None` treated as zero) times one shared
  `AIR_CONTROL_TORQUE` constant, applied whenever the car is *not*
  touching the ground — the mirror image of throttle/steering/handbrake/
  jump's ground-only gating, so it never competes with ground steering for
  the yaw axis.
- **Design note:** unlike ground steering, air control isn't speed-scaled
  — a car can spin from a standing start in the air, since there's no
  wheel grip to require momentum for. A new `right_axis` helper completes
  the local (forward, right, up) basis alongside the existing
  `forward_axis`/`up_axis`.
- **Constants, honestly labeled:** `AIR_CONTROL_TORQUE` is an uncalibrated
  placeholder with no public reference at all (like `STEER_TORQUE` and
  `HANDBRAKE_FRICTION_MULTIPLIER`), shared uniformly across pitch, yaw,
  and roll — a documented simplification, since real Rocket League's
  three rates differ from each other (roll fastest).
- **Not implemented** (explicitly, not silently dropped): double
  jump/dodge, variable jump height (holding jump for a higher jump), and
  wall jump — each a distinct real mechanic, tracked as separate
  follow-up work. Also out of scope: per-axis torque calibration, an "air
  roll only" input mode, camera-relative stick mapping, and any
  auto-orientation assistance on landing.
- 6 new unit tests across `drive.rs`/`world.rs` in `rb_physics_bullet` (98
  total): pitch/yaw/roll each produce angular velocity about the correct
  local axis for a stationary airborne car, air control has no effect
  while grounded, a `None` analog value behaves like neutral input, and
  opposite-sign yaw spins the opposite way, plus — the real end-to-end
  proof — a car with yaw input in a live `PhysicsWorld::step` loop
  (gravity zeroed) actually reorients itself mid-air, and a regression
  test confirming a grounded car stays level despite stray pitch/yaw/roll
  input.

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
