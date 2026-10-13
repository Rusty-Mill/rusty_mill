---
category: Changed
changelog: `rb_rl_game_probe --analyze` takes `--no-names`, which prints actor ids and teams instead of player display names, and the live-test runner uses it so its report holds no display name.
---
## 2026-10-13 - rb_rl_game_probe: --no-names

- **Added:** `rb_rl_game_probe --analyze FILE --no-names` leaves the display names out of the two places the analysis prints them (the `PlayerSpawn` line and each car's summary line). Without the flag the output is as before; the two outputs differ only in those names (checked on the committed `live_freeplay_slice.bin`, which has one name).
- **Changed:** the live-test runner (`rusty_bullet/tools/rl_live_test`) runs the analysis with `--no-names`. Its report used to rely on a regex in the PowerShell script to rewrite the human's name, which only matched two formats; now nothing needs to match. The regex stays as a second pass and for the bots' readable names.
- **Verified:** unit tests that a hidden name reaches no line and that the flag is off by default; run on the real fixture, `--no-names` output has zero occurrences of the name and differs from the default in exactly those lines.
- **Known limitations:** the PowerShell change is one added argument and was **not run** (no PowerShell here; Windows only, needs the game). Recordings and screenshots are still unscrubbed, as documented. The runner's other portability limits (Epic app id, `C:\Program Files`, six `..` to the repo root) are unchanged (board b34).
