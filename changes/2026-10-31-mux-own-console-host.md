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
- **Changed:** `rusty_mux_server::Config` gains a public `console_host`
  field (`ConsoleHost::ConPty` or `Own`), so code building a `Config` by
  struct literal must set it.
- **Known limitations:** on our host, closing a block still ends its
  program after the mux's 1 s grace; other programs on the block's console
  are ended by the host 5 s after the close notification. The echo mode is
  still reported as unknown on either host.
