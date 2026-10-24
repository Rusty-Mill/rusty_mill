---
category: Added
changelog: ADR-0085, sim runs and captures shown in the existing replay viewer (design for the b5 to b7 slices).
---
## 2026-10-24 - Rocket League: sim viewer design (ADR-0085)

- **Added:** `rusty_bullet/docs/adr/0085-sim-viewer-extends-replay-viewer.md`. Sim runs and BakkesMod captures will play in `replay-viewer` through a `PhysicsFrame` to `Scene` adapter that reads ADR-0005 JSONL, with the port's candidate drawn as a ghost over the recording. `rb-verify` writes the candidate file, so the viewer never links the physics and the sim core gains no viewing code. Three slices: render one run (b5), velocity arrows and an input timeline (b6), contact points and the real arena mesh (b7).
- **Known limitations:** design only; no code yet. `Scene` gains three optional fields across the slices, and 120 Hz pages are larger than replay pages.
