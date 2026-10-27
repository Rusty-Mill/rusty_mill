---
category: Added
changelog: rusty_mux process info: `terminal.process` lists what runs in a block (the foreground process, its ancestors and the block's program), and the server announces `process-changed` when another process group takes a block's terminal.
---
## 2026-10-27 - rusty_mux process info (C31)

- **Added:** the `terminal.process` block method (`rusty-mux-ctl terminal.process --block …`, and an MCP tool from the same catalog). It answers one line per process, `PID<TAB>NAME<TAB>COMMAND`: the terminal's foreground process first, then its ancestors, the block's own program last. An idle shell is one line. The name and command line are escaped (`\\`, `\t`, `\n`, `\r`, other control characters as `\u{HEX}`), so an argument holding a tab, newline or escape sequence cannot add a field or a row, or reach a terminal raw. Read from `/proc`, with the foreground group from `tcgetpgrp` on the PTY master (`rusty_libc`); no new dependency.
- **Added:** `Event::ProcessChanged` (kind 20) with the new foreground group's leader `pid` and program `name`, behind the new `Features::PROCESS` (bit 12); a client without it never receives one, live or on replay. The server checks each block's foreground group after every output read and every 500 ms, so a job that starts without writing anything is still announced. `rusty-mux-ctl events`, `watch` rules and `wait --event process-changed --name sleep` see it as `process-changed block=… pid=… name=…`.
- **Known limitations:** Windows has no foreground process group under ConPTY: `terminal.process` lists only the program's pid, with an empty name and command line, and no event is sent. When a pipeline's group leader has exited, only the block's program is listed.
