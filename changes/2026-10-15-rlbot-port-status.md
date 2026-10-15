---
category: Changed
changelog: RLBOT-PORT.md states what is built (build order steps 1 to 3) instead of "scope only, no code".
---
## 2026-10-15 - RLBOT-PORT.md status

- **Changed (docs):** `docs/design/RLBOT-PORT.md`. The status line said "scope only, no code" although the wire messages, the simulated core and the game bridge are on main. The status now says so, each build-order step is marked built, partly built or not started, and the first slice records that its done-criteria (ball touched within a fixed time, two identical lockstep runs) have no test yet.
- **Checked against the tree, not run:** crate names and the `rb_rlbot_core` README. `rb_rlbot_server`, `rb_rlbot_launch` and `rb_rlbot_match` do not exist; the server role is in `rb_rlbot_core`.
- **Known limitations:** the contents of the packets (boost pads, scores, match phases) were not checked, so step 5 is marked "partly" on the strength of the state-setting code only. Docs only; no code changed.
