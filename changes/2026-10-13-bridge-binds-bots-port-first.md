---
category: Changed
changelog: `rb_rl_bridge` takes the bots' port before it launches or waits for the game, so a port clash fails at once and bots started early wait instead of being refused; `rb_rlbot_core` gains `Core::from_listener`.
---
## 2026-10-13 - rb_rl_bridge: bind the bots port first

- **Changed:** `rb_rl_bridge` binds the bots' port (`--port`, default 23234) right after the game port, before `--launch` and before waiting for the game. A taken port now fails immediately with the port named; before, it failed only after the game had connected, leaving Rocket League in `-rlbot` mode with nobody to talk to. Bots that start while the game is loading connect and wait in the listener's backlog, where before they got `ConnectionRefused` (seen in the first live runs).
- **Added:** `rb_rlbot_core::Core::from_listener(listener, backend, pacing)`; `Core::bind` is now `from_listener(TcpListener::bind(addr)?, ...)`. Additive, nothing removed.
- **Verified:** a test connects a bot before the core exists and plays it through; a bin test checks a taken port is refused by name. Run by hand: a taken `--port` exits 1 with the message before any wait, and a connection to `--port` succeeds while no game is attached.
- **Known limitations:** how long an early bot waits is the bot's own timeout (not measured against `rb_chaser_bot`). The first local connection to the game port is still taken as the game (board b23, second half).
