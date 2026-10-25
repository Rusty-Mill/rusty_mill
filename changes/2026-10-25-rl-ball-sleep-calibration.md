---
category: Fixed
changelog: Ball sleep calibrated to the game (40 uu/s, 1.0 rad/s, 2.0 s) and the ball capped before a tick moves it; six ball probes become golden fixtures.
---
## 2026-10-25 - Ball sleep thresholds calibrated to the game

- **Fixed:** `rb_physics_bullet`'s sleep thresholds (`RB-PHYSICS-001-FR-037`) were placeholders (20 uu/s, 0.5 rad/s, 0.5 s). The game's `probe_ball_rest_*` captures freeze a ball rolling at 10, 20 or 40 uu/s 2.017 s in, all on the same tick, and leave one at 80 uu/s rolling: Bullet's defaults at 50 uu per metre, 40 uu/s, 1.0 rad/s and 2.0 s. Ball error max on rest_10 / 20 / 40: 13.6 / 22.6 / 113.0 uu, now 1.1 / 1.6 / 11.1.
- **Fixed:** the ball is also capped at the start of a tick (FR-061 amended). The game moves a ball set at 6500 uu/s by 5996.6 / 120 uu on its first tick; the port moved it 6496.6 / 120. `probe_ball_fast_6500` ball error mean 10.3 -> 5.4 uu, within 0.3 uu until the side wall.
- **Added:** the six captures as golden fixtures with bounds in `golden_captures.rs`; `fixture_checks.py ball-rest` reads the freeze off them; `probe_ball_rest_50` and `_60` for the next game session.
- **Changed:** `wob_0_200`'s ball and `wob_0_500`'s car bounds rose (18.4 / 70.2 and 7.2 / 33.4 uu): the old 0.5 s timer froze those non-parity drops early.
- **Known limitations:** the recordings bound the thresholds only from below (above 39.98 uu/s and 0.438 rad/s); the 80 uu/s ball that is never frozen says the linear threshold is at most 72.95 uu/s or the angular one at most 0.80 rad/s, not which; cars share the ball's thresholds unmeasured. rest_40 keeps 11 uu: the port's rolling ball slows 2.1%/s against the game's 3.0%/s, so it drops under 40 uu/s about 0.25 s late. fast_6500's 55 uu max is the rebound off the side wall's top seam (the game leaves at -210 uu/s in x, the port nearer -60).
