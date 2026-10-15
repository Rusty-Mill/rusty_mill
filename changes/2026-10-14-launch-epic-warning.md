---
category: Changed
changelog: `rb_rl_bridge --launch` now warns when the game is in an Epic Games install, where a direct launch has no login arguments and the game will not connect, and points to the live-test runner.
---
## 2026-10-14 - rb_rl_bridge: `--launch` warns about an Epic install (b31)

- **Changed:** `needs_epic_login(path)` is true when a directory of the game's path is named `Epic Games` (either path style, any case, whole name only). `rb_rl_bridge --launch` prints `EPIC_LAUNCH_WARNING` first when it is, then launches as before. Only the warning is new; the launch itself is unchanged.
- **Decision (b31):** the Epic kill-before-login relaunch is not ported into the bridge. The live-test runner does it, live-verified, and only closes a game that is a direct child of the launcher it started. Doing that from Rust needs a process handle, which the standard library does not give and this crate forbids (`unsafe_code = "forbid"`); the alternatives are a new dependency or killing by PID through a shell, which has the PID-reuse race the runner avoids. It would also hold the login arguments in the bridge. Neither could be tested without Windows and Epic.
- **Verified:** a unit test of the check (Windows and Unix separators, case, a part of a directory name, the file name, Steam). The warning line itself is a one-line call and is not run against a game.
- **Known limitations:** a path heuristic: an Epic install in a directory not named `Epic Games` gets no warning, and a launch that works (a game given its login arguments some other way) still gets one. The warning does not stop the launch.
