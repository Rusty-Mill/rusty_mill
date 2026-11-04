---
category: Added
changelog: rusty_mux keeps each block's history past its 10,000-line scrollback on disk, up to 64 MiB, and reads it back with `terminal.history` (C70).
---
## 2026-11-04 - rusty_mux scrollback spooled to disk (C70)

- **Added:** while `rusty-mux-server` keeps state (C45), the lines a block's scrollback evicts are written to `<block>.spool/` in the store, up to 64 MiB per block, oldest dropped first. The spool survives a restart and is deleted when its block closes.
- **Added:** `rusty-mux-ctl terminal.history --block B --history N [--skip K]` reads them as text, oldest first, with `--trim`, `--unwrap` and `--vt` as in `terminal.format`. MCP exposes it as a read-only tool.
- **Protocol:** a block call behind `Features::TERMINAL_HISTORY` (bit 34). No new message type, kind or field.
- **Added:** `rusty_term_core::Grid::spill_evicted`, `Grid::take_spilled`, `Grid::spill_failed` and `checkpoint::spilled_grid`, which is a public API change.
- **Known limitations:** a spilled line keeps colors and wrapping but drops hyperlinks and characters combined from several. Lines a resize rewraps past the cap are not spilled. After a crash the spool's newest lines can repeat the oldest lines of the restored scrollback. History text is stored unencrypted until encryption at rest lands. A read runs on the server's thread.
