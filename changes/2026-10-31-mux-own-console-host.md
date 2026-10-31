---
category: Added
changelog: rusty_mux_server on Windows can run its blocks on our own console host with RUSTY_CONSOLE_HOST=own (#1222).
---
## 2026-10-31 - rusty_mux_server: blocks on our own console host

- **Added:** on Windows, `RUSTY_CONSOLE_HOST=own` runs every block's
  program on our own console host, `rusty_conhost`, instead of ConPTY: no
  conhost.exe or OpenConsole.exe in the tree. `conpty`, or the variable
  unset, keeps ConPTY. Any other value stops the server at startup, and a
  Windows without a usable ConDrv fails the block's spawn: there is no
  silent fallback to ConPTY. Linux ignores the variable but still checks it.
- **Fixed:** our console host accepts SetCurrentConsoleFontEx and ignores
  it, as ConPTY does. Windows PowerShell 5.1 calls it at startup and exited
  at once without it. GetConsoleProcessList is answered too, each
  process once, newest first.
- **Added:** a clean stop. `rusty_mux_server::serve_until` serves until a
  given future resolves, and the Windows binary stops that way on Ctrl+C or
  Ctrl+Break. However serving ends, every block's program is ended and, on
  our host, the server waits until each console has ended the other
  programs attached to it (5 s after the close notification at most), with
  one 6 s deadline for all blocks together; past it, it logs and returns.
  No block starts meanwhile. ConPTY and Linux have nothing to wait for.
- **Changed:** `rusty_mux_server::Config` gains a public `console_host`
  field (`ConsoleHost::ConPty` or `Own`), so code building a `Config` by
  struct literal must set it. On Windows, Ctrl+C or Ctrl+Break now stops
  the server cleanly instead of ending it at once.
- **Known limitations:** our host runs inside the server, so a server that
  is killed, crashes or has its console window closed sends no close
  notification: a program still attached to a block's console besides the
  shell, such as one started with `start /b`, can outlive it (design V15).
  On our host, closing a block still ends its program after the mux's 1 s
  grace; other programs on the block's console are ended by the host 5 s
  after the close notification. The echo mode is still reported as unknown
  on either host.
