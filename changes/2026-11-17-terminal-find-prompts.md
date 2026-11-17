---
category: Added
changelog: rusty_mux_server answers terminal.prompts and terminal.find, so ctl, MCP and agents can read where a shell's prompts started and search a block's scrollback and screen.
---
## 2026-11-17 - rusty_mux: terminal.prompts and terminal.find (C55, part 3)

- **Added:** block methods `terminal.prompts` (one `ROW<TAB>TEXT` line per prompt the shell marked with OSC 133/633 `A`) and `terminal.find` (`query`, optional `limit`; one `ROW<TAB>COL<TAB>TEXT` line per case-insensitive match, across wrapped rows). `ROW` counts from the top of the screen, negative in the scrollback. Behind `Features::TERMINAL_FIND` (bit 49); they travel as `BlockCall`, so no new message type. They appear in `rusty-mux-ctl` and MCP through the catalog.
- **Added:** `rusty_term_core` `Grid::find_text`, `Grid::prompt_marks` and `TextHit`, headless (no `gui` feature). The terminal client's find now uses the same search.
- **Known limitations:** history spooled to disk (`terminal.history`) is not searched or marked; literal text only.
