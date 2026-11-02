---
category: Added
changelog: rusty_mux layout editing: `resize-split`, `swap-blocks`, `move-block`, `detach-block` and `move-window`, with the `window-moved` event.
---
## 2026-11-02 - rusty_mux layout editing (C41, part 1)

- **Added:** five requests edit a window's layout and the order of windows, behind the new `Features::LAYOUT_EDIT` (bit 27), each in ctl and as an MCP tool of the same name: `resize-split` (type 0x19) gives a block a percent share of the innermost split along `cols` or `rows` that holds it; `swap-blocks` (0x1a) trades two blocks' places; `move-block` (0x1b) moves a block into a new split beside another, or into a window that shows no block; `detach-block` (0x1c) takes a block out of the layout while its program runs on; `move-window` (0x1d) moves a window to a place among its session's windows.
- **Added:** `Event::WindowMoved` (kind 31) names the window and its new place; a client without `LAYOUT_EDIT` never receives it. `rusty-mux-ctl events`, `watch`, `wait --event` and `on` rules know it as `window-moved window=… index=…`. Layout edits are announced by `layout-changed`, as splits are.
- **Changed:** a window's layout may now leave out some of its blocks: those are detached. `list` still lists them in the window, and `zoom-block` refuses one.
- **Known limitations:** blocks move only within their window. The attach client has no keys for the edits yet, and it ends, as when its last block closes, if every block of its window is detached. Server-side focus is the next part of C41.
