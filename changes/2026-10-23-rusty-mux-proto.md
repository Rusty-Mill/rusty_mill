---
category: Added
changelog: rusty_mux_proto, the multiplexer's wire protocol (length-prefixed frames of tagged LEB128 fields, feature bits).
---
## 2026-10-23 - rusty_mux_proto

- **Added:** `crates/apps/rusty_mux/crates/proto` (`rusty_mux_proto`), the Phase 1 wire protocol of rusty_mux on `rusty_wire`. A frame is a `u32` big-endian length, a `u8` message type and tagged LEB128 fields; unknown field ids are skipped, and new message types are gated by feature bits exchanged in `Hello`/`Welcome`. It carries the client requests (create, attach, lease, input, resize, close), the server's replies, the core model's events, and the snapshot, ready and raw output frames. Frames over 16 MiB are refused from the length prefix alone. No I/O.
- **Known limitations:** no compression, resumable event stream or input op-id dedupe yet (Phase 2). The server and TUI that speak it follow in later PRs.
