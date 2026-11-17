---
category: Added
changelog: rusty-mux-gui has a command palette (Ctrl+Shift+P) with directory browse that types a folder or file path into the focused block (C08).
---
## 2026-11-17 - GUI command palette and directory browse (C08)

- **Added:** `rusty-mux-gui` opens a command palette with Ctrl+Shift+P (`command_palette` in `[keys]`) or the session chip menu's first item. It fuzzy-finds actions (split, zoom, close, move focus, new window, switcher, settings, reload config, take over and give back), sessions and windows, and runs the pick. "Go to directory" and "Insert path" list the focused block's host through `terminal.list-dir`, Tab goes into a folder, Backspace up, and Enter types `cd PATH` or the path into the block, quoted for the host's path style, without pressing Enter.
- **Added:** `Source::lists` and `Source::list_dir` on the window's source port; the mux source asks over the block's own connection, which holds the input lease the server requires.
- **Known limitations:** a typed path in the search box does not start a browse yet; no recents; no remote-host listing; the palette is not shown for a server without `terminal.list-dir`, where the two folder rows are hidden.
