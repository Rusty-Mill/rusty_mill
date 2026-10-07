# Project Status

Current state only. Per-change history lives in
[RELEASE_NOTES.md](../RELEASE_NOTES.md) and [CHANGELOG.md](../CHANGELOG.md);
per-unit status in [ROADMAP.md](./roadmap/ROADMAP.md),
[SPEC-REGISTRY.md](./specifications/SPEC-REGISTRY.md), and
[TRACEABILITY.md](./traceability/TRACEABILITY.md). The long-form
"Completed" log this file used to carry is in git history (last full
version: `a245d35`).

- Last verified main commit: the merge of
  [#257](https://github.com/baileyrd/rusty_bullet/pull/257)
- Verified at: 2026-10-07
- Current milestone: `PHASE-1-PHYSICS-CORE` (In Progress) — mechanics are
  broad and, since the tape bot (ADR-0059, ADR-0064), measured against the real
  game on controlled recordings: driving, jumps, flips, boost, ball bounces,
  car-ball hits and car-car bumps and demolitions agree to a few uu; wall
  climbing, car-body contact from the air, long powerslides and wheels on the
  ball do not (see
  [FIDELITY-SCOREBOARD.md](./research/FIDELITY-SCOREBOARD.md)).
  `RB-PHYSICS-001-FR-005` (real-data constant calibration) has not started
  as a unit.
- Health: green — workspace builds, `fmt`/`clippy`/`test` all pass on `main`;
  47 real recordings gate `cargo test` (ADR-0066)

## Phases

| Phase | Status |
|---|---|
| `PHASE-0-BOOTSTRAP` | Done |
| `PHASE-0-REPLAY-INGEST` | Done |
| `PHASE-0-CAPTURE-INGEST` | Done |
| `PHASE-0-EXIT` | Done |
| `PHASE-1-PHYSICS-CORE-V0` | Done (v0 scope) |
| `PHASE-1-PHYSICS-CORE` | In Progress |
| `PHASE-2-DETERMINISM` | Not Started |
| `PHASE-3-NETCODE` | Not Started |
| `PHASE-4-POLISH` | Not Started |

## Recently completed

- 2026-10-07 (autonomous session, PRs #246 to #257; evidence in
  [FIDELITY-SCOREBOARD.md](./research/FIDELITY-SCOREBOARD.md)):
  `RB-PHYSICS-001-FR-139` (a second jump press within 6 ticks is ignored:
  `speed_flip` 87 -> 0.8 uu, ADR-0065), `FR-140`/`FR-143` (car bumps, ground
  and air), `FR-141` (box-box contacts as `dBoxBox`), `FR-142` (demolition),
  ADR-0068; `rb-verify --recorded-inputs` (`RB-VERIFY-003-FR-016`) and ball
  scoring (`FR-015`), two-car scenarios and a hivemind bot (`FR-018`,
  ADR-0067), the golden-capture gate (`FR-017`, ADR-0066), capture plugin
  1.5, unattended batches (ADR-0064), `RB-RESEARCH-O011` to `O018`.

- [#186](https://github.com/baileyrd/rusty_bullet/pull/186) —
  `RB-PHYSICS-001-FR-094`, flip clock starts after the press tick.
- [#185](https://github.com/baileyrd/rusty_bullet/pull/185) —
  `RB-PHYSICS-001-FR-093`, air control during a flip; `RB-VERIFY-003`
  0.14.0 car-frame spin.
- [#183](https://github.com/baileyrd/rusty_bullet/pull/183) —
  `RB-PHYSICS-001-FR-091`, ground-jump force from the press tick (ADR-0019).
- [#182](https://github.com/baileyrd/rusty_bullet/pull/182) —
  `RB-PHYSICS-001-FR-090`, raycast suspension and hitbox offset (ADR-0018).
- [#181](https://github.com/baileyrd/rusty_bullet/pull/181) —
  `RB-PHYSICS-001-FR-089`, engine force at each wheel along its heading.
- [#180](https://github.com/baileyrd/rusty_bullet/pull/180) —
  `RB-PHYSICS-001-FR-088`, ground contact from wheel rays (ADR-0017).
- [#179](https://github.com/baileyrd/rusty_bullet/pull/179) —
  `RB-PHYSICS-001-FR-087`, car spin clamped after the transform;
  `RB-VERIFY-003` 0.13.0 `q-rate`.
- [#178](https://github.com/baileyrd/rusty_bullet/pull/178) —
  `RB-PHYSICS-001-FR-086`, steering by per-wheel side impulses (ADR-0016).
- [#177](https://github.com/baileyrd/rusty_bullet/pull/177) — spin in
  `--self-trace` (`RB-VERIFY-003` 0.12.0) and `RB-PHYSICS-001-FR-085`,
  flip torque on the press tick.
- [#176](https://github.com/baileyrd/rusty_bullet/pull/176) —
  `RB-PHYSICS-001-FR-084`, RocketSim air control (ADR-0015).
- [#175](https://github.com/baileyrd/rusty_bullet/pull/175) —
  `RB-PHYSICS-001-FR-083`, the flip as RocketSim has it (ADR-0014); flip
  vz stall matches the recording.
- [#174](https://github.com/baileyrd/rusty_bullet/pull/174) —
  `RB-PHYSICS-001-FR-082`, RocketSim stick signs and dodge impulse
  (ADR-0013); real-capture position error at 5.0 s 1,590 to 79 uu.
- [#173](https://github.com/baileyrd/rusty_bullet/pull/173) —
  `RB-PHYSICS-001-FR-081`, per-axis tire grip (ADR-0012); real-capture
  velocity error at 4.0 s ~250 to 9.9 uu/s.
- [#172](https://github.com/baileyrd/rusty_bullet/pull/172) —
  `RB-PHYSICS-001-FR-080`, curve-based steering (ADR-0011); real-capture
  heading error at 4.0 s 0.46 to 0.06 rad.
- [#171](https://github.com/baileyrd/rusty_bullet/pull/171) —
  `RB-PHYSICS-001-FR-079`, restitution threshold in uu; ended the
  driving-car hop.
- [#170](https://github.com/baileyrd/rusty_bullet/pull/170) —
  `RB-VERIFY-003-FR-005`, per-frame trace (`rb-verify --self-trace`).
- [#165](https://github.com/baileyrd/rusty_bullet/pull/165) — per-car drive
  state grouped into `drive::DriveState`; refactor, no behavior change.
- [#164](https://github.com/baileyrd/rusty_bullet/pull/164) — `drive.rs`
  split into `drive/{ground,air,jump,boost}`; `AGENTS.md` crate map fixed.
- [#163](https://github.com/baileyrd/rusty_bullet/pull/163) —
  `RB-VERIFY-003-FR-004`, divergence-growth diagnostic
  (`rb-verify --self-growth`); sanity-checked on the synthetic fixture only.
- [#162](https://github.com/baileyrd/rusty_bullet/pull/162) —
  `RB-PHYSICS-001-FR-077`'s first real-capture fidelity number recorded
  (see Validation).
- [#161](https://github.com/baileyrd/rusty_bullet/pull/161) —
  `RB-PHYSICS-001-FR-078`, car hitbox tests calibrated to
  `CAR_HALF_EXTENTS`.

## In progress

- `RB-PHYSICS-001-FR-095` (air control waits one step after the wheels let
  go): implemented; the 4.15-4.35 s re-trace confirms it (yaw rate holds
  at -2.03 vs -2.02 rad/s through 4.192 s, rotation error 0.00 into and
  through the 4.317 s flip).
- `RB-PHYSICS-001-FR-096` (no air throttle while boosting): implemented;
  the 4.9-5.6 s re-trace confirms it (velocity error 8.0 vs 17.0 uu/s at
  5.175 s, the end of the throttle -1 boost).

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
  replays, see Validation) already closes the "runs correctly on real owner
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

1. `RB-PHYSICS-001-FR-100` (car body uses RocketSim's car-vs-world
   friction and bounce, ADR-0021) is confirmed as an improvement: velocity
   error after the 19.083 s impact 13-27 uu/s (was 35-78). Still open
   there: the candidate's impact is symmetric and 1.46x too strong while
   the recorded car rolls and yaws (possibly the real triangle-mesh
   floor). The side-flip capture (`side.jsonl`) confirms the roll axis:
   first flip tick, mid-flip rate and landing match, rotation error 0.00.
   `RB-PHYSICS-001-FR-101` (jumps along the car's up axis, tires after the
   jump) is confirmed: `test2.jsonl` growth window 4 s 0.64 uu (was 1.27),
   5 s 4.6 uu (was 7.3). The 5.575 s landing was investigated: what is
   left is a few uu/s of lateral tire friction on a tilted two-wheel
   landing that the side-flip capture contradicts; parked.
   `rb-verify --self-onestep` (`RB-VERIFY-003-FR-006`) ranks the model's
   worst single steps. Its first finding, the `test2.jsonl` corner at
   8.9 s (~1,390 uu/s), is fixed by `RB-PHYSICS-001-FR-102` (real mesh
   radii, swept corners, wheel rays on every surface, ADR-0022): `test2`
   mean one-step error 3.1 uu/s (was 15.2). `RB-PHYSICS-001-FR-103`
   (ADR-0023) fixed the next two, keyboard dodges whose forward part
   follows throttle plus the 2300 uu/s car speed cap: `test2` mean 2.72.
   `RB-PHYSICS-001-FR-104` fixed the 15.6-15.8 s stall (yaw against air
   roll): `test2` mean 2.25. `RB-PHYSICS-001-FR-105` (ADR-0024) gave cars
   Bullet's one-corner-a-tick plane contact: `front` 25.967 s 156 (was
   252), 19.083 s 30 (was 55). `RB-PHYSICS-001-FR-106` (ADR-0025) made
   the side ramps and corners the real collision mesh (repository
   GPL-3.0-only until FR-117): `test2` mean 1.78, corner/wall segment 6.0 (was 13.4).
   `RB-PHYSICS-001-FR-107` (ADR-0026) gave car-ball contacts RocketSim's
   material and Psyonix's extra hit velocity: kickoff hit 21 uu/s (was
   155), `test2` mean 1.67; `--self-onestep` now shows the ball too
   (`RB-VERIFY-003-FR-007`). `RB-PHYSICS-001-FR-108` (ADR-0027) folds
   ball-world contacts into one velocity-only contact and drops the
   speculative term, as RocketSim does: `test2` ball mean 0.41 (was 1.00),
   max 252 (was 1072); car mean 1.72 (was 1.67); `front` 0.302.
   `RB-PHYSICS-001-FR-109` (ADR-0028) gives mesh edge contacts Bullet's
   internal-edge adjustment: `test2` ball mean 0.17 (7.95 s 32, was 201).
   `RB-PHYSICS-001-FR-110` (ADR-0029) gives each contact one friction row,
   Bullet's default: `test2` car mean 1.59, ball 0.11; `front` 0.192, and
   free-running `front` stays within 34 uu (was 1026). The 8.958 s corner
   impact and `front` 25.967 s landing are gone from the worst steps.
   Worst steps now: `test2` car 14.408 s (42), 14.292 s (33); ball 7.95 s
   (32), 12.267 s (27). `--self-kstep` (`RB-VERIFY-003-FR-008`, ADR-0030)
   now scores quarter-second predictions: k = 30 baseline `test2` car
   3.29 uu / 26.27 uu/s, ball 0.56 uu; `front` 0.11 / 0.84; `side` 0.10 /
   0.52. Bullet's row order, ERP2 0.8 and turn ERP 0.1 were measured with
   it and not adopted (no gain). `RB-VERIFY-003-FR-009`: snapping now
   takes the recorded boost fuel (`test2` is unlimited-boost freeplay and
   the candidate's tank had run dry): `test2` k = 30 car 0.68 uu / 5.78
   uu/s, one-step car mean 0.90. Worst k = 30 windows now: the 5.5 s
   kickoff (49 uu/s) and the 9.0 s corner impact (45), both hard contacts.
   `RB-PHYSICS-001-FR-111` (ADR-0031) models unlimited boost as a drain
   rate of 0, detected from the capture: `test2` free run 906 uu (was
   1,365), still chaotic after the kickoff. A fourth capture,
   `hitjump.jsonl` (293 s, 25 freeplay resets, 33 jumps, goals), shows
   jumps right after a ball hit match (mean one-step Δvz error ≤ 0.34
   uu/s/tick), so `test2`'s weak kickoff jump is a one-off.
   `RB-PHYSICS-001-FR-112` (ADR-0032) opens the goal mouth (goal fillets
   out, floor seam stops at the posts): `hitjump` one-step car 2.28 uu/s
   (was 23.41). `RB-PHYSICS-001-FR-113` (ADR-0033) makes the goals and
   back walls the real mesh, so the arena is now RLUtilities' exactly:
   `hitjump` one-step car 0.58 uu/s, ball 0.75 (resets excluded; were
   2.06 and 2.35). `RB-PHYSICS-001-FR-114` (ADR-0034) calibrates the
   car-ball contact radius to 92.3 uu from 28 recorded hits: ball 0.62.
   `RB-PHYSICS-001-FR-115` (ADR-0035) keeps ball-mesh contacts as
   Bullet's manifold does (near points merge, a full manifold keeps the
   largest area): ball 0.595. `RB-PHYSICS-001-FR-116` (ADR-0036)
   matches those points on the ball, as Bullet does: ball 0.589.
   `RB-PHYSICS-001-FR-117` (ADR-0037) switches the arena to RocketSim's
   16 mesh files (Apache-2.0; the repository is MIT OR Apache-2.0 again)
   and reports their triangles in Bullet's BVH order: ball 0.513, worst
   frame 208 uu/s (was 988, the crossbar bevel vertex).
   `RB-PHYSICS-001-FR-118` (ADR-0038) pushes ball-world penetration out
   per point at RocketSim's ERP 0.8, measured from its 91.25 uu sphere:
   `test2` k = 30 ball 0.37 uu (was 0.45), `hitjump` flat.
   `RB-PHYSICS-001-FR-119` (ADR-0039) renormalizes the combined
   ball-world normal, so friction no longer picks up part of the bounce:
   `hitjump` one-step ball 0.501, k = 30 ball 20.41 (was 20.88); `test2`
   k = 30 ball 0.33. Worst one-step ball frame: `hitjump` 97.908 s, a
   bounce one tick early (207 uu/s), which Bullet's unported persistent
   manifold would decide. `RB-PHYSICS-001-FR-120` (ADR-0040) classifies
   mesh-seam edges across meshes, removing a sideways kick on the goal
   roof near `x = 0`: `hitjump` 119.908 s 75 uu/s (was 110), k = 30 ball
   20.37. `RB-PHYSICS-001-FR-121` (ADR-0041) keeps the ball's mesh
   manifolds across ticks as Bullet does: the second bounce a tick after
   a corner hit is gone in k = 30 (97.908 s window 206 to 4 uu/s), mean
   20.35; one-step is unchanged by construction, so its worst ball frame
   still reads 97.908 s (207). `RB-PHYSICS-001-FR-122` (ADR-0042)
   calibrates the wheel pushback's ERP to 0.1 from a bottomed-out
   landing: `test2` one-step car 0.470 (was 0.895), `front` 0.178,
   `side` 0.192, `hitjump` car 0.494. `RB-PHYSICS-001-FR-123` (ADR-0043)
   ports RocketSim's auto-flip, the pop and roll a car on its roof gets
   from a jump press: `hitjump` 279.75 s 12.6 uu/s (was 205), its spin
   error on the press tick still 4.1 rad/s (one sample; RocketSim's
   torque ramps where the game's roll is immediate). `RB-PHYSICS-001-FR-124`
   (ADR-0044) rounds the car's hitbox corners by Bullet's 2 uu collision
   margin against mesh facets (sharp against planes, as Bullet): the
   `test2` wall ride at 14.292 s 4.7 uu/s (was 31.7), one-step car 0.447
   (was 0.470), k = 30 car 5.53; `hitjump` car 0.484, its four worst ramp
   contacts under 3 uu/s (were 34–60). `RB-PHYSICS-001-FR-125` (ADR-0045)
   applies the same margin to the car-ball contact and drops FR-114's
   calibrated 92.3 uu radius for RocketSim's 91.25 sphere admitted to the
   1.825 uu breaking threshold: the source now explains all 28 recorded
   hits; `hitjump` hit-frame ball sum 470 (was 821), one-step ball 0.490,
   k = 30 ball 21.29; `test2` k = 30 ball 1.39 (was 1.62).
   `RB-PHYSICS-001-FR-126` (ADR-0046) drops and folds the ball's mesh
   manifold points in Bullet's slot order, found by matching the game's
   per-tick contact set on the 104.4 s fillet grind: k = 30 ball 21.24,
   the 20.7–21.0 s frames from 98–164 uu/s to under 30; with manifolds
   carried through snaps the one-step ball would read 0.464 (97.908 s
   207 → 0) but 20.800 s regresses (2 → 109), left as a lead with the
   cold one-step unchanged. `RB-PHYSICS-001-FR-127`/`FR-128` (ADR-0047)
   then close the lead: the ball's mesh contacts are made, kept and folded
   at Bullet's thresholds (0.005 uu past the contact sphere, not 1.86) and
   a snapped ball keeps its contact history unless it teleported, so
   one-step now predicts from the game's own carried contact set:
   `hitjump` one-step ball 0.435 (was 0.490), frames over 50 uu/s 18 → 1
   (97.908 s 207 → 0), k = 30 ball 21.18; `test2`, `front`, `side`
   unchanged. Largest ball frame was the aerial car hit at 77.667 s (135).
   `RB-PHYSICS-001-FR-129` (ADR-0048) explains it: the game's wheel rays
   hit the ball, so a car passing over a resting ball is pushed up by its
   bottomed-out suspension (car 110 → 8 uu/s, ball 135 → 46, hitjump
   one-step car 0.474, k = 30 car 11.24). `RB-PHYSICS-001-FR-130`
   (ADR-0049) runs each body's lever arm to its own contact point (the
   ball's, not the box's), which lowers every car-ball hit: `hitjump`
   274.217 s ball 47 → 10, 37.4 s 36 → 5, 223.2 s 30 → 8; one-step ball
   0.428, k = 30 ball 20.96; `test2` k = 30 ball 1.27. Frames over 20
   uu/s in `hitjump`: 1 (77.667 s, 58): the ball is 57 uu/s short along
   the car's forward direction (and 11 down) after the wheel-over-ball hit,
   the car within 6. Tried and rejected: the ball's velocity in the wheel
   model (it is at rest), a wheel-impulse reaction on the ball before the
   solve (the downward push bounces off the floor, z error 95) or after it
   (best fit 31 at a free scale of 0.05, direction misfit, spin unchanged),
   so one frame stays open. Open car-side leads: `test2`
   14.408 s (25 uu/s, a one-tick recording glitch in spin and velocity)
   and the slow lateral drift along the wall after it (about 1-2% bias,
   minor). `RB-PHYSICS-001-FR-131` (ADR-0050, auto-roll) fixed the landing
   spin drift. `RB-PHYSICS-001-FR-132` (ADR-0051) then fixed `test2` 5.77-5.92 s: a tire's side force acts along the wheel's own axle (window error 4.87 -> 0.44 uu/s, test2 k = 30 car 4.73 -> 2.55). The largest remaining non-glitch `test2` frames are hard impacts near the (3800, 4150) corner: 8.958 s (19 uu/s) and 9.125 s (19). The direction is exact; the sim's impulse is 2% short (965 against 984 uu/s). The car-world material (`CAR_WORLD_MATERIAL`, RocketSim's 0.3 restitution and 0.3 friction) is at the captures' optimum: restitution 0.2/0.32/0.34/0.4 and friction 0.1/0.28/0.32/0.6 all leave `test2` k = 30 car at 2.55 or worse (to 8.5), and 9.125 s stays at 18-21 uu/s throughout; 8.958 s is a sharp, chaotic frame (4-730 uu/s over the sweep). The miss is not the material. Contact trace at 8.958 s: one point (3870, 4189, 194), normal (-0.708, -0.641, 0.296), depth 5.7 uu, friction saturated at 0.30 of the normal impulse in both recording and sim; the sim's normal impulse is 2% short (924 against 942 uu/s of the car's normal speed change, 1301 uu/s in). That fits a restitution of 0.326 or a 2% effective-mass difference (lever arm at the corner); 9.108 s and 9.125 s do not move together under restitution, which favours geometry over material. Tried: the car's static-contact lever arm to its own surface point (`Contact::point_on_a`, as FR-130 does for two bodies): 8.958 s 19 -> 10.4 uu/s, `test2` k = 30 car 2.55 -> 2.42, but `hitjump` k = 30 10.82 -> 10.90 and `front` 0.49 -> 0.50, 9.125 s unmoved (18.8). Mixed, but adopted afterwards (`RB-PHYSICS-001-FR-135`, ADR-0054) on the principle. The car's spin is clamped at 5.5 rad/s through these impacts, so the recording's spin says nothing about the contact point. `test2` 18.317-18.392 s (the bottomed-out nose landing at 1600 uu/s down, mean one-step error 5.0 uu/s): sim vertical speed is 5-9 uu/s too negative through the bottoming ticks and the roll spin is 0.2-0.4 rad/s short. `PUSHBACK_ERP` is a sharp optimum at 0.1 (0.04-0.2 give window means 6.8-13 and worse `test2` k = 30, 2.58-3.37 against 2.55), and dividing the pushback among the touching wheels instead of four makes it far worse (window 39, `test2` k = 30 3.58). One landing is too little data to separate wheel-order and roll-asymmetry causes; a capture with more hard landings would. `hitjump` 77.667 s ball (a teleported, spin-free car at 2150 uu/s passing over a resting ball with its wheels, ball 71 uu/s off along the car's travel direction, rec 2034 against sim 1962 uu/s of ball speed change): two more tries, both rejected. Measuring the extra hit impulse (`extra_ball_hit_velocity`) from the hitbox centre instead of the car origin raises that frame's error to 163; scaling the whole extra impulse by 1.03/1.06 lowers it to 45/38 but raises 37.4 s from 5 to 38/72, so the frame is not a global hit-strength error. Source audit (2026-10-04) of RocketSim's `Car.cpp` (`_PreTickUpdate`, `_UpdateWheels`, `_UpdateBoost`; gave FR-133, FR-134), `Ball.cpp` (`_OnHit`, velocity caps) and `Arena.cpp` (`Step`, contact callbacks) against the port found no further missing soccar mechanic: boost pads only change the tank, `Ball::_PreTickUpdate` and the car-world/ball hooks are mode-specific or already ported. One difference remains by choice: RocketSim sets `m_erp2` to 0.8 for every contact, the port keeps 0.2 for car-world and car-ball penetration pushes (0.8 only for the ball's, FR-118); trying 0.8 for all raises `test2` k = 30 car 2.12 -> 2.59 and leaves `hitjump` unchanged, so the recordings prefer 0.2. `test2` 9.125 s (18.8 uu/s, sim decelerates 18 uu/s too much along the wall normal) after FR-135: the car slides along the corner wall, hitting two contacts 1.2-1.8 uu clear of their surfaces on the ticks 9.108-9.133 s (a ramp facet, normal z 0.30, and a vertical facet). Vanilla Bullet's speculative allowance (gap/dt of free approach) for cars is far worse (9.108 s 8 -> 77, 9.125 s 19 -> 35, `test2` k = 30 2.12 -> 2.09 but `hitjump` 10.52 -> 10.92), confirming RocketSim's act-as-touching rule for cars as for the ball. Dropping contacts more than 1.6 or 1.2 uu clear fixes 9.125 s (19 -> 8.4) and `test2` k = 30 (1.99) but delays 9.108 s's impact a tick (8 -> 77); at 1.0 uu both go wrong. The impact timing there needs a rule between the two, unfound; one capture is too little to pick it. Car-ball hit parameter sweep (2026-10-06, `RB-RESEARCH-O010`): 648 combinations of car-ball restitution and friction and the extra impulse's scale, z and forward factors. A windowed optimum (extra impulse x0.7, z 0.5) cuts the `hitjump` 77.6-77.8 s car velocity error 51 -> 8 uu/s but doubles `test2` k = 30 ball distance (0.54 -> 1.17) and barely moves k = 30 car velocity (10.60 -> 10.46); rejected, nothing committed. The four-tick hit is structural, not a parameter. It was an ordering bug: `RB-PHYSICS-001-FR-137` (ADR-0062) adds the extra hit velocity after the ball moves, as RocketSim does (`test2` k = 30 ball 0.54 -> 0.17 uu; `hitjump` car 10.60 -> 10.49).

A second pass over `Car.cpp` (2026-10-05: `_UpdateAirTorque`, flip cancel and stall, the flip clock, pitch lock, flip-Z damping, `_BulletSetup`, supersonic) found every constant and rule already in the port (flip torque 260/224, 0.65 s torque, 0.3 s pitch lock, Z damping 0.35 over 0.15-0.21 s, air control 130/95/400 and damping 30/20/50), except the second-jump window (`RB-PHYSICS-001-FR-136`); the port's documented deviations (air control stays on through a flip; no pre-minimum jump scale) come from the owner's captures. Supersonic state only drives demolitions in RocketSim. Not modelled, with no capture to check against: car-car bumps and demolitions, supersonic state, the other game modes.
2. (Optional, owner-side, non-blocking) The manual BakkesMod-overlay
   single-timestamp cross-checks for `RB-VERIFY-001`/`RB-VERIFY-002` (see
   Blocked).

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (523 tests: 27 `rb_domain`, 452
  `rb_physics_bullet`, 14 `rb_replay_ingest` (incl. real-fixture
  integration test), 10 `rb_capture_ingest`, 20 `rb_verify_cli`)
- `cargo run -p rb_replay_ingest --bin corpus_check` (local only, not CI):
  40/40 real owner replays parsed cleanly, 2026-08-28
- `cargo run -p rb_verify_cli --bin rb-verify -- <replay> <capture>`
  (manual, 2026-08-28, default 0.02s timestamp tolerance): `frames
  compared: 6, mean ball distance: 0.25 uu, max ball distance: 0.25 uu,
  car pairs compared: 6, mean car position/rotation/velocity distance:
  2816.42 uu / 2.36 rad / 1307.87 uu/s` against the real replay fixture +
  (now time-aligned) synthetic capture fixture.
- `rb-verify --self-trace test2.jsonl 4.3 5.6` and `--self-growth`
  (owner's machine, 2026-10-01, after `RB-PHYSICS-001-FR-087`): flip
  `q-rate` simulated (0.59, 7.49, 0.32) vs recorded (0.78, 7.55, 0.15);
  orientation error <= 0.15 rad through 5.6 s (was ~1.3 rad by 4.95 s);
  growth 4-5 s 15.7 uu / 0.07 rad / 35 uu/s, 5-6 s 64 uu / 0.24 rad /
  212 uu/s (was 157 uu), ball 30 uu / 191 uu/s (was 82 / 660), 6-7 s
  442 uu / 1.70 rad / 923 uu/s.
- Same, after `RB-PHYSICS-001-FR-088` (wheel-ray grounding), 2026-10-01:
  at 5.758 s the candidate ground-jumps (vz 57 to 344, recorded 228)
  instead of side-dodging, and dodges at 6.058 s with the recording.
  Growth 5-6 s 49 uu / 115 uu/s (was 64 / 212), ball 21 uu (was 30); 4-5 s
  23 uu / 40 uu/s (was 16 / 35): the candidate now keeps tire grip for 3
  ticks after the 4.142 s jump (the recording keeps it for 5), which
  carries the pre-existing 45 uu/s steering error differently; 6-7 s
  517 uu / 2.27 rad (was 442 / 1.70), dominated by the late car-ball hit.
- Same, after `RB-PHYSICS-001-FR-089` (per-wheel engine force),
  2026-10-01: yaw rate under full steer 2.40 rad/s simulated and recorded
  (3.95-4.0 s, was 2.27); velocity error at 4.05 s 9.4 uu/s (was 47.8),
  0.5 uu/s at 4.133 s. Growth 3-4 s 2.3 uu / 1.5 uu/s (was 2.5 / 5.2),
  4-5 s 7.9 uu / 24 uu/s (was 23 / 40), 6-7 s 329 uu / 2.02 rad (was 517 /
  2.27); 5-6 s 63 uu / 152 uu/s (was 49 / 115) — past the 5.758 s car-ball
  hit the windows follow a different chaotic path.
- Same, after `RB-PHYSICS-001-FR-090` (suspension and hitbox offset),
  2026-10-01: rest height 17.0 simulated and recorded; 0-3 s growth 0.02 uu
  (was 2.2, the old 2.3 uu ride-height offset); at 4.1-4.133 s error 0.1 uu
  / 0.3 uu/s; 4-5 s 7.0 uu / 17 uu/s (was 7.9 / 24); 5-6 s 54 uu / 164
  uu/s (was 63 / 152); 6-7 s 430 uu / 1.25 rad (was 329 / 2.02).
- Same, after `RB-PHYSICS-001-FR-091` (ground-jump force), 2026-10-01:
  vertical velocity matches the recording to 0.1 uu/s from the 4.142 s
  press through 4.175 s (295.9 / 299.9 / 304.0 / 308.0 / 312.0 both); the
  candidate's wheels then leave 1-2 ticks early (vz 5 uu/s high, vx 12 low
  by 4.2 s). Growth 4-5 s 5.1 uu / 15 uu/s (was 7.0 / 17).
- Same, after `RB-PHYSICS-001-FR-092` (wheel reach, sticky timing),
  2026-10-01: every jump tick matches (vx 384.4 vs 385.1 recorded at
  4.183 s; vz within 0.1 uu/s through 4.3 s); velocity error 2.6-3.3 uu/s
  over 4.19-4.3 s (was 13). Growth 4-5 s 2.7 uu / 8.5 uu/s (was 5.1 / 15),
  5-6 s 41 uu / 147 uu/s (was 54 / 170), ball 15 uu (was 25), 7-8 s 853 uu
  (was 1251).
- Same, after `RB-PHYSICS-001-FR-093` (air control during a flip),
  2026-10-01: car-frame flip spin (4.40, 3.30) with roll held and (4.08,
  3.69) after, recorded (4.42, 3.27) / (4.11, 3.66); orientation error
  0.03-0.04 rad through 4.97 s (was 0.12-0.13), velocity error at 5.5 s 32
  uu/s (was 71). Growth 4-5 s 2.7 uu / 0.02 rad, 5-6 s 19 uu / 0.12 rad /
  71 uu/s (was 41 / 0.26 / 147), ball 5-6 s 8.9 uu (was 15).
- Same, after `RB-PHYSICS-001-FR-094` (flip clock), 2026-10-01: flip spin
  stays clamped through 4.975 s and drops at 4.983 s, as recorded; height
  through the flip within 0.1 uu (was 1.9, the flip's vertical damping no
  longer starting a tick early); orientation error 0.04-0.05 rad through
  5.5 s (was rising to 0.10); vz at 5.567 s -300 vs -311 recorded (was
  -288). Growth 4-5 s 2.4 uu / 6.7 uu/s (was 2.7 / 8.0).
- `cargo run -p rb_verify_cli --bin rb-verify -- --self test2.jsonl`
  (manual, owner's machine, 2026-09-04, default 0.02s timestamp
  tolerance, `RB-PHYSICS-001-FR-077`'s own real-capture run): `frames
  compared: 2818, mean ball distance: 2206.08 uu, max ball distance:
  5673.98 uu, car pairs compared: 2818, mean car position/rotation/
  velocity distance: 4508.71 uu / 2.12 rad / 1421.73 uu/s, max car
  position/rotation/velocity distance: 8798.56 uu / 3.14 rad / 3643.64
  uu/s` — this project's first genuine fidelity number (candidate
  actually simulated from the real capture's own recorded input, not an
  unrelated match); see FR-077's entry in `RELEASE_NOTES.md` and
  `RB-PHYSICS-001`'s Interpretation note for what this large a divergence
  does and doesn't establish.
- `rb-verify --self-growth test2.jsonl` (owner's machine, 2026-10-01,
  `RB-VERIFY-003-FR-004`'s real run, 1 s windows): car error ~2 uu /
  0.01 rad for 0-3 s; 34 uu at 3 s; 1,315 uu / 1.37 rad / 2,887 uu/s at
  4 s; ball within 0.05 uu until 5 s, when a car reaches it in one run
  but not the other. Abrupt, car-only derailment: a targeted mechanic
  fix, not broad `RB-PHYSICS-001-FR-005` calibration, comes first.
- `rb-verify --self-growth test2.jsonl` after `RB-PHYSICS-001-FR-079`
  (owner's machine, 2026-10-01): 3-4 s window 10 uu / 67 uu/s (was 34 uu
  / 164 uu/s); 4-5 s window 751 uu / 1.21 rad / 1,902 uu/s (was 1,315 uu
  / 1.37 rad / 2,887 uu/s); ball 0.01 uu (was 0.05) until a car reaches
  it. `--self-trace 4.10 4.20`: the jump fires as a jump (sim vz 286 vs
  recorded 296), no sideways dodge; remaining error is heading (~0.45 rad)
  and ~10% speed.
- `rb-verify --self-trace test2.jsonl 3.7 4.3` after
  `RB-PHYSICS-001-FR-080` (owner's machine, 2026-10-01): orientation error
  at 4.0 s 0.06 rad (was 0.46), at most 0.09 rad through 4.13 s; velocity
  error at 4.10 s 364 uu/s (was 563); jump still correct (vz 286 vs 296).
  Remaining: velocity direction ~53 deg vs 72 deg recorded, speed ~10% low.
- `rb-verify --self-trace test2.jsonl 3.7 4.3` after
  `RB-PHYSICS-001-FR-081` (owner's machine, 2026-10-01): velocity error at
  4.0 s 9.9 uu/s (was ~250), simulated (325, 968) vs recorded (318, 961).
  `--self-growth`: 3-4 s window 2.8 uu / 0.03 rad / 5.7 uu/s (was 10 uu /
  67 uu/s); 4-5 s 532 uu / 1.15 rad / 1,574 uu/s (was 751 / 1.21 /
  1,902). With the zero-steer yaw fix: velocity error at 4.10 s 22 uu/s
  (was 96), at 4.30 s 38 uu/s (was 132); position error at most 6 uu
  through 3.7-4.3 s.
- `rb-verify --self-trace test2.jsonl 4.3 5.0` after
  `RB-PHYSICS-001-FR-083` (owner's machine, 2026-10-01): flip vz stall
  -15.5 uu/s from 4.55 s, matching the recording (was falling to -211);
  vz at 5.0 s -23 (recorded -4, was -211); position error at 5.0 s 79 uu;
  orientation error at 5.0 s 1.41 rad (was 1.63). Damping starts one tick
  early, likely because the trace's per-pair dt differs from RocketSim's
  fixed 1/120.
- After `RB-PHYSICS-001-FR-084` (owner's machine, 2026-10-01):
  `--self-growth` 4-5 s window 30 uu / 0.55 rad / 95 uu/s (was 1,315 uu /
  1.37 rad / 2,887 uu/s at the start of the day), 5-6 s 219 uu (was
  2,660); ball diverges from 5 s. `--self-trace 4.3 5.0`: orientation
  error at 5.0 s 1.36 rad (was 1.41), at 4.30 s 0.21 (was 0.14).
- After `RB-PHYSICS-001-FR-086` (owner's machine, 2026-10-01):
  `--self-growth` 3-4 s 2.5 uu / 0.01 rad / 5.2 uu/s; 4-5 s 16 uu / 0.48
  rad / 35 uu/s (was 30 / 0.55 / 95); 5-6 s 157 uu (was 219).
  `--self-trace 3.7 4.6`: yaw spin ramps with the recording (0.27 vs
  0.31, 0.92 vs 1.06, 1.61 vs 1.80 rad/s over the first 9 ticks after
  3.742 s); orientation error 0.04 rad at the dodge (was 0.22); spin error
  just before it 0.10 rad/s (was 0.71); velocity error after it 36 uu/s
  (was 128).
- `rb-verify --self-trace` against the synthetic capture fixture
  (2026-10-01, `RB-VERIFY-003-FR-005`): runs end-to-end; shows a
  recorded ground jump (t=0.15 s) firing as a dodge in the candidate
  (see Next).

## Risks and decisions needed

- `RB-RESEARCH-O002` (binary reverse engineering) — needs explicit owner
  sign-off after legal/practical review before any work starts, and needs
  the owner's own machine/game install since this sandbox has neither.
  Owner: baileyrd.
