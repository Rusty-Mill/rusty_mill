---
category: Added
changelog: Three tape-bot ball probes for the 6000 uu/s speed cap and for a ball rolling to rest.
---
## 2026-10-24 - Ball speed-cap and rest probes

- **Added:** `gen_probes.py` round 6 writes `probe_ball_fast_4500` and `probe_ball_fast_6500` (the ball launched along the floor below and above the 6000 uu/s cap) and `probe_ball_rest` (a 600 uu/s roll held for 10 s so the ball stops). The committed captures reach only 3,541 uu/s and never show a ball coming to rest, so neither the cap nor the rolling and sleep thresholds have been measured in the real game.
- **Known limitations:** scenarios only. They need a run on the game machine; whether the game accepts a 6500 uu/s state-set is itself part of what the run shows.
