---
category: Added
changelog: `rb_rlbot_bridge`: runs an `rb_env` policy as an RLBot bot (game packet to `PhysicsFrame`, `ControllerInput` to `ControllerState`) on `rb_rlbot_client` (RLBot port stage 5).
---
## 2026-10-10 - rb_rlbot_bridge: an rb_env policy plays in the real game (RLBot port stage 5)

- **Added:** `crates/apps/rocket_league/crates/rb_rlbot_bridge`: `observation` (a `GamePacket` as the `PhysicsFrame` `Env::reset` takes), `controller_state` (a `ControllerInput` as RLBot's `ControllerState`), and `Policy` / `PolicyBot` / `run_policy` (one policy per car, one input per packet) over `rb_rlbot_client`. Rotations reuse `rb_scenario::rotator_to_quat`. Seven tests, including that the chaser drives straight at a ball it faces from an observed packet; a negated yaw and a throttle/steer swap each fail them.
- **Known limitations:** not run against the real game (CI cannot run Rocket League). A policy sees only a `PhysicsFrame`: no boost pad timers, score or match clock beyond the timestamp, and no previous inputs. No binary yet; the crate is the library.
