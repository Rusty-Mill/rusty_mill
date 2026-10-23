---
category: Added
changelog: rusty-mux, a throwaway Phase 1 terminal client that attaches the local terminal to one rusty_mux block.
---
## 2026-10-23 - rusty_mux throwaway TUI

- **Added:** `crates/apps/rusty_mux/crates/tui` (`rusty_mux_tui`, binary `rusty-mux <socket> [block]`). Without a block id it creates a session, window and block; it attaches, takes the input lease, sizes the block to the local terminal, paints the snapshot once and then passes raw output to the local terminal. Ctrl-] detaches and prints the block id for reattaching. Two blocking threads, no async runtime.
- **Known limitations:** throwaway by design (Phase 2 brings real clients). Linux only, no local resize tracking, only colours are painted from the snapshot (no bold or underline), and a client without the lease is view-only.
