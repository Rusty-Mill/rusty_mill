---
category: Added
changelog: rusty_mux announces ports-changed when the TCP ports a block's programs listen on change, so clients need not poll terminal.ports.
---
## 2026-11-19 - rusty_mux: ports-changed (C80, slice 2)

- **Added:** `Event::PortsChanged { block }` (kind 37), sent to clients that negotiated `Features::PORTS` (bit 39) when what `terminal.ports` would reply for the block differs from the last announcement. It carries no ports: the client reads `terminal.ports`. The server reads the ports of every block every 2 s, and only while some client has the bit, since a read walks `/proc`. `rusty-mux-ctl events`, `watch` rules and `wait --event ports-changed` see it as `ports-changed block=…`.
- **Known limitations:** Linux only, as `terminal.ports`; a listener that starts and stops between two reads is not seen.
