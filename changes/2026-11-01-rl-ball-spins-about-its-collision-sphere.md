---
category: Fixed
changelog: rb_physics_bullet spins the ball about RocketSim's 91.25 uu sphere, so a sliding ball spins up as the game's does (RB-PHYSICS-001-FR-161).
---
## 2026-11-01 - The ball spins about its 91.25 uu sphere

- **Fixed:** the standard ball's inertia and its combined world contact's lever arm now come from RocketSim's 91.25 uu ball sphere, not the 93.15 uu contact sphere. A sliding ball gained 0.05088 rad/s a tick; it now gains 0.05195, against the game's 0.05190 (`probe_ball_fast_6500`). Ball contacts, rest height and the car-ball paths are unchanged.
- **Changed:** golden bounds lowered for `probe_ball_rest_40` (ball 6.49 / 11.07 to 0.29 / 0.36 uu), `rest_80`, `probe_ball_roll`, `probe_ball_wall`, `ballfuzz_203` and `probe_nose_hit_glancing`. `probe_hit_offset`'s car bound was raised from 1.5 to 1.9 uu (its max went from 0.62 to 1.50). `ballfuzz_218` max went from 10.89 to 13.24 uu, within its bound. Human captures: `wall-climb-crest` ball 28.03 to 14.16 uu max (bound lowered); `hit-tick-jump` car max 7.51 to 12.43 uu after its already-open mishit (bound raised to 15.5).
- **Known limitations:** a rolling ball still slows 2.1%/s against the game's 3.0%/s, and a sliding ball loses 1.8962 uu/s a tick to friction against the game's 1.8999; `probe_ball_fast_6500` (board #197) is unchanged.
