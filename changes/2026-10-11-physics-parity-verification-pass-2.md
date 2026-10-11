---
category: Changed
changelog: Physics parity table: caption-based rows checked against the committed real-game captures; ball spin cap, minimum boost time, ball friction and rolling threshold settled.
---
## 2026-10-11 - Physics parity table: verification pass 2 (real game)

- **Changed:** `docs/research/PHYSICS-PARITY-VS-ROCKETSIM.md`. The 73 committed game captures (18,146 frames, 120 Hz) were read directly. Settled: the ball angular cap is 6.0 rad/s (1,715 frames saturate there, none above; 2π excluded); the minimum boost burn is 12 ticks (matching RocketSim and `rb_physics_bullet`, not the video's 13); sliding friction is about 228 uu/s² (0.35 × 650); the rolling threshold is 547.5 uu/s = 6 × 91.25, not the caption's 565; restitution 0.6 is confirmed; car speed and angular caps are confirmed. Pad pickup is already fitted to the game in ADR-0072 (origin reach 176 uu small, about 208 big, 160 high); a cross-check on the captures agrees and contradicts RocketSim's 120 uu small-pad box. Findings F1 to F3 are closed or resolved.
- **Correction to pass 1:** pad radii agreeing with RocketSim was not independent evidence, since RocketSim took them mostly from the same video.
- **Known limitations:** the 6000 uu/s ball cap, big-pad and airborne pickups, ball sleep thresholds and the audio-only rows remain untested. No physics code was changed; the analysis scripts are not committed.
