---
category: Added
changelog: Six tape-bot ball probes for the 6000 uu/s speed cap and for the speed below which a rolling ball freezes.
---
## 2026-10-24 - Ball speed-cap and rest probes

- **Added:** `gen_probes.py` round 6 writes `probe_ball_fast_4500` and `probe_ball_fast_6500` (the ball launched along the floor below and above the 6000 uu/s cap) and `probe_ball_rest_{10,20,40,80}` (the ball already rolling without sliding at each speed, held 5 s). The committed captures reach only 3,541 uu/s and never show a ball at rest. Rolling decays only about 3%/s, so a faster start never nears rest; the ladder brackets the freeze speed instead. Every ball state sets its spin explicitly and uses no settle ticks.
- **Known limitations:** scenarios only; the game's cap and rest thresholds stay unmeasured until these run on the game machine. In the port, 6500 is clamped to about 6000 at once, and its own thresholds (20 uu/s, 0.5 rad/s, 0.5 s) freeze the 10 and 20 rungs and let 40 and 80 roll.
