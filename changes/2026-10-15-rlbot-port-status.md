---
category: Changed
changelog: RLBOT-PORT.md states what is built (build order steps 1 to 3) instead of "scope only, no code".
---
## 2026-10-15 - RLBOT-PORT.md status

- **Changed (docs):** `docs/design/RLBOT-PORT.md`. The status line said "scope only, no code" although the wire messages, the simulated core and the game bridge are on main. The status now says so, each build-order step is marked built, partly built or not started, and the first slice records that repeatability is covered by a component-level test (`the_same_bot_gives_the_same_match_every_time`) while the process-level `rb_chaser_bot` ball-touch test is still missing.
- **Checked against the tree, not run:** crate names and the `rb_rlbot_core` README. `rb_rlbot_server`, `rb_rlbot_launch` and `rb_rlbot_match` do not exist; the server role is in `rb_rlbot_core`.
- **Known limitations:** step 5 is marked built from reading `sim.rs` and its tests, not from running them or comparing with the game; match info (gravity, game speed) is ignored by the core. Docs only; no code changed.
