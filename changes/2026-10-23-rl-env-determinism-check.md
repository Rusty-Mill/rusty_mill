---
category: Added
changelog: rb_env has a determinism check: two environments fed the same start and inputs give bit-identical frames, with and without match flow, fresh or reused.
---
## 2026-10-23 - rb_env: determinism check

- **Added:** `rb_env/tests/determinism.rs` compares every frame of two `Env`s bit for bit (raw `f32` bits, so -0.0 and NaN count): a 20 s scripted tape for four cars that drives, steers, boosts, handbrakes, jumps, dodges and air-rolls; the same tape on an `Env` that ran something else first and was reset, against a fresh one; and a chaser match with match flow and a one-minute clock, where the match state is compared too. All three pass with no sim changes. A failure names the first divergent tick. This is the first step for rollback and the loopback netcode experiment.
- **Known limitations:** both environments are in one process on one machine, so this says nothing about determinism across platforms, compilers or optimisation levels. Restoring a run from an observed `PhysicsFrame` (rollback from a snapshot) is not checked: a frame does not hold every piece of car state.
