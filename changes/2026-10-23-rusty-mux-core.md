---
category: Added
changelog: rusty_mux_core, the multiplexer's pure domain model (sessions, windows, blocks, clients, single-writer input lease, events).
---
## 2026-10-23 - rusty_mux_core

- **Added:** `crates/apps/rusty_mux/crates/core` (`rusty_mux_core`), the first Phase 1 crate of rusty_mux. It holds sessions, windows, blocks (one PTY each) and clients (one connection each), a single-writer input lease per block, and an ordered event queue the server drains. It does no I/O and has no async runtime; its only dependency is `rusty_uuid`.
- **Known limitations:** nothing uses it yet. The wire protocol, server and TUI follow in later PRs. Lease hand-over is cooperative: there is no forced takeover.
