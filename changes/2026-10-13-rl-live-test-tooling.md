---
category: Added
changelog: Live-test tooling for `rb_rl_bridge` against the real game on Windows/Epic: `rb_rl_demo_count` (tells a demolition from a destroy-and-respawn in a recording, no display names), `rusty_bullet/tools/rl_live_test` (one-command launch, menu walk, report), and a findings note; the Epic launch recipe now says to kill the game before it logs in.
---
## 2026-10-13 - rb_rl_game_bridge: live-test tooling and the first live findings

- **Added:** `rb_rl_demo_count FILE` in `rb_rl_game_bridge`. It reads a `rb_rl_bridge --record-in` recording and lists, per map, every player spawn, every destroy of a player actor and every stretch a car was reported Demolished (`PlayerPhysicsChange` state 5). Players show as actor id and team only, never by name. Five unit tests; checked against a real recording (18 demolitions, each 3.00 s, the same actor back every time).
- **Added:** `rusty_bullet/tools/rl_live_test` (`rl-lib.ps1`, `run-live-test.ps1`, README): launches the game the way RLBot does, walks the menus offline, plays two chaser bots against each other, pauses once, reloads the map once, then writes a scrubbed `report.txt`. Windows only, needs an unlocked desktop, coordinates calibrated at 2560x1440. Not part of CI.
- **Added:** `rusty_bullet/docs/research/RL-LIVE-TEST-2026-10-12.md`: demolition behaviour, `BroadcastHeld`/`BroadcastResumed` timings, the census against the Freeplay slice (tags 6, 13, 14, 15, 16, 17 are new evidence; none outside the known unions), and what is still open.
- **Changed:** the `rb_rl_game_bridge` README's Epic recipe says to close the game as soon as a new `Init: Command line:` line appears, not after the menu: a game that has logged in seems to have spent the login code and the relaunch sticks at "click to start".
- **Known limitations:** a demolition of the human's car was not observed; what lifts a `BroadcastHeld` after a map load is not established; late in the day launches began failing with an empty `Launch.log` for a reason not found (the note lists the guesses tried).
