---
category: Added
changelog: replay-viewer draws velocity arrows and a recorded-versus-simulated input strip for physics runs (ADR-0085, slice b6).
---
## 2026-10-24 - Rocket League: velocity and inputs in the sim viewer

- **Added:** physics scenes give each `SceneCar` an optional velocity (`v`) and controller input (`input`: five axes, `null` where the source has none, plus a jump/boost/handbrake bit set). Both are omitted from replay scenes, which are unchanged.
- **Added:** the page draws a velocity arrow on each car (Settings → velocity) and an input strip above the timeline for the followed car, or the first recorded car: throttle, steer, pitch, yaw and roll lanes and jump, boost and handbrake bars, the recording solid and the sim run dashed beside it.
- **Added:** gaps stay gaps in the strip: a frame with no matched candidate or no input breaks the lines, and a button bar ends at the next frame. `tests/input_strip.mjs` checks this and runs in the viewer's CI job.
- **Known limitations:** the strip shows one car at a time. Contact points and the real arena mesh are b7.
