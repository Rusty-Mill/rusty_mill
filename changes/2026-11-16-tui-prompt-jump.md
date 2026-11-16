---
category: Added
changelog: The rusty-mux terminal client scrolls the focused block to the previous or next shell prompt with Ctrl-] [ and Ctrl-] ].
---
## 2026-11-16 - rusty-mux client: jump between prompts (C55, part 1)

- **Added:** after the prefix, `[` scrolls the focused block to the previous prompt its shell marked (OSC 133 or 633 `A`) and `]` to the next; past the last one, `]` returns to the live screen. The block's history is drawn from the client's own copy, so a single block leaves pass-through while you browse and returns to it on the next key. The next key typed (or any other command) also returns to the live screen. No wire change.
- **Changed:** `Grid::viewport_cell` also exists in a headless build of `rusty_term_core` (no `gui` feature, no folds), so the client draws scrolled history without enabling `gui`.
- **Known limitations:** find in scrollback and a server-side read of prompt marks are not in this change. Jump reaches only the history the client holds.
