---
category: Added
changelog: rusty_mux_server, the multiplexer's Phase 1 server on rusty_tokio (PTYs, snapshot-then-raw-output attach, single-writer input), plus LinuxPty::spawn_concrete and LinuxPtyMaster::set_nonblocking.
---
## 2026-10-23 - rusty_mux_server

- **Added:** `crates/apps/rusty_mux/crates/server` (`rusty_mux_server`, binary `rusty-mux-server <socket>`). One thread on `rusty_tokio`'s `LocalRuntime`; each block is a PTY running `$SHELL`, read through the reactor and fed to a headless `rusty_term_core` grid. Attaching sends a snapshot of that grid, a ready frame, then the raw PTY bytes; input reaches a PTY only from the lease holder, and the terminal's replies to queries go back to the PTY. Clients over an 8 MiB output backlog are dropped rather than stall the PTY. The socket is made owner-only (0600), and a peer whose uid (SO_PEERCRED) is not the server's effective uid is closed before any reply. No tokio in the binary.
- **Added:** `platform_linux::LinuxPty::spawn_concrete` (the trait's `spawn` without type erasure, so the master is `Send + Sync` and pollable) and `LinuxPtyMaster::set_nonblocking`. Both additive; `Pty::spawn` delegates.
- **Fixed (before release):** a flooding PTY starved input; the PTY pump now yields between chunks. Under a background flood a command round-trips in about 5-70 ms in a debug build (`input_round_trips_while_the_pty_floods`, bound 1 s).
- **Known limitations:** Linux only (ConPTY after spike 1). No authentication beyond the same-uid check, no TLS or daemonizing. Closing a session or window is not on the wire yet, only blocks. History is sent whole in the snapshot, not streamed newest-first.
