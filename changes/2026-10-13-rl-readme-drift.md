---
category: Changed
changelog: Docs only: the RLBot core and game-bridge READMEs no longer carry stale test counts or method counts, and the `rb_rl_bridge` usage lists `--no-spawn`, `--record-in` and `--record-out`.
---
## 2026-10-13 - rb_rlbot_core, rb_rl_game_bridge: README drift

- **Changed (docs):** the `rb_rlbot_core` README no longer states test counts (it said 8 and 6; there are more). The `rb_rl_game_bridge` README's `rb_rl_bridge` usage line now shows `--no-spawn`, `--record-in FILE` and `--record-out FILE`, and its mapping table no longer says "five" `TcpMessenger` methods. Counts are dropped rather than updated so they cannot go stale again.
