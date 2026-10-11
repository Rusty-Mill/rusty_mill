---
category: Added
changelog: Research note `docs/research/RL-SERVER.md` on Rocket League netcode, a server clone's needs, legal posture and a phased roadmap.
---
## 2026-10-11 - Rocket League server research note

- **Added:** `docs/research/RL-SERVER.md`. Re-reads the GDC 2018 slides (120 Hz fixed tick, per-frame input buffer with upstream and downstream throttles, frame-numbered prediction with revert-and-resimulate, no server-side lag compensation), lists what a server needs beyond the simulator, compares legal postures (recommends spec-only), links it to the simulator and the RLBot port, and ends with a phased roadmap and a first experiment (loopback reconciliation).
- **Known limitations:** research only, not legal clearance and not implementation authorization. The note acknowledges that the repository already vendors game-derived arena collision meshes (see `THIRD_PARTY_NOTICES.md`) and leaves licensing and release questions open. It separates netcode correctness from simulator fidelity: a loopback client and server share the same fidelity error, so only real-game captures test fidelity. The spoken talk was not watched, only the extracted slide text. The note records that RB-RESEARCH-S001 and ADR-0001's "periodic snapshots" and "reconciliation smoothing" statements are not supported by the slide text and need checking against the video. The server send rate rests on one unverified forum quote. The unimported server-clone branch was not read.
