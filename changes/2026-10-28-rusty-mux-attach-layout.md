---
category: Added
changelog: The rusty-mux terminal client draws a window's split layout, with Ctrl-] commands to move between, split, zoom and close blocks.
---
## 2026-10-28 - rusty_mux attach client draws window layouts (C12)

- **Added:** `rusty-mux <socket> [block]` shows the block's whole window. A window of several blocks is drawn side by side and stacked, with dividers, and follows `layout-changed` events as blocks are split, zoomed and closed by anyone.
- **Added:** Ctrl-] commands: `d` detach, arrows or `h` `j` `k` `l` move the focus, `z` zoom or unzoom, `|` and `-` split right or down, `x` close, Ctrl-] Ctrl-] types a Ctrl-].
- **Changed:** Ctrl-] alone no longer detaches; press Ctrl-] then `d`. The client holds only the focused block's input lease, so agents and other clients can type into the others.
- **Changed:** the SGR style writer behind `terminal.format --vt` moves to `rusty_mux_server::style`, shared with the client.
- **Not yet:** resizing splits; blocks out of focus keep their size and are clipped or padded.
