---
category: Fixed
changelog: rusty_win32 process_list() no longer fails when the console's process list grows between calls.
---
## 2026-11-03 - rusty_win32 process_list() headroom and bounded retry

- **Fixed:** `console::process_list()` sized its buffer to exactly the count
  `GetConsoleProcessList` reported, so a process attaching in between made the second call
  too small and it returned `ERROR_INSUFFICIENT_BUFFER` (a Windows CI flake, card #525).
  Each retry now leaves twice the room asked for, with up to four attempts. The grow logic is
  a helper with deterministic tests that inject the list sizes.
