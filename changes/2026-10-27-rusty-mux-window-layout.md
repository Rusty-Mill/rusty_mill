---
category: Added
changelog: rusty_mux window layouts kept by the server: split a block, zoom one, and follow layout changes as events.
---
## 2026-10-27 - rusty_mux server-side window layout (C12)

- **Added:** each window keeps a layout, a split tree of its blocks (`rusty_mux_core::Layout`), and an optional zoomed block. Its text form is `cols50(A,rows50(B,C))`: `cols` side by side, `rows` stacked, the first child's share in percent.
- **Added:** `split-block --block B --side left|right|up|down` starts a program (the shell, or `--command`, `--arg`, `--cwd`) in a new block beside `B`, splitting its place and size in half; `zoom-block` and `unzoom-window`. Closing a block gives its place to its sibling.
- **Added:** `layout-changed` events (kind 25) with the window's `layout` and `zoomed` block, for `events`, `watch` rules and `wait --event`; `list` shows each window's `layout` and `zoomed`.
- **Changed:** `create-block` and `run` place the new block beside the window's largest block. The protocol gains `Features::LAYOUT` (bit 15) for the new requests and events; window entries carry the layout to every client, and older clients skip it.
- **Not yet:** the attach client does not draw layouts, and splits cannot be resized.
