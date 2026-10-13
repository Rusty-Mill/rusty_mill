---
category: Changed
changelog: `rb_rlbot_core` refuses state-setting values beyond sane bounds, and a simulated match that has ended now ends the run (`RunEnd::MatchOver`).
---
## 2026-10-13 - rb_rlbot_core: state-setting bounds, and a finished match ends the run

- **Changed:** a `DesiredGameState` with a location beyond ±20,000 uu, a velocity beyond ±20,000 uu/s or an angular velocity beyond ±100 rad/s on any axis is refused whole, as a nonfinite one already was (`InvalidInput`; core reports `Ignored` and keeps serving). Before, any finite value was accepted, so a buggy bot could hand the simulation, or the real game through `rb_rl_game_bridge`, a value it cannot hold. The bounds are far outside anything legal (the arena is about 8200 x 10300 x 2000; a ball is capped at 6000 uu/s, a car at 2300). Refused, not clamped: a clamped request is a different request. Boost is unchanged (the simulation already clamps it).
- **Added:** `RunEnd::MatchOver` and `MatchBackend::finished()` (default `false`). A `Simulation` whose match reaches the `Ended` phase now ends the run after sending the last packet, instead of stepping on until a client stops it. A real game never reports it. Adding a `RunEnd` variant is breaking for an exhaustive `match` on it; the repo has none.
- **Verified:** boundary tests for each bound (at the bound accepted, just beyond refused, `f32::MAX` refused; mutation-checked); a stepped stand-in ends the run after exactly its last packet (26 packets, mutation-checked); a full five-minute simulated match with one goal ends with `MatchOver` and an `Ended` last packet (an `#[ignore]`d test, about 20 s).
- **Not done (board b25, decided):** agent-id squatting (RLBot has no authentication either; any local process can already join), lockstep double input (the wire has no frame number to tag, and it only matters after a timeout), and lockstep with no ready bot running flat out (by design for lockstep).
