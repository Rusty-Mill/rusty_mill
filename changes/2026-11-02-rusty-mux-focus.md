---
category: Added
changelog: rusty_mux server-side focus: each window's focused block, set by `focus-block` and `move-focus` and announced by `focus-changed`.
---
## 2026-11-02 - rusty_mux server-side focus (C41, part 2)

- **Added:** the server keeps each window's focused block: the first block placed in it, until the new `focus-block` (type 0x1e) or `move-focus` (0x1f, toward `left`, `right`, `up`, `down`, `next` or `previous`) moves it, both in ctl and as MCP tools. When the focused block leaves the layout, the block that takes its place gets the focus. Zooming a block focuses it, and focusing another ends the zoom. The window entry of `list` shows it as `focused`.
- **Added:** `Event::FocusChanged` (kind 32) names the window and its focused block, none once no block is placed. The requests and the event sit behind the new `Features::FOCUS` (bit 28); a client without it never receives the event. `rusty-mux-ctl events`, `watch`, `wait --event` and `on` rules know it as `focus-changed window=… block=…`.
- **Known limitations:** the attach client still keeps its own focus; following the server's is part 3 of C41.
