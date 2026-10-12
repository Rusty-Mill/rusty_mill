---
category: Added
changelog: `rb_rl_game_probe --census-in/--census-out` and `rb_rl_game_bridge::Census` tally the message and command tags in a recording and flag any outside the known unions, to find game traffic the format note does not cover.
---
## 2026-10-12 - rb_rl_game_bridge: tag census

- **Added:** `Census` (and `GAME_MESSAGES`, `CONTROLLER_COMMANDS`) tallies the wrapper tags of game-to-RLBot messages and RLBot-to-game commands in a recording, names each against the unions in `RL-GAME-SOCKET-FORMAT.md`, and reports tags outside them as `UNKNOWN`. `rb_rl_game_probe --census-in FILE --census-out FILE` prints it. It reads only tags, so it works on frames whose bodies the decoders cannot read, and it separates "known but not decoded" (`Other`) from "not in the format".
- **Evidence so far:** the committed live Freeplay slices hold 13 of 22 message tags and 2 of 13 command tags, none outside the unions. That is weak: no goal, pickup state change, destroy, pause or render command was recorded.
- **Known limitations:** a clean census says the traffic is covered by the DLL's unions, not what the game would accept beyond them; `Exec` console commands are not inspected.
