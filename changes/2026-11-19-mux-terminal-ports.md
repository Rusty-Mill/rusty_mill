---
category: Added
changelog: rusty_mux_server answers terminal.ports on Linux, so ctl, MCP and agents can read which TCP ports a block's programs listen on.
---
## 2026-11-19 - rusty_mux: terminal.ports (C80, slice 1)

- **Added:** block method `terminal.ports`: one `PORT<TAB>PID<TAB>NAME` line per TCP socket a process of the block's session listens on, by port then pid. Behind `Features::PORTS` (bit 39), offered on Linux only; it travels as `BlockCall`, so no new message type. It appears in `rusty-mux-ctl` and MCP through the catalog.
- **Known limitations:** Linux only (read from `/proc`), TCP only, and only sockets in the server's network namespace. The `PortsChanged` event and the click-to-forward of C81 are later slices.
