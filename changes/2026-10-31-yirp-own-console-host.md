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
- **Added:** our host runs inside the worker, so before exiting the worker
  waits up to 6 s (`PtySession::shut_down`) for it to end processes the
  session left attached. The daemon's graceful close now lets an
  acknowledging worker exit by itself for up to 7 s before forcing it, and
  a worker that fails to publish a started session ends it first.
- **Known limitations:** the default stays ConPTY until the Phase B gate.
  With something left attached, a session's exit is recorded up to 2 s
  later than on ConPTY. A worker that crashes takes our host with it, and
  the processes it left attached keep running without CTRL_CLOSE.
