# RocketSim — Reverse-Engineering Report (Sim, Physics, Car/Ball Model)

- **Subject:** [ZealanL/RocketSim](https://github.com/ZealanL/RocketSim), a standalone C++ reimplementation of Rocket League's gameplay logic and physics.
- **Snapshot read:** `main` @ `c2baacb` (2026-07-14), `RS_VERSION "2.2.1"`. Source read in full under `src/`; the vendored Bullet 3.24 under `libsrc/` was diffed in spirit by grepping for `ROCKETSIM CHANGE` markers (not every Bullet file was read).
- **Official docs:** <https://zealanl.github.io/RocketSimDocs/> — a Sphinx/Doxygen site that is explicitly a work in progress and covers only `Arena`, `Car`/`CarState`, `Ball`/`BallState` API signatures plus one usage example. It adds nothing the headers do not already say, so this report is built from the source.
- **Companion report:** [`RLBOT-V5-WIKI-ANALYSIS.md`](./RLBOT-V5-WIKI-ANALYSIS.md) (the RLBot wiki's secondary account of the same physics). Where the two disagree, §14 lists it.
- **Provenance rule:** every number is read from RocketSim's source. RocketSim describes itself as "not a perfectly accurate replication"; several constants carry author TODOs (§13). It is a strong prior, not ground truth — our captures remain authoritative (ADR-0013, ADR-0019).

> Correction to an earlier chat answer: RocketSim *does* have an official documentation site (above); it is just thin.

---

## 1. What it is

- A complete, standalone simulation of RL gameplay + physics: **Soccar, Hoops, Dropshot, Heatseeker, Snowday**, plus a `THE_VOID` mode (no arena, goals or pads — cars and ball fall forever). Rumble is deliberately unsupported.
- Built on a **vendored, modified Bullet 3.24** (rigid body, GJK/EPA, sequential-impulse solver, BVH triangle mesh) with a custom raycast-vehicle (`btVehicleRL`) and RL-specific game logic layered on top.
- Contains no game code; arena collision meshes are **not shipped** — users dump them from their own install with `RLArenaCollisionDumper` and load them at `RocketSim::Init(folder)`.
- Performance claim: ~114 k ticks/s single-thread for 2v2 (v2.1.0), ≈20 min of game time per second; 120 Hz native.
- Bindings (unofficial): Python (`mtheall`), Rust (`VirxEC/rocketsim-rs`). This is the de-facto training sim for RLGym.

### Source layout (all of `src/`, ~200 KB)
| Path | Role |
|---|---|
| `RLConst.h` | every tuned constant, curves, spawn tables, pad coordinates (§15) |
| `RocketSim.{h,cpp}` | global init, mesh loading, mesh-hash whitelist |
| `Sim/Arena/` | the world: step loop, Bullet setup, contact callbacks, goal logic, serialization, clone |
| `Sim/Car/`, `Sim/CarControls.h`, `Sim/Car/CarConfig/` | car state machine + hitbox/wheel presets |
| `Sim/btVehicleRL/` | RL-specific raycast vehicle (suspension, tyre friction) |
| `Sim/Ball/` | ball state, hit impulse, per-mode behaviour |
| `Sim/BoostPad/` (+`BoostPadGrid`) | pads + spatial grid |
| `Sim/MutatorConfig/` | runtime-tunable physics/game constants |
| `Sim/Arena/DropshotTiles/` | hex tile geometry, neighbour sets |
| `Sim/BallPredTracker/`, `Sim/GameEventTracker/` | optional utilities (rolling ball prediction; shot/goal/save events) |
| `CollisionMeshFile/`, `DataStream/` | mesh file format, binary (de)serialisation |
| `Math/` | `Vec`, `RotMat`, `Angle`, `LinearPieceCurve`, UE3 rotator rounding |

---

## 2. Units, coordinates, conventions

- **`BT_TO_UU = 50`, `UU_TO_BT = 1/50`.** Bullet works in "metres", game state is exposed in Unreal units; every force/velocity conversion goes through these. (The header comment says "(1m) … (2cm)"; the factor is what matters.)
- Z up; **blue defends −Y** (`RS_TEAM_FROM_Y(y) = y<0 ? BLUE : ORANGE`, goal-score callback uses `-ball.y`). Orange kickoff = blue's spawn × (−1, −1, 1) with yaw + π.
- `RotMat` is **column-major** (`forward/right/up`); `Angle` order is **Pitch-Yaw-Roll**. Car-local axes: X forward, Y right, Z up (`setCoordinateSystem(1,2,0)` on the vehicle).
- Tick rate configurable **15–120 Hz** (asserted), default 120. Many constants are scaled by `tickTime/(1/120)` so lower rates behave consistently.

---

## 3. Public API surface

- **`RocketSim::Init(meshFolder)`** / `InitFromMem(map<GameMode, vector<FileData>>)` — one-shot, mutex-guarded; loads each `.cmf` mesh, builds a `btBvhTriangleMeshShape` **with internal-edge info** (`btGenerateInternalEdgeInfo`) and checks a **hash whitelist** (16 known Soccar meshes, 12 Hoops; Dropshot unchecked). Heatseeker/Snowday reuse Soccar meshes.
- **`Arena::Create(mode, ArenaConfig, tickRate)`**, `AddCar(team, CarConfig=OCTANE)`, `RemoveCar`, `GetCar(id)`, `Step(n)`, `Clone(copyCallbacks)`, `ResetToRandomKickoff(seed)`, `SetMutatorConfig`, `SetDropshotTilesState`, `IsBallScored()`, `IsBallProbablyGoingIn(maxTime, margin, &team)` (Soccar/Snowday/Hoops only), `Serialize/WriteToFile/LoadFromFile`, callbacks `SetGoalScoreCallback`, `SetCarBumpCallback`.
- **`Car`:** `controls` (`CarControls{throttle, steer, pitch, yaw, roll, jump, boost, handbrake}`), `GetState/SetState(CarState)`, `Demolish`, `Respawn`.
- **`Ball`:** `GetState/SetState(BallState)`, `GetRadius`, `GetMass`; `BallState::Matches(other, 0.8 uu, 0.4 uu/s, 0.02 rad/s)` is the tolerance used by prediction reuse.
- **`BoostPad`:** `GetState/SetState`; **`ArenaConfig`:** `memWeightMode {HEAVY, LIGHT}`, `minPos/maxPos` (default (−5600,−6000,0)…(5600,6000,2200)), `maxAABBLen 370`, `noBallRot=true`, `useCustomBroadphase=true`, `maxObjects 512`, `useCustomBoostPads` + list.
- State is **fully value-typed**: `CarState` carries jump/flip/boost/supersonic/handbrake/auto-flip/demo/contact/`ballHitInfo` fields, so a state set restores the whole machine (`tickCountSinceUpdate` flags a stateset for consumers). Everything round-trips through a versioned binary stream (`MutatorConfig` serialises with a field-count guard).

---

## 4. The tick pipeline (`Arena::Step`)

For each tick (`stepSimulation(tickTime, maxSubSteps=0, fixedTimeStep=tickTime)` → exactly one Bullet step, no interpolation):

1. `setWorldUserInfo(this)`.
2. **Ball sleep:** zero linear and angular velocity ⇒ `ISLAND_SLEEPING`, else `ACTIVE_TAG`.
3. **`Car::_PreTickUpdate` for each car** (§8.1): demo timer / respawn, then `updateVehicleFirst` (wheel rays + friction impulses), contact count, `_UpdateWheels`, air torque, jump, auto-flip, double-jump/flip, auto-roll, `updateVehicleSecond` (suspension + friction impulses), boost.
4. `BoostPad::_PreTickUpdate` (cooldown tick-down) — skipped when ball-only or `THE_VOID`.
5. `Ball::_PreTickUpdate` (Heatseeker steering; Snowday ground-stick reset; Hoops/Dropshot kickoff launch).
6. **`btDiscreteDynamicsWorld::stepSimulation`** — broadphase, narrowphase (contact-added callback fires here: §6), solver, integration.
7. `Car::_PostTickUpdate` (rotMat sync, supersonic, bump-cooldown, `lastControls`) then **`_FinishPhysicsTick`** (apply cached velocity impulses, clamp linear speed to 2300 and angular to 5.5), then pad collision for that car.
8. `BoostPad::_PostTickUpdate` (award boost if locked + active; start cooldown).
9. `Ball::_FinishPhysicsTick` (apply cached extra-hit impulse, clamp linear to `ballMaxSpeed`, angular to 6).
10. Dropshot: re-sync tile collision flags *after* the tick (so a freshly broken tile does not swallow the ball mid-step).
11. Goal callback if `IsBallScored()`; `tickCount++`.

Design consequences worth copying: **velocity-affecting game-logic impulses are cached and applied after the physics step** (`_velocityImpulseCache`) rather than as Bullet forces, so they do not interact with the solver.

---

## 5. Bullet configuration and RocketSim's Bullet patches

- Collision config pools shrunk (÷16/÷32 heavy, ÷32/÷64 light); `btHashedOverlappingPairCache`; broadphase = **`btRSBroadphase`** (custom fixed voxel grid, cell ≈ `maxAABBLen`, static/dynamic split) or stock `btDbvtBroadphase`.
- Solver: `btSequentialImpulseConstraintSolver`, Bullet defaults **10 iterations, warm-starting + SIMD, split impulse enabled**, overridden to match "Bullet from ~2013–2015": **`m_splitImpulsePenetrationThreshold = 1e30`, `m_erp2 = 0.8`**.
- Marked `ROCKETSIM CHANGE` in `libsrc`:
  - **Sphere margin +0.08 bt-units** (and the matching radius add in `btCollisionShape`).
  - **`m_noRot`** on rigid bodies: a no-rotation integrator (`integrateTransformNoRot`) used for the ball (spheres only) to save time; ball angular velocity is still tracked/clamped.
  - **Solver:** removed the positive-penetration velocity bleed (`velocityError -= penetration*invTimeStep` commented out — "ruins ball bounces at low velocities"); skip joints; null-contact skip; contact conversion split into `convertContactInner`.
  - **"Special" manifolds (`m_isSpecial`)**: contacts flagged in the add-callback are *accumulated per body* (count, mean normal, mean lever distance, friction, restitution) and resolved as **one averaged constraint against a fixed body** in a separate pass (`convertContactSpecial`), with special/non-special rows solved in different loops. This is how **ball-vs-world** is made to behave like RL (one averaged normal, not N independent points). Snowday's puck opts out.
  - `btPersistentManifold`: don't overwrite old manifold points with new (`ROCKETSIM CHANGE` at refresh).
  - `btContactConstraint.h`: `applyImpulses` flag for computing impulses without applying (used by wheel push-back).
  - Vehicle raycaster: extra collision masks (so wheel rays hit the Dropshot floor).
  - Internal members made public, `btCollisionObject` gets `userIndex`/`m_specialResolveInfo`.
- **Internal-edge utility** (`btAdjustInternalEdgeContacts`) is applied to *every* contact in the callback, using the per-mesh `btTriangleInfoMap` — this is what stops cars/balls snagging on mesh seams.

---

## 6. Collision routing and materials

`gContactAddedCallback = Arena::_BulletContactAddedCallback` fires for every new manifold point. Bodies carry `userIndex` {car, ball, dropshot tile, none}; the lower-index body is "A". Dispatch:

| Pair | Handler | Material override |
|---|---|---|
| Car–Ball | `Ball::_OnHit` (extra impulse §9) | friction **2.0**, restitution **0.0** |
| Car–Car | `_BtCallback_OnCarCarCollision` (bump/demo §8.8) | friction 0.09, restitution 0.1 |
| Car–World | records `worldContact{hasContact, normal}` | friction 0.3, restitution 0.3 (mutator-tunable) |
| Ball–World | `Ball::_OnWorldCollision`; sets `m_isSpecial` (not Snowday) | rigid-body values: friction 0.35, restitution 0.6 |
| Ball–Dropshot tile | `_OnDropshotTileCollision` | — |

- Base arena friction/restitution **0.6 / 0.3**, rolling friction 0.
- Car rigid-body defaults: friction 0.3, restitution 0.1 (but "cars don't use their rigidbody values for world contact" — the callback overrides them).
- **Car geometry:** *one* `btBoxShape` child of a compound shape at `hitboxPosOffset`; centre of mass is the car origin (0,0,0). Gyroscopic force disabled (`m_rigidbodyFlags = 0`). Collision groups: car + wheel rays include `DROPSHOT_FLOOR`; ball includes `HOOPS_NET` and `DROPSHOT_TILE`; the Hoops **net mesh is identified by vertex count (505)** and masked out for cars.
- **Arena geometry:** the dumped BVH meshes (internal-edge-aware) **plus analytic planes** — floor (z=0, or 1.5 in Dropshot), ceiling (2048 Soccar / 1820 Hoops / 2024 Dropshot), side walls (x=±4096; Hoops ±8900/3), Hoops back walls (y=±3581). Soccar back walls are part of the mesh (goals are open).
- **Scored test is positional, not a trigger volume** (§11).

---

## 7. Arena-level game logic

- **Kickoff:** `ResetToRandomKickoff(seed)` shuffles the 5 canonical spawns (4 for Heatseeker), extra cars use respawn points with +250 uu Y stagger per stacked car, orange mirrored. Ball at default state (Heatseeker: random side, `BALL_START_POS/VEL`; Snowday: z-vel = ε so it doesn't freeze). Resets boost pads and Dropshot tiles.
- **Goal scoring** (`IsBallScored`): Soccar/Heatseeker/Snowday `|ball.y| > goalBaseThresholdY (5124.25) + ballRadius`; Hoops `z < 270` and inside the hoop disc `(x² + (0.9|y| − 2770)²) < 716²`; Dropshot `z < −1.75·ballRadius`.
- **`IsBallProbablyGoingIn`** extrapolates a ballistic path to the goal plane with approximate goal bounds (half-width 892.755, height 642.775, margin 0.1·radius); Hoops version solves a quadratic for rim-height crossing with a crude backboard-bounce model. Unsupported for other modes (hard error).
- **Demolition/bump** are in the car–car callback (§8.8); respawn uses random of 4 respawn points at z=36.

---

## 8. The car model

### 8.1 Per-tick order (`Car::_PreTickUpdate`)
1. Clamp inputs to [−1,1] (unless `RS_MAX_SPEED`).
2. If demoed: count down timer, respawn at 0, rigid body `DISABLE_SIMULATION` + no contact response; **return**.
3. `updateVehicleFirst` — wheel rays, suspension length, friction impulse computation.
4. `jumpPressed = jump && !lastControls.jump` (edge-triggered).
5. Count wheels in contact; **`isOnGround = wheels ≥ 3`**.
6. `_UpdateWheels` (throttle/brake/steer/friction/sticky).
7. If <3 wheels: `_UpdateAirTorque` (air control; allowed only with 0 wheels, flip torque otherwise); else `isFlipping=false`.
8. `_UpdateJump`, `_UpdateAutoFlip`, `_UpdateDoubleJumpOrFlip`.
9. If throttle ≠ 0 and (1–3 wheels or world contact): `_UpdateAutoRoll`.
10. `updateVehicleSecond` — **suspension impulses then friction impulses applied**.
11. `_UpdateBoost`.

### 8.2 Hitboxes and wheel presets (`CarConfig.cpp`)
Hitbox sizes are **full extents**, deliberately *not* the values RL's `GetLocalCollisionExtent()` returns (author: those are slightly larger than the simulated box; his values reproduce RL's inertia matrix).

| Car | hitbox (L×W×H) | hitbox offset | front wheel r / sus-rest / connect | back wheel r / sus-rest / connect |
|---|---|---|---|---|
| Octane | 120.507×86.6994×38.6591 | (13.8757, 0, 20.755) | 12.5 / 38.755 / (51.25, 25.90, 20.755) | 15.0 / 37.055 / (−33.75, 29.50, 20.755) |
| Dominus | 130.427×85.7799×33.8 | (9, 0, 15.75) | 12.0 / 33.95 / (50.30, 31.10, 15.75) | 13.5 / 33.85 / (−34.75, 33.00, 15.75) |
| Plank (Batmobile) | 131.32×87.1704×31.8944 | (9.00857, 0, 12.0942) | 12.5 / 31.9242 / (49.97, 27.80, 12.0942) | 17.0 / 27.9242 / (−35.43, 20.28, 12.0942) |
| Breakout | 133.992×83.021×32.8 | (12.5, 0, 11.75) | 13.5 / 29.7 / (51.50, 26.67, 11.75) | 15.0 / 29.666 / (−35.75, 35.00, 11.75) |
| Hybrid | 129.519×84.6879×36.6591 | (13.8757, 0, 20.755) | 12.5 / 38.755 / (51.25, 25.90, 20.755) | 15.0 / 37.055 / (−34.00, 29.50, 20.755) |
| Merc | 123.22×79.2103×44.1591 | (11.3757, 0, 21.505) | 15.0 / 39.505 / (51.25, 25.90, 21.505) | 15.0 / 39.105 / (−33.75, 29.50, 21.505) |
| Psyclops | Octane + 0.134 each axis | (13.8757, 0, 15) | 12.5 / 33.0 / (51.25, **5.0**, 15) | 15.0 / 31.3 / (−33.75, 29.50, 15) |

Psyclops is "three-wheel": front pair merged (connect Y=5), own steering/lateral-friction curves, no baseline sticky force. Chassis mass **180** (`CAR_MASS_BT`); ball mass 30 (=180/6). Default `dodgeDeadzone = 0.5` (`|yaw|+|pitch|+|roll|` must reach it to flip).

### 8.3 Suspension (`btVehicleRL`)
- Four wheels (always; "no car actually has three"), ray direction −Z, axle −Y, wheel Y negated for the second of each pair.
- Per-wheel: `suspensionRestLength = preset − MAX_SUSPENSION_TRAVEL(12)`; ray length = `rest + travel + radius − SUSPENSION_SUBTRACTION(0.05)`; stiffness **500**, damping compression **25** / relaxation **40**; force scale **front 35.75, back 54.265**; `maxSuspensionForce = FLT_MAX`; "RL never uses downward suspension forces" (clamped ≥ 0).
- `force = (rest − length)·stiffness·(1/clippedCosine)`; damped by relative velocity; scaled; applied as an **impulse at the contact point** along the contact normal (+ `extraPushback`).
- **`extraPushback`**: when the trace distance falls under `(rest + radius) − 0.05` against a *static* body, a Bullet `resolveSingleCollision` (impulses not applied) supplies an extra push, divided across wheels — the "ERP" mechanism the repo calibrated in ADR-0042.
- Relative-velocity denominator < 0.1 is treated as degenerate (velocity 0, clipped factor 10).

### 8.4 Tyre friction and drive
- Per wheel with ground contact: lateral dir = wheel axle, longitudinal = lat × normal. **Slip input** = `lat / (long + lat)` of the contact-point velocity (only if lateral > 5 uu/s).
- `latFriction = LAT_FRICTION_CURVE(slip)` (1.0→0.2; Psyclops 0.30→0.25); `longFriction = LONG_FRICTION_CURVE` (empty → default). **Handbrake** lerps both by `HANDBRAKE_*_FRICTION_FACTOR` curves weighted by the analog `handbrakeVal`; without handbrake `longFriction = 1`.
- **Non-sticky** (zero throttle): both scaled by `NON_STICKY_FRICTION_FACTOR_CURVE(contactNormal.z)` (0.1@0 → 0.5@0.7075 → 1@1).
- Impulses: side impulse from a Bullet bilateral constraint (`resolveSingleBilateral`); rolling friction = `−engineForce/frictionScale` if driving, else a clamped brake term (`−relVel·113.73963` clamped to ±brake, with a small deadzone at <80 TPS), else 0. `frictionScale = chassisMass/3`. Total per-wheel impulse = `(forward·rolling·long + axle·side·lat)·frictionScale`, applied at the contact lever arm **projected perpendicular to up**.
- **Throttle/brake logic:** boost forces `realThrottle = 1`; opposing-direction throttle ⇒ full brake (engine killed if speed > 0.01); coasting ⇒ brake factor 0.15 (full brake below 25 uu/s forward speed); deadzone 0.001. **Engine force = throttle · `THROTTLE_TORQUE_AMOUNT (180·400)` · UU_TO_BT · `DRIVE_SPEED_TORQUE_FACTOR_CURVE(|v|)`** (1.0@0 → 0.1@1400 → 0@1410; ÷4 when <3 wheels touch). Brake torque `180·(14.25+1/3)`.
- **Steering:** wheels 0/1 steer angle = `STEER_ANGLE_FROM_SPEED_CURVE(|v|) · steer` (0.53356@0, 0.31930@500, 0.18203@1000, 0.10570@1500, 0.08507@1750, 0.03454@3000 rad); handbrake blends toward `POWERSLIDE_STEER_ANGLE_FROM_SPEED_CURVE` (0.39235@0 → 0.12610@2500). `handbrakeVal` rises 5/s, falls 2/s.
- **Sticky force:** if any wheel touches the *world*: force along the average contact normal = `stickyScale · (−650) · 180`, with `stickyScale = 0.5` (0 for Psyclops) `+ (1 − |up.z|)` when "full-stick" (throttle ≠ 0 or |v| > 25).

### 8.5 Jump
- `isOnGround && !isJumping` resets `hasJumped/jumpTime` — except for `JUMP_MIN_TIME + JUMP_RESET_TIME_PAD (0.025 + 0.025 s)` after the press (avoids resetting before leaving the ground on a tap).
- Press (edge) on ground: **impulse `JUMP_IMMEDIATE_FORCE (875/3 ≈ 291.667 uu/s)` along car up**, `isJumping = true`.
- While jumping: **force `JUMP_ACCEL (4375/3 ≈ 1458.33 uu/s²)` along up** every tick; ×**0.62** during the first 0.025 s (a TODO'd fudge); continues while button held and `jumpTime < 0.2`, forced through the 0.025 s minimum.
- `jumpTime` keeps counting while `isJumping || hasJumped`.

### 8.6 Double jump and flip (`_UpdateDoubleJumpOrFlip`)
- Reset on ground. In air: `airTimeSinceJump` accumulates only after the jump ends; usable while `< 1.25 s` (`DOUBLEJUMP_MAX_DELAY`).
- On jump press: **flip input** if `|yaw|+|pitch|+|roll| ≥ dodgeDeadzone`, else **double jump** (another 291.667 impulse along up). A double jump or flip consumes both (unless `unlimitedFlips/unlimitedDoubleJumps`). Auto-flipping blocks both.
- **Flip:** `dodgeDir = (−pitch, yaw+roll)` normalised (if either component > 0.1; else zero = "stall"), `flipRelTorque = (−dir.y, dir.x)/tickScale`. Impulse on the **flat 2-D forward/right plane** = `dir·500`, with X scaled `1 + (maxScale−1)·|v|/2300` where maxScale = 1 forward / **2.5 backward** and `×16/15` extra for backward, and Y scaled with **1.9**. "Backward" is decided by dodge direction vs. current forward speed (below 100 uu/s: by input sign).
- **Flip dynamics:** torque `(260, 224)` (X side / Y forward) for `FLIP_TORQUE_TIME 0.65 s` (min 0.41), applied as a *world torque from the inverse inertia*; pitch-cancel: same-sign pitch input scales the Y torque by `1 − |pitch|` and also enables air control; **vertical velocity damped** by `(1 − 0.35)^(tickScale)` per tick between `flipTime ≥ 0.15` and (`<0.21` or while falling) until 0.65 s; pitch air-control locked until `0.65 + 0.3 s` after the flip.

### 8.7 Air control, auto-flip, auto-roll, boost
- **Air torque** (`CAR_AIR_CONTROL_TORQUE (130, 95, 400)` pitch/yaw/roll, `CAR_AIR_CONTROL_DAMPING (30, 20, 50)`), world torque = inverse-inertia · (input torque − damping) · `CAR_TORQUE_SCALE (2π/65536·1000)`. Damping on pitch/yaw is *reduced by the input magnitude* (`1 − |input|`); roll damping is not. Only with **all four wheels airborne**. Air throttle force **66.667 uu/s²** forward.
- **Auto-flip (roof self-right):** on a jump press with world contact whose normal z > 0.707 and |roll| > 2.8 rad: `autoFlipTimer = 0.4·|roll|/π`, impulse **200 uu/s into the roof**, then constant angular-velocity increments of 50·dt about forward until the timer ends. (Repo implements this — ADR-0043.)
- **Auto-roll (self-level):** with throttle, some-but-not-all wheels in contact (or world contact): a downward force 100 and a levelling torque (scale 80) built from the contact normal's cross products; magnitudes vanish as the car levels. (Repo: ADR-0050.)
- **Boost:** needs `boost > 0`; **minimum 0.1 s boost** once started; consumes `100/3 = 33.33` per second; force = `boostAccel · forward · mass` (**991.667 ground, 1058.33 air**; ground/air from `isOnGround`). `timeSinceBoosted` resets; optional **recharge** (`rechargeBoostEnabled`: 10/s after 0.25 s) used by Dropshot. Clamp 0–100. Heatseeker spawns with 100 and `boostUsedPerSecond = 0`.
- **Supersonic:** enters at ≥ 2200 uu/s; **stays** while ≥ 2100 for up to `SUPERSONIC_MAINTAIN_MAX_TIME` 1 s, else needs 2200 again.
- **Speed limits:** linear 2300, angular 5.5 rad/s, applied after the step (`_FinishPhysicsTick`).

### 8.8 Bump / demolition (`Arena::_BtCallback_OnCarCarCollision`)
- Evaluated **both ways** per contact. Skips demoed cars and the pair in cooldown (`BUMP_COOLDOWN_TIME 0.25 s`).
- Conditions: car1 moving toward car2 (`vel·Δpos > 0`), speed toward car2 > car2's speed along car1's heading, and the contact point is on car1's **bumper** (`local x > 64.5 uu`).
- `DemoMode`: `NORMAL` (demo if **attacker supersonic**; no team demos unless `enableTeamDemos`), `ON_CONTACT`, `DISABLED`. Demolished car: `demoRespawnTimer = respawnDelay (3 s)`.
- Non-demo bump: impulse = `velDir·base + hitUpDir·up`, with base/up from piecewise curves of the closing speed (ground: 5/6→1100@1400→1530@2200; air: 5/6→1390→1945; up: 2/6→278→417), scaled by `bumpForceScale`; **added through the velocity cache**, plus the bump callback.
- Car–car friction/restitution overridden to 0.09 / 0.1.

---

## 9. The ball

- **Shape:** sphere radius **91.25** (Hoops 96.3831, Dropshot 100.2565, Snowday puck = 20-gon cylinder r 114.25, h 62.5, 20 pts, mass 50, friction 0.1, restitution 0.3). Mass 30. Rest height 93.15 (> radius "because of arena mesh collision margin").
- **Damping:** linear `m_linearDamping = ballDrag = 0.03` (Bullet applies `v *= (1 − d)^dt`); angular damping 0; ball–world friction 0.35, restitution 0.6; **max speed 6000**, **max angular speed 6**.
- **`noRot` optimisation** defaults on (spheres only): rotation is not integrated, only velocities.
- **Extra car–ball hit impulse** (`Ball::_OnHit`) — the key RL "feel" term, *applied once per ≥2 ticks per car*:
  - `relPos = ballPos − carPos`, `relVel = ballVel − carVel`, `relSpeed = min(|relVel|, 4600)`.
  - `hitDir = normalize(relPos · (1, 1, 0.35))`; remove 35 % of its forward-axis component: `hitDir −= forward · (hitDir·forward) · (1 − 0.65)`; renormalise. (Hoops grounded cars use zScale ×1.55.)
  - `Δv = hitDir · relSpeed · BALL_CAR_EXTRA_IMPULSE_FACTOR_CURVE(relSpeed) · ballHitExtraForceScale`, curve **0.65 (0–500) → 0.55@2300 → 0.30@4600**; added post-step via the velocity cache.
  - Contact friction forced to 2.0, restitution 0.0; `BallHitInfo` records `relativePosOnBall`, `ballPos`, `extraHitVel`, tick counts.
- **Per-mode behaviour:** §10.

---

## 10. Game modes

- **Heatseeker:** target goal Y=±5120, Z=320; every tick the ball's velocity direction is steered toward the goal (yaw blend 1.45, pitch blend 0.78, max pitch 7000·π/32768, **UE3 rotator rounding** `RoundAngleUE3` — "surprisingly important for accuracy"); speed eases (blend 0.3) to a target that starts at 2900 and **+85 per hit (≥1 s apart)**, cap 4600; wall bounce toward the other goal adds a 1/3-speed impulse (30 % straight up). Kickoff spawn (−1000, −2220, 92.75), v=(0, −65, 650), side randomised. Infinite boost.
- **Snowday:** puck as above; on world contact applies a ground-stick force 70 along −normal (once per tick); goal logic as Soccar.
- **Hoops:** separate mesh set, ARENA_EXTENT X = 8900/3, Y 3581, height 1820; net mesh masked from cars; ball launched **+1000 uu/s Z after 0.265 s** if still frozen at centre; own pad layout (14 small, 6 big), spawns, respawns; goal by hoop-disc test.
- **Dropshot:** 70 hex tiles per team (`13…7` per row), tile geometry in Bullet units (`7.6643 × 8.85`), tile convex hulls clamped at the midline; ball launched **+985 uu/s after 0.26 s**; **charge model:** `accumulatedHitForce += (car→ball closing speed)` when ≥ 500; ≥ 2500 ⇒ charge level 2, ≥ 11000 ⇒ 3; level L damages radius L (1 / 7 / 19 tiles) — only tiles on the targeted side when charged; damage needs downward speed ≥ 250 and ≥ 0.1 s since the last damage; damaged tile state increments to `STATE_BROKEN` (collision then set `CF_NO_CONTACT_RESPONSE`); scoring = falling below the floor. Cars spawn with 100 boost and recharge.

---

## 11. Boost pads

- 34 pads Soccar (28 small + 6 big), 20 Hoops (14 + 6); **big pads first** in RocketSim's ordering (the opposite convention to RLBot's 34-pad list — do not index across them).
- Small: +12, cooldown 4 s; big: +100, 10 s (mutator-tunable).
- **Pickup:** if the car was *not* locked to this pad last tick → test car **origin** inside a cylinder (r 144/208, **height 95**) in XY and Z; if it was → test the car's **AABB** against a box (small 120², big 160² radius, height 64). A locked car keeps "holding" the pad until it leaves, so a stationary car cannot re-trigger. Collision is evaluated *after* the physics step; boost is granted in `_PostTickUpdate`; at ≥100 boost the grid skips the car.
- `BoostPadGrid`: spatial hash for the default layout; custom pad lists fall back to O(cars×pads).

---

## 12. Optional utilities

- **`BallPredTracker`:** keeps a private `Arena` and a vector of future ball states; on update, reuses the old trajectory if `predData[ticksElapsed].Matches(current)` (tolerance 0.8 uu / 0.4 uu/s / 0.02 rad/s), shifting and extending it; otherwise re-simulates fully. Ball-only (no cars) physics; `GetBallStateForTime(t)` indexes by tick.
- **`GameEventTracker`:** heuristic shot / goal / save / assist events built on `IsBallProbablyGoingIn` (defaults: shot min speed 1750 uu/s, touch delay 0.3 s, shot cooldown 1 s, min time-to-score 2 s, goal touch window 4 s, pass window 2 s). Explicitly *not* Rocket League's own scoring values.

---

## 13. Author-flagged uncertainties (RocketSim's own TODO/comments)

- Suspension constants "might change from car to car" (applied uniformly).
- Dropshot `MIN_ABSORBED_FORCE_*`, `MIN_DAMAGE_INTERVAL`, `BALL_LAUNCH_*` — "unconfirmed assumptions based on lots of testing".
- `JUMP_PRE_MIN_ACCEL_SCALE 0.62` and the jump reset pad — "RL does something similar … not exactly the same".
- `ROLLING_FRICTION_SCALE_MAGIC 113.73963` — "no idea where this number comes from".
- Auto-flip: "Improve accuracy"; hitbox sizes validated only via inertia-matrix match; Hoops pad at (−1280, 2304) omitted for a suspected Psyonix radius bug.
- Accuracy statement: errors accumulate; best for short/consistent-feedback horizons.

---

## 14. Differences from the RLBot v5 wiki (see companion report)

| Item | RocketSim (source) | RLBot wiki |
|---|---|---|
| Boost accel (g/a) | 2975/3 = 991.667 / 3175/3 = 1058.33 | 991.666 / 1058.333 ✅ |
| Jump impulse / hold | 291.667 / 1458.33 ✅ | 292 / 1460 over 0.2 s ✅ |
| Air throttle | 200/3 = 66.667 ✅ | 66.667 ✅ |
| Brake / coast | torque-based; coast factor 0.15 | 3500 / 525 uu/s² (derived) |
| Dropshot ball radius | 100.2565 | 102.24 |
| Hoops ball radius | 96.3831 | 98.38 |
| Dropshot arena height / floor z | 2024 / 1.5 | 1986 / 3.2 |
| Dropshot damage interval | 0.1 s | 0.2 s |
| Dropshot recharge delay | 0.25 s | ≈0.5 s |
| Dropshot ball launch | +985 uu/s @ 0.26 s | +1000 uu/s |
| Boost-pad pickup height | cylinder 95 uu | 165/168 uu |
| Boost-pad z | 70 (small) / 73 (big) | 0.082 / 8 (RLBot FieldInfo) |
| Ball drag | 0.03 as Bullet damping `(1−d)^dt` | 0.030562 (fitted terminal velocity) |
| Ball radius | 91.25 | 91.25 (92.75 in one tutorial) |
| Hitboxes | Inertia-matched custom values | RL-reported extents differ (author says those are wrong) |

---

## 15. Constant appendix (`RLConst.h`, abridged to what matters for fidelity)

Gravity −650 · arena extents (4096, 5120, 2048) · base friction/restitution 0.6/0.3 · car mass 180 · ball mass 30 · car–ball (2.0 / 0.0) · car–world (0.3 / 0.3) · car–car (0.09 / 0.1) · ball friction/restitution 0.35/0.6 · ball drag 0.03 · max speeds car 2300, ball 6000 · max angular car 5.5, ball 6 · boost max 100, use 33.33/s, spawn 33.33, min boost time 0.1 s · supersonic 2200 (maintain 2100, ≤1 s) · powerslide rise 5/s fall 2/s · throttle torque 180·400, brake torque 180·(14.25+1/3), stopping speed 25, coast brake 0.15, throttle deadzone 0.001 · air throttle 66.667 · jump accel 1458.33, immediate 291.667, min 0.025 s, max 0.2 s, reset pad 0.025 s, double-jump window 1.25 s · flip: z-damp 0.35 (start 0.15, end 0.21), torque time 0.65 (min 0.41), pitch lock 1.0 (+0.3), initial velocity 500, torque (260, 224), side scale 1.9, backward scale 2.5, back X ×16/15 · air-control torque (130, 95, 400), damping (30, 20, 50), torque scale 2π/65536·1000 · auto-flip impulse 200, torque 50, time 0.4, normal-z 0.7071, roll 2.8 · auto-roll force 100, torque 80 · ball–car extra impulse z-scale 0.35, forward scale 0.65, max Δv 4600, curve (0→0.65, 500→0.65, 2300→0.55, 4600→0.30) · bump cooldown 0.25, min forward distance 64.5, demo respawn 3 s · spawn rest z 17, respawn z 36 · goal threshold y 5124.25 · suspension: stiffness 500, damping 25/40, travel 12, subtraction 0.05, force scale front 35.75 / back 54.265 · steering/torque/friction curves as in §8.4.

---

## 16. Cross-check against this repository

`crates/rb_physics_bullet` is, by design (ADR-0004), a Rust port of Bullet's pipeline plus RocketSim-style car mechanics, calibrated against captures. Read against RocketSim:

**Ported/aligned (evidence: matching constants, ADRs, and symbols in `crates/`):** gravity −650, ball radius 91.25 and mass 30, car mass 180, 2300 / 1410 / 6000 speed limits, brake 3500 uu/s², boost use 33.3/s with the 0.1 s minimum (ADR-0052), 1.25 s double-jump expiry (ADR-0055), jump 291.667/1458.33 (without the 0.62 early-scale, see below), steer-angle curve (0.53356 …), the Octane hitbox (120.507 …), flip torque/z-damp and pitch cancel (ADR-0014), air control (ADR-0015), wheel-ray ground contact and per-wheel side impulses (ADR-0016/0017/0051), suspension hit-box offset (ADR-0018), roof auto-flip (ADR-0043), auto-roll (ADR-0050), car–ball contact following RocketSim (ADR-0026/0045/0062), ball–world contact and averaged normal (ADR-0027/0038/0039), internal-edge utility and ball manifolds (ADR-0028/0035/0041/0046/0047), RocketSim's meshes in Bullet BVH order (ADR-0037), solver ERP 0.8 (ADR-0038).

**Deliberate divergence from RocketSim:** jump force follows the capture, not RocketSim (ADR-0019); the `JUMP_PRE_MIN_ACCEL_SCALE = 0.62` early-window scale is *not* applied because the owner's capture shows full-strength acceleration (`drive/jump.rs`, `RB-PHYSICS-001-FR-091`); the repo also records rejected solver-parity changes (ADR-0030).

**RocketSim features with no counterpart found in `crates/`** (grep for `BoostPad`, `Hoops`, `Dropshot`, `Snowday`, `supersonic`, `BallPred`, `Respawn`, `bump`, `no_rot` returned no source hits; `demo` / `Heatseeker` / `mutator` appear only in 1–3 files): boost pads, bump/demolition, supersonic state, Hoops/Dropshot/Heatseeker/Snowday modes, mutator configuration, ball-prediction tracker, game-event tracker, kickoff randomisation/respawn, arena/state serialisation, `BallState::Matches` tolerances, the `noRot` ball optimisation, Psyclops three-wheel behaviour, non-Octane presets (Dominus, Plank, Breakout, Hybrid, Merc). These are the natural next-port candidates, ordered by relevance to a Soccar verification pipeline: **boost pads → bump/demo → supersonic → car presets → ball prediction**.

**Verification hooks suggested by this reading** (not done here): (a) use RocketSim's per-preset hitbox/wheel table to extend the car presets with ground-truth-checkable values; (b) the flip speed scales (1.9 / 2.5 / 16/15) are already confirmed exact against RocketSim in `drive/tests.rs` (FR-059), so only the *capture* agreement on recorded dodges remains open; (c) treat §13's TODO list as the places RocketSim itself is least trustworthy when scoring divergence.
