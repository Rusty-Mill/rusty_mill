---
category: Fixed
changelog: A flaky `rb_rl_game_bridge` end-to-end test no longer races the stand-in game's close, `FrameDecoder` drains in linear time, and stale docs in the RLBot crates are corrected.
---
## 2026-10-13 - rb_rl_game_bridge, rb_rlbot_wire: flaky test and docs drift

- **Fixed:** `a_bot_plays_through_core_in_a_game_that_is_not_the_simulation` failed about 10% of local runs: the stand-in game closed as soon as it saw the bot's ball placement, which the bot sent after its second packet, so the bot sometimes saw two packets, not three. The bot now places the ball after its third packet, so three is guaranteed. The assertion is unchanged; 0 failures in 60 runs (was 6 in 60).
- **Changed:** `rb_rlbot_wire::frame::FrameDecoder` keeps a read offset instead of splitting the buffer on every frame, so draining a large buffer (a recording of 100,000 messages) is linear, not quadratic. Behaviour is unchanged; a new test drains 20,000 frames from one push and checks `finish` around a half frame. Mutation-checked.
- **Changed (docs):** `InterfaceMessage::from_payload` says unmodelled members are `Other`, not `UnknownUnion`; the `rb_rl_game_bridge` Cargo description no longer says the game's socket format is not implemented; the README no longer carries a test count that had gone stale.
