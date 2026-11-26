---
category: Added
changelog: A forwarded port that nothing answers shows a notice in the `rusty-mux` GUI, and a forwarded port you allow stays trusted across runs.
---
## 2026-11-26 - Port forwards: a notice for a refused dial, saved trust

- **Added:** when the server ends a forward with a reason (a dial nothing answers, say), the GUI shows "Port forward closed" with the port and the reason; `forward::run_fed` reports it through a new `refused` callback. Allowing a forwarded port now saves the choice, one file per host and port in `trusted_forwards/` beside `trusted_origins/`, so the prompt asks once per host and port, not once per pane and window (C81 slice 4, card #1584, D341).
- **Known limitations:** the host is the server's socket path, so the same server reached by another socket is asked again; no Settings row to revoke yet (delete the file); a failed save leaves the port trusted until the window closes. Untested on a real display.
