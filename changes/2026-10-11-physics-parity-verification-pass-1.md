---
category: Changed
changelog: Physics parity table: low-confidence (auto-caption) rows checked against RocketSim's source, arithmetic and the GDC slide text; statuses and four findings recorded.
---
## 2026-10-11 - Physics parity table: verification pass 1

- **Changed:** `docs/research/PHYSICS-PARITY-VS-ROCKETSIM.md`. Every row resting on auto-captions, and the rows flagged unverified, now carries a `V1` tag (✔ agrees with the cited source, ✎ corrected, ? still unknown) with a RocketSim `file:line`, arithmetic or slide-text basis, plus a section listing rows whose status changed, four findings and what remains unknown. Corrections: the ball spin cap note ("60 RPM, rounded") is unproven, the jump impulse unit is uu/s, the big boost pad radius "28" is a garble of 208, and the minimum boost duration differs by one tick between the video (13) and RocketSim's logic (12).
- **Known limitations:** no row is checked against the real game; that is pass 2 (captures). Audio-dependent digits are unresolved. No physics code was changed. The GDC deck's vehicle-tuning slides contain no constants, so a GDC-attributed supersonic claim could not be confirmed.
