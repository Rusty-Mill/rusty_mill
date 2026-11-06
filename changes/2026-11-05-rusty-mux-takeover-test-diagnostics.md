---
category: Fixed
changelog: rusty_mux: server tests wait for a command's output as text, not in the window title cmd.exe sets, which raced the takeover screen checks on Windows (test only).
---
## 2026-11-05 - Takeover screen tests wait for real output on Windows

- **Fixed:** `run_and_wait` took `MARK2` in `cmd.exe`'s window title (OSC 0, "cmd.exe - echo  MARK2", sent before the output) as the command's output. A takeover right after could checkpoint a screen without it, so the two takeover screen checks failed on fast Windows runners, on main as in #1532's queue run. It now waits for the marker outside OSC strings. A regression test feeds the bytes the runner sent.
- **Changed:** the reconnect test's screen assertion keeps its condition and, on failure, prints the checkpoint's screen, the old client's screen and its raw output.
- **Known limitations:** test only; no server behaviour changes.
