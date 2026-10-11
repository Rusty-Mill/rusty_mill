---
category: Added
changelog: `rb_chaser_bot`, a runnable RLBot bot that plays `rb_env`'s scripted chaser through `rb_rlbot_bridge`.
---
## 2026-10-11 - rb_chaser_bot: the chaser as an RLBot bot

- **Added:** `rb_rlbot_bridge::chaser(team)` (the `rb_env` chaser as a `Policy`), the `rb_chaser_bot` binary and its `chaser.bot.toml` manifest. `rb_env` is now a normal dependency of the bridge.
- **Known limitations:** not run against the real game; tests cover the policy and one packet answered over the socket.
