---
category: Changed
changelog: rusty_term's ConPTY rollback backend runs on platform-windows' ConPTY wrapper instead of its own FFI copy.
---
## 2026-11-10 - ConPTY rollback backend on platform-windows

- **Changed:** `[console] host = "conpty"` now spawns through `platform_windows::WindowsPty`, the wrapper rusty_mux_server uses, and builds the shell's command line with the same `shell_command` as our own host, so both backends quote launcher arguments the same way (#1279 rules). rusty_term's raw ConPTY FFI is deleted. Our own host stays the Windows default; ConPTY stays rollback only, with no auto-fallback.
- **Fixed (review):** closing a ConPTY pane no longer blocks: the hang-up and the 2 s grace before a kill run off the closing thread. An unquoted shell path with spaces that names a file now runs that file on both Windows backends, not a shorter existing prefix such as `C:\Program.exe`.
- **Known limitations:** the exit token polls the child every 10 ms, as our own host does, instead of waiting on the process handle.
