---
category: Added
changelog: rusty_mux: rusty-mux and rusty-mux-gui start a server on the default socket when none answers, and rusty-mux-server --idle-exit SECS stops a server with no blocks and no clients (C69 slice b).
---
## 2026-11-06 - rusty_mux auto-start and idle exit

- **Added:** `rusty-mux-server --idle-exit SECS` stops the server cleanly once it has had no block and no client for SECS; it never stops while a block lives.
- **Added:** run with no socket and no `$RUSTY_MUX_SOCKET`, `rusty-mux` and `rusty-mux-gui` start `rusty-mux-server --idle-exit 60` on the default socket when nothing answers there, detached from the terminal, and connect to it. `rusty-mux-ctl`, `rusty-mux-mcp` and `--ssh` never start a server.
- **Known limitations:** a started server's errors are not shown; run `rusty-mux-server` by hand to see why one fails to start.
