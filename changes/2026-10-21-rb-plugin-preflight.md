---
category: Added
changelog: tape bot batch checks the capture plugin against the installed BakkesMod and rebuilds it when BakkesMod, its SDK library or the plugin source changed; warns when the game outran RLBot core.
---
## 2026-10-21 - rb_tape_bot: capture plugin preflight

- **Added:** `tools/rb_tape_bot/preflight.ps1`. A plugin DLL built against an older BakkesMod SDK can crash the game
  after a game or BakkesMod update, and the only guard was a README line saying to rebuild. The script records what
  the installed DLL was built from (BakkesMod `version.txt`, the SHA-256 of `pluginsdk.lib`, the SHA-256 of the
  plugin source) in `rusty_bullet_capture.stamp.json` beside it, compares that with what is installed now, and on a
  difference rebuilds, backs the old DLL up as `rusty_bullet_capture.bm<version>.dll.bak` and installs the new one.
  `-Check` reports and exits with an error without changing anything. An unstamped DLL (built by hand) counts as
  stale, so the first run rebuilds once. It refuses to replace the DLL while Rocket League is running unless `-CloseGame` is given, in which case it stops
  the game, and only when a rebuild is actually needed (never with `-Check` or on a current plugin). It leaves the
  installed DLL and stamp untouched when the build fails or the game does not exit, and treats an unreadable stamp
  as an error.
- **Added:** warnings, never failures, when Rocket League was updated after RLBot core was installed (core's bridge
  is closed source and talks to the game) or after BakkesMod last updated.
- **Changed:** `run_batch.ps1` runs the preflight after it starts BakkesMod and before it builds and launches the
  game, with `-CloseGame` because RLBot core kills and relaunches the game for every batch anyway.
  `-NoPreflight` skips it. `-ScoreOnly` never runs it.
- **Added:** `tools/rb_tape_bot/test-preflight.ps1` (15 cases, stubbed `cmake`, no game or desktop), run by the
  `rb-tape-verifier` CI job.
- **Known limitations:** it cannot accept BakkesMod's own update prompt, which needs a click. If the prompt is still
  open when the batch starts, the preflight builds against the old SDK and stamps that, and the next run rebuilds once
  the update lands. It does not check that the game's hook name still fires; only a capture shows that. It does not
  update RLBot core; it only warns.
