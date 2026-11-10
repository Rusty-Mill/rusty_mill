---
category: Changed
changelog: rusty_mux's MCP stdio suite and ctl's pwd, title, watch-rule and interrupt tests now also run on Windows.
---
## 2026-11-10 - rusty_mux tests ported to Windows (part 1)

- **Changed:** `rusty-mux-mcp`'s stdio suite runs on Windows (its two full-accept-backlog cases stay Linux-only, the one platform where ctl bounds connecting). `rusty-mux-ctl`'s `terminal.pwd`/`terminal.title`, watch-rule and interrupted-follow tests gain Windows coverage: `cmd.exe` prints OSC 7 with an ESC taken from `prompt $E`, and Ctrl+Break ends a follow with `STATUS_CONTROL_C_EXIT` (card #282, item 1).
- **Known limitations:** the attach-client lifecycle suite (`tui/tests/lifecycle.rs`) and the server's event suite are still Linux-only; they follow in their own PR.
