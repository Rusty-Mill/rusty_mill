---
category: Added
changelog: rusty-mux-gui draws and edits floating blocks (C42): drag the header to move, an edge or corner to resize, click to raise, Ctrl-] f n u U m H J K L as in the TUI.
---
## 2026-11-17 - rusty-mux-gui draws floating layers (C42)

- **Added:** the GUI asks the server for `FLOATS` and draws each float over the tiled layout as a block frame inside a keyline, with a "Float" tag in its header, the focused one in the accent and an unfocused one veiled (gui-design D89, D92). Drag a header to move a float, an edge or corner (6 px band) to resize it, click it to focus and raise it; the new place goes to the server once, on mouse-up, and one it refuses snaps back (D91). The TUI's prefix keys work: `Ctrl-]` then `f` float, `n` next block, `u` / `U` raise and lower, `m` then `h j k l` move, `H J K L` resize. A tile zoom hides the floats. The window's source port grows `FloatAt`, `Float`, `Act::Float`, `Act::Place`, `Act::Stack`, `Source::floating` and `Remote::floats`, and `Remote::layout` takes the floats (handoff D334).
- **Changed:** both renderers draw a frame with floats in layers over the tiles (the CPU one paints it whole, the GPU one adds a pass per float); a frame with none is drawn as before.
- **Known limitations:** a block that floats from a tile reopens its pane. Floats are not in the strip's thumbnails. The keys were not typed live; the mouse was run through posted window messages on one Windows machine (gui-design D93). Floats are still not saved across a server restart (handoff D323).
