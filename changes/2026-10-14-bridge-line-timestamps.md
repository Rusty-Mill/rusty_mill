---
category: Changed
changelog: Every line `rb_rl_bridge` prints now starts with the seconds since it started, so a log shows when the game dialled and when it first spoke.
---
## 2026-10-14 - rb_rl_bridge: per-line timestamps

- **Changed:** each line of the bridge's output is led by `[  12.345s]`, the seconds since the first line was printed (the bridge starts a moment before it, and the live-test runner starts it just before the game). The lines themselves are unchanged, so the runner's wait for `waiting to speak` and the tests, which look for text within a line, are unaffected.
- **Why:** the 2026-10-13 live runs gave "about 45 s from launch to `the game connected`" only from a log file's last-write time. With stamps, the gap between `waiting to speak` (the game dials) and `the game connected` (its first message decodes) is read straight off the log, and so are the timings in the unexplained run (b40).
- **Verified:** a unit test of the stamp format; `game_finding.rs` already runs the real binary and reads its lines in order.
- **Known limitations:** seconds since the process started, not wall-clock, so it cannot be lined up against the game's own logs without knowing when the bridge was started. The stamp is not applied to the probe (`rb_rl_game_probe`) or to the lines on stderr.
