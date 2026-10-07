# Physics parity: `rb_physics_bullet` vs RocketSim 2.2.1

- **Scope:** every physics constant and mechanic in `crates/rb_physics_bullet` (read from `src/`, not from ADR prose) against RocketSim `c2baacb` (`RLConst.h` and `src/Sim/**`; see [`ROCKETSIM-ANALYSIS.md`](./ROCKETSIM-ANALYSIS.md)).
- **Scope limit:** Soccar, Octane, 120 Hz. `rb_env`/`rb_scenario` hold no physics constants of their own.
- **Reading the verdicts.** RocketSim is a strong prior, not ground truth (ADR-0013, ADR-0019): a ⚠ marked "calibrated" is a deliberate, capture-driven divergence, not a bug.

| | |
|---|---|
| ✅ | same value / same mechanism |
| ≈ | equivalent by derivation, or same number in different units/form |
| ⚠ | differs (deliberately or unexplained) |
| ❌ | RocketSim has it, we do not |
| ➕ | we have it, RocketSim does not |

## 1. Constants

### World, ball, car body
| Quantity | RocketSim | rusty_bullet | |
|---|---|---|---|
| Gravity | −650 | −650 (`world.rs`, sticky `STICKY_GRAVITY_Z`) | ✅ |
| Ball collision radius | 91.25 | 91.25 `BALL_COLLISION_RADIUS` | ✅ |
| Ball body radius | sphere 91.25 + Bullet sphere margin +0.08 bt (=4 uu) | `BALL_RADIUS` 93.15 (rest height); 91.25 used for contacts (`BALL_CONTACT_SLACK`, ADR-0034/0036/0045) | ⚠ modelled differently, calibrated |
| Ball mass | 30 | 30 | ✅ |
| Ball drag | 0.03, `v *= (1−d)^dt` | 0.03, `(1−d).powf(dt)` (`integrate::apply_damping`) | ✅ |
| Ball friction / restitution | 0.35 / 0.6 | 0.35 / 0.6 (`RigidBody::ball`) | ✅ |
| Ball max speed / angular | 6000 / 6 | 6000 / 6 | ✅ |
| Ball rotation integration | skipped (`noRot`, default on) | integrated | ⚠ perf-only in RocketSim; outputs differ only in `rot`, not `vel` |
| Car mass | 180 | 180 | ✅ |
| Car max speed / angular | 2300 / 5.5 | 2300 / 5.5 | ✅ |
| Octane hitbox (full) / offset | 120.507×86.6994×38.6591 / (13.8757, 0, 20.755) | half-extents 60.2535, 43.3497, 19.32955 / same offset | ✅ |
| Arena friction / restitution | 0.6 / 0.3 | 0.6 / 0.3 | ✅ |
| Car–ball friction / restitution | 2.0 / 0.0 | 2.0 / 0.0 `CAR_BALL_MATERIAL` | ✅ |
| Car–world friction / restitution | 0.3 / 0.3 | 0.3 / 0.3 `CAR_WORLD_MATERIAL` | ✅ |
| Car–car friction / restitution | 0.09 / 0.1 | 0.09 / 0.1 `CAR_CAR_MATERIAL` | ✅ (contact only; no bump logic, §2) |
| Car own rigid-body friction / restitution | 0.3 / 0.1 | not a separate constant (materials above are per pair) | ≈ |

### Car–ball extra hit impulse
| Quantity | RocketSim | rusty_bullet | |
|---|---|---|---|
| Z scale / forward scale | 0.35 / 0.65 | 0.35 / 0.65 | ✅ |
| Max relative speed | 4600 | 4600 | ✅ |
| Factor curve | (0,.65) (500,.65) (2300,.55) (4600,.30) | identical | ✅ |
| Applied after the physics step, ≥2 ticks apart per car | yes (velocity cache) | ADR-0062 / ADR-0047 | ✅ (ported; behaviour not re-read here) |
| `ballHitExtraForceScale` mutator | 1.0 default | `CarBallTuning::hit_scale` 1.0 | ✅ |

### Drive, steering, tyres
| Quantity | RocketSim | rusty_bullet | |
|---|---|---|---|
| Throttle | engine force `180·400·UU_TO_BT` per wheel via friction impulse | `THROTTLE_ACCELERATION` 1600 uu/s² (=4 wheels × 8 m/s² derived) | ≈ derived from the same constant |
| Drive taper curve | (0,1) (1400,.1) (1410,0) | same | ✅ |
| Taper input | `abs(forward speed)` | signed speed in throttle direction, clamped ≥0 | ⚠ deliberate (FR-058 non-goal) |
| Engine ÷4 when <3 wheels touch | yes | yes (`ground.rs` `partial_contact` 0.25) | ✅ |
| Brake | torque `180·(14.25+1/3)` | 3500 uu/s² (derived) | ≈ |
| Coast brake / full-stop speed | 0.15 / 25 | 0.15 / 25 | ✅ |
| Throttle deadzone | 0.001 | 0.001 | ✅ |
| Steer angle curve | (0,.53356) (500,.31930) (1000,.18203) (1500,.10570) (1750,.08507) (3000,.03454) | identical | ✅ |
| Powerslide steer curve | (0,.39235) (2500,.12610) | identical | ✅ |
| Lateral friction curve | (0,1) (1,.2) | identical | ✅ |
| Handbrake lat factor | constant 0.1 | 0.1 | ✅ |
| Handbrake long factor curve | (0,.5) (1,.9) | identical | ✅ |
| Non-sticky friction curve | (0,.1) (.7075,.5) (1,1) | identical | ✅ |
| Handbrake rise / fall | 5 / 2 per s | 5 / 2 | ✅ |
| Slip threshold | 5 uu/s | 5 | ✅ |
| `frictionScale` | mass/3 = 60 | `180/3` | ✅ |
| Side-impulse contact damping | Bullet `resolveSingleBilateral` | `SIDE_IMPULSE_DAMPING` 0.2 | ≈ Bullet default |
| Axle x (front / rear) | 51.25 / −33.75 | 51.25 / −33.75 | ✅ |
| Front / back wheel y | 25.90 / 29.50 | 25.90 / 29.50 | ✅ |
| Wheel ray start z | 20.755 | 20.755 | ✅ |

### Suspension
| Quantity | RocketSim | rusty_bullet | |
|---|---|---|---|
| Stiffness / damping (compress, relax) | 500 / 25 / 40 | 500 / 25 / 40 | ✅ |
| Max travel | 12 | 12 | ✅ |
| Rest length front / back | 38.755−12 / 37.055−12 | same | ✅ |
| Wheel radius front / back | 12.5 / 15.0 | 12.5 / 15.0 | ✅ |
| Force scale front / back | 35.75 / 54.265 | `36−0.25` / `54+0.25+0.015` | ✅ |
| `SUSPENSION_SUBTRACTION` | 0.05 bt = 2.5 uu, on ray length **and** pushback reach | 2.5 uu on pushback only | ⚠ deliberate (FR-092) |
| Pushback ERP | Bullet default 0.2 | 0.1 | ⚠ calibrated (ADR-0042) |
| Steep-contact clip | denominator <0.1 → factor 10 | `STEEP_CONTACT_CLIP` 10 | ✅ |
| Wheels for "on ground" | ≥3 | 3 | ✅ |
| Sticky force | 0.5 base (+`1−|up.z|` when full-stick), 0 for Psyclops | 0.5, full-stick speed 25 | ✅ (Octane) |
| Sticky force timing | applied this tick | applied one step late (`sticky_surface_up`) | ⚠ deliberate (FR-092) |

### Jump, flip, air
| Quantity | RocketSim | rusty_bullet | |
|---|---|---|---|
| Jump impulse | 875/3 ≈ 291.667 | 875/3 | ✅ |
| Hold acceleration | 4375/3 ≈ 1458.33 | 4375/3 | ✅ |
| Early-window (0.025 s) scale | **×0.62** | not applied | ⚠ deliberate, capture shows full strength (FR-091, ADR-0019) |
| Min / max hold time | 0.025 / 0.2 | 0.025 / 0.2 | ✅ |
| Reset pad | 1/40 | `0.025 + 1/40` window | ✅ |
| Double-jump window | 1.25 s | 1.25 s | ✅ |
| Flip input deadzone (Σ|yaw,pitch,roll|) | 0.5 | `FLIP_INPUT_DEADZONE` 0.5 | ✅ |
| Dodge direction deadzone / axis snap | 0.1 / 0.1 | 0.1 / 0.1 | ✅ |
| Flip initial speed | 500 | 500 | ✅ |
| Side / backward scale / back-X | 1.9 / 2.5 / 16/15 | 1.9 / 2.5 / 16/15 | ✅ |
| Backward classification speed | 100 | 100 | ✅ |
| Flip torque (side / forward) | 260 / 224 | 260 / 224 | ✅ |
| Flip torque time / pitch-lock extra | 0.65 / 0.3 | 0.65 / 0.3 | ✅ |
| Flip Z damp | 0.35 @120 Hz, 0.15–0.21 s | same | ✅ |
| Air-control torque / damping | (130,95,400) / (30,20,50) | identical | ✅ |
| Torque scale | 2π/65536·1000 | identical | ✅ |
| Air throttle | 200/3 | 200/3 | ✅ |
| Auto-flip impulse / torque / time / n.z / roll | 200 / 50 / 0.4 / 0.7071 / 2.8 | identical | ✅ |
| Auto-roll force / torque | 100 / 80 | 100 / 80 | ✅ |
| **Wall jump** (550 uu/s push-off) | **does not exist** (wall behaviour emerges from orientation + sticky) | `WALL_JUMP_HORIZONTAL_SPEED` 550, placeholder | ➕ legacy of the pre-wheel car; see §3 |

### Boost
| Quantity | RocketSim | rusty_bullet | |
|---|---|---|---|
| Accel ground / air | 2975/3 / 3175/3 | same | ✅ |
| Use per second | 100/3 = 33.333 | **33.3** | ⚠ 0.1% low, unexplained |
| Min boost time | 0.1 | 0.1 | ✅ |
| Max boost | 100 | 100 | ✅ |
| Spawn boost | 33.333 (100 in Heatseeker/Dropshot) | 100 (`DriveState::new`, `rb_env` reset) | ⚠ differs; capture-seeded runs override it |
| Unlimited boost | `boostUsedPerSecond = 0` mutator (Heatseeker) | `boost_used_per_second` 0 option (ADR-0031) | ✅ |
| Recharge (10/s after 0.25 s) | Dropshot mutator | none | ❌ |

### Solver / engine (Bullet)
| Quantity | RocketSim | rusty_bullet | |
|---|---|---|---|
| Iterations | 10 (default) | 10 | ✅ |
| Restitution velocity threshold | 0.2 bt = 10 uu/s | `0.2·50` | ✅ |
| ERP (contact) | 0.2 default | 0.2 `ERP2` | ✅ |
| ERP2 (split impulse) | 0.8, threshold 1e30 | 0.8 `ROCKETSIM_ERP2` for push-only rows | ≈ ADR-0038 |
| Relaxation / CFM / linear slop | 1 / 0 / 0 | 1 / 0 / 0 | ✅ |
| Box collision margin | 0.04 bt = 2 uu | 2.0 | ✅ |
| Max angular velocity clamp | π/2 per step | `MAX_ANGVEL` π/2 | ✅ |
| Manifold capacity / breaking factor | 4 / 0.02 | 4 / 0.02 | ✅ |
| Special ball–world manifold (averaged) | yes | yes (ADR-0038/0039) | ✅ |
| Internal-edge utility | yes, on every contact | yes (ADR-0028) | ✅ |
| Sleeping | ball forced asleep only at **exact zero** velocity; cars forced `ACTIVE_TAG` every tick | velocity-threshold sleep: 20 uu/s, 0.5 rad/s, 0.5 s (own placeholders) | ⚠ **repo-only mechanism, uncalibrated** — see §3 |
| Tick rate | 15–120 Hz, `tickTime/(1/120)` scaling | 120 Hz (`TICK_SECS`); `dt` is a parameter; flip uses `TICK_120` scaling | ≈ other rates untested |

### Arena
| Item | RocketSim | rusty_bullet | |
|---|---|---|---|
| Collision geometry | RocketSim's dumped meshes (16 Soccar) + planes | RocketSim's meshes in Bullet BVH order (ADR-0037) + planes | ✅ |
| Floor / ceiling / side walls | z=0 / 2048 / x=±4096 | 0 / 2048 / ±4096 (`arena.rs`) | ✅ |
| Goal mouth / back | open mesh, no soft net | mesh (ADR-0032/0033); mass-spring `net.rs` retained in code, not in `standard_arena` | ➕ dead-ish code, see §3 |
| Goal scored test | `|y| > 5124.25 + r` | none found in `rb_physics_bullet`/`rb_env` | ❌ |

## 2. Mechanics RocketSim has that we do not

Ranked by value to a Soccar verification pipeline (cheapest/most-testable first):

| # | Missing | RocketSim source | Needed for | Notes |
|---|---|---|---|---|
| 1 | **Boost pads** (34 pads, cylinder/box pickup, 4 s / 10 s cooldown, +12 / +100) | `BoostPad.cpp`, `RLConst::BoostPads` | any boost-using scenario; boost is otherwise a free resource | pure, no solver coupling; coordinates in `RLConst.h`; note big-first ordering vs RLBot |
| 2 | **Supersonic state** (≥2200, maintain ≥2100 for ≤1 s) | `Car::_PostTickUpdate` | demos, flip/dodge conditions in replays | ~10 lines |
| 3 | **Bump / demolition / respawn** (bumper x>64.5, curves, 0.25 s cooldown, 3 s respawn) | `Arena::_BtCallback_OnCarCarCollision`, `Car::Respawn` | multi-car matches; `world.rs` explicitly says "bumps and demolitions are not modeled" | needs car–car callback hook |
| 4 | **Goal detection** (`|y| > 5124.25 + r`) and **kickoff reset** (5 spawns, mirrored; random order) | `Arena::IsBallScored`, `ResetToRandomKickoff` | episodes that end on goals; `rb_env` | positional test only |
| 5 | **Car presets** (Dominus, Plank, Breakout, Hybrid, Merc, Psyclops three-wheel) — hitboxes, offsets, wheels, sus-rest | `CarConfig.cpp` | replays with non-Octane cars | all numbers are in §8.2 of the RocketSim report; Psyclops needs its own friction curves and no sticky base |
| 6 | **Boost recharge** (10/s, 0.25 s delay) | `Car::_UpdateBoost` | Dropshot only | trivial |
| 7 | **Ball prediction tracker** | `BallPredTracker` | policies | not physics; belongs in `rb_env` |
| 8 | **Mutator surface** (jump/flip accel, boost accel/strength, ball radius/mass, gravity, bump scale…) | `MutatorConfig` | scenario sweeps | gravity, unlimited boost, car–ball tuning already exist as separate knobs; no unified struct |
| 9 | **Other modes**: Hoops (net mask, launch +1000 @0.265 s), Dropshot (tiles, charge, launch 985 @0.26 s), Heatseeker (steered ball), Snowday (puck) | `Ball.cpp`, `DropshotTiles` | non-Soccar captures | out of the current Soccar scope |
| 10 | **`noRot` ball integration** | `btRigidBody::m_noRot` | speed only | skip unless profiling demands it |

## 3. Things we have that RocketSim does not (review for removal or calibration)

1. **Wall jump** (`WALL_JUMP_HORIZONTAL_SPEED = 550`). Its own doc comment says it substitutes for a missing surface-tracking orientation system. The car now has wheel rays and sticky forces (ADR-0017/0018), so that premise is stale; RocketSim has no wall-jump rule and a wall jump there is just a ground jump along the car's up. Candidate for deletion once the captures confirm.
2. **Velocity-threshold sleeping** (20 uu/s, 0.5 rad/s, 0.5 s). Own placeholders. RocketSim forces the ball asleep only at exactly zero velocity and keeps cars active. A ball rolling at <20 uu/s for 0.5 s is frozen here but keeps moving in RocketSim — a plausible source of drift on slow balls. Candidate to match RocketSim (zero-only) and re-score.
3. **Mass-spring goal net** (`net.rs`, 802 lines, FR-033). Superseded by the real goal mesh in ADR-0033 (`standard_arena` no longer includes it). Check for remaining callers; if none, delete.
4. **`BALL_RADIUS` 93.15 as the ball body radius** vs RocketSim's 91.25 sphere + Bullet margin. Intentional and capture-calibrated (ADR-0034/0036/0045), but worth keeping as an explicit named difference in the spec.
5. **Stale doc in `arena.rs`** (`//!` block still describes analytic planes/cylinders/tori and soft nets that `standard_arena` no longer builds, ADR-0033/0037). Doc-only fix.

## 4. Open numeric checks (cheap to settle)

| Check | RocketSim | rusty_bullet | Action |
|---|---|---|---|
| `BOOST_USED_PER_SECOND` | 100/3 = 33.3333 | 33.3 | Adopt `100.0 / 3.0` unless a capture says 33.3; 0.1 boost per 3 s of burn |
| Taper input sign | `abs(v)` | signed, ≥0 | Documented as deliberate; check whether a reversing-throttle capture exists |
| Spawn boost | 33.333 | `DriveState::new` = 100 (`MAX_BOOST`); `rb_env` seeds 100 | Decide whether kickoffs should start at 33.333 (RocketSim) or 100; Heatseeker/Dropshot are the 100 cases |
| Pushback ERP | 0.2 | 0.1 | Calibrated against `test2.jsonl`; keep, but record as a named difference |
| Non-120 Hz stepping | tickScale on flip damping/torque | `TICK_120` in flip only | Test a 60 Hz step before using `rb_env` at other rates |

## 5. Method and limits

- Repo constants were extracted with `grep -rnE "const [A-Z_0-9]+:"` over `crates/rb_physics_bullet/src` and then read in context for `ground.rs`, `wheels.rs`, `jump.rs`, `world.rs`, `body.rs`, `solver.rs`, `integrate.rs` and `arena.rs`. `boost.rs`, `air.rs` and `roll.rs` were matched by constant only.
- "✅ ported; behaviour not re-read" rows rely on the ADR title and constant match; they have not been re-run against RocketSim.
- Nothing here was executed. Parity of *behaviour* needs a golden-trace comparison (e.g. drive the same input tape through `rb_env` and a RocketSim binding), which this report does not provide.
