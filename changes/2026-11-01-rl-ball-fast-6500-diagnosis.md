---
category: Added
changelog: rb_verify_cli guards the ball's side-ramp ride and wall bounce from the game's own state, and records why probe_ball_fast_6500 still misses (board #197).
---
## 2026-11-01 - Ball side-ramp guard and the fast_6500 cause

- **Added:** `from_the_games_state_the_port_rides_the_side_ramp_and_bounces_off_the_wall` in `rb_verify_cli`'s golden captures. Started from `probe_ball_fast_6500` tick 141, the port follows the game's ball for 20 ticks to 0.23 uu and 3.44 uu/s, through the side ramp and the wall bounce (1500 to -210 uu/s).
- **Changed:** the golden case comment no longer blames the side wall's top seam. The scenario's remaining ball error (5.4 uu mean, 55.2 uu max on `e1a47953`) comes from sliding friction: the port slows a sliding ball by 1.8958 uu/s a tick, the game by 1.8999 (`probe_ball_fast_4500`, `_6500` and `probe_ball_roll` agree). The 0.3 uu lead this builds by tick 136 makes the port leave the ramp's top facet a tick early.
- **Known limitations:** the cause of the 0.2% friction gap is unknown, so the port is not tuned to it and the case's bound is unchanged.
