---
category: Added
changelog: In the `rusty-mux` workspace tree, `y` copies the selected path to the clipboard.
---
## 2026-11-26 - TUI workspace tree: copy path

- **Added:** `y` in the workspace tree (`e`) sends the selected row's path, relative to the workspace, to the terminal's clipboard with OSC 52 (`rusty_mux_client::clipboard::osc52_set`). Card #1573, D374.
- **Known limitations:** the terminal must accept OSC 52 (the status line says the path was sent, not that it landed); paths over 4096 bytes are not copied; `cd` and open in an editor are not built.
