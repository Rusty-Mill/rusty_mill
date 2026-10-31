---
category: Added
changelog: rusty_yirp can run PTY sessions on our own Windows console host with `RUSTY_CONSOLE_HOST=own` (host PR 12, #1222).
---
## 2026-10-31 - rusty_yirp: sessions on our own console host

- **Added:** `sessionmgr-pty` gains `PtyOptions::console_host` and
  `ConsoleHost`, read from `RUSTY_CONSOLE_HOST` (`conpty`, the default, or
  `own`). With `own`, a Windows session's process runs on `rusty_conhost`
  instead of ConPTY. Any other value, or a Windows that can't run our host,
  fails the session's start with an error; there is no silent fallback.
- **Added:** on our host the session's output ends when its process exits,
  as on ConPTY: a watcher hangs the console up, so processes it left
  attached get CTRL_CLOSE. The worker then waits up to 6 s
  (`PtySession::shut_down`) so our host can end them before the worker exits.
- **Known limitations:** the default stays ConPTY until the Phase B gate.
