---
category: Added
changelog: New `rusty_mux_client` crate with rusty_mux's attach protocol, shared by the terminal client and the coming GUI client; `rusty_mux_tui` no longer links the server.
---
## 2026-11-05 - rusty_mux: `rusty_mux_client`, the shared attach protocol

- **Added:** `crates/apps/rusty_mux/crates/client` (`rusty_mux_client`), arc 2 slice S1 (`docs/projects/term/ARC2-MUX-CLIENT.md`, D96). `conn::Conn` (frame I/O) and `attach` (connect, Hello and Welcome, the pre-loop requests, takeover on a fresh path, reading a window's layout from a List) moved out of `rusty_mux_tui` unchanged.
- **Changed:** the RTCP v6 checkpoint assembler and the cell style codec move from `rusty_mux_server` to `rusty_mux_proto`, behind a new `screen` feature that adds `rusty_term_core` (without `l13`). The server and the clients turn it on; ctl, mcp and script don't, so they never build the terminal engine. `rusty_mux_server::{checkpoint, style}` re-export them. `rusty_mux_tui` depends on the server only for its tests.
- **Known limitations:** no behaviour or wire change. `upgrade` (SSH to UDP) and the render and input loop stay in the TUI until the GUI needs them.
