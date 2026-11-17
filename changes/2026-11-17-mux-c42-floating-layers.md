---
category: Added
changelog: rusty_mux windows can hold floating blocks over their layout (C42, slice 1: model, wire, server and ctl).
---
## 2026-11-17 - rusty_mux floating layers (C42, slice 1)

- **Added:** a window can float blocks over its tiled layout. `place-float` floats a block on top, or moves and resizes one that floats (a rect in whole percent of the window, at least 10% wide and tall); `stack-float` raises or lowers it. `Features::FLOATS` (bit 50), `FloatsChanged` events and a floats row in `list` go only to clients that ask for the bit. `rusty-mux-ctl` lists and prints them.
- **Known limitations:** the GUI and TUI do not draw floats yet (a floated block shows as detached to them; the next slice adds drawing and drag). Floats are not saved: after a server restart a float comes back as a detached block, because storing them needs a schema bump (owner's call).
