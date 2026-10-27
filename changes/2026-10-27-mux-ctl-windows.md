---
category: Added
changelog: rusty-mux-ctl connects to a rusty-mux-server on Windows, with the same commands, output and exit codes as on Linux.
---
## 2026-10-27 - rusty-mux-ctl on Windows

- **Added:** `rusty-mux-ctl` runs on Windows over AF_UNIX (`platform_windows::WindowsUnixStream`). A server that is not there exits 4, as on Linux. `rusty-mux-mcp` builds on it, so it now reaches a Windows server too.
- **Known limitations:** on Windows a bounded call (`run_until`, used by `rusty-mux-mcp`) is not bounded while it connects or while a write blocks on a server that stopped reading, so its deadline holds only for reads. `terminal.pwd` works only for programs that report OSC 7. The interrupt test, the `terminal.pwd`/`terminal.title` test and the `watch` rules test (OSC 7 via `printf`) stay Unix-only. `rusty-mux-mcp`'s stdio tests stay Linux-only. Windows tests run only on CI's Windows leg.
