---
category: Added
changelog: The mux TUI gets a pure workspace tree model for the explorer (not wired to a key yet).
---
## 2026-11-24 - TUI workspace tree model (card #1573)

- **Added:** `rusty_mux_tui::explorer` parses `terminal.workspace-list` replies into a lazily opened tree with a cursor and renders it as text rows (directories open, links and other files never followed, cut listings say so). No key, overlay or server call yet; those are the next slice. No wire change.
