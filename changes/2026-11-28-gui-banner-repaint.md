---
category: Fixed
changelog: The CPU renderer now repaints a banner that changed or cleared, so an upload's "Sending file" banner no longer stays on screen until the mouse moves.
---
## 2026-11-28 - GUI banner clears when it should

- **Fixed:** the CPU renderer's damage tracking compared the strip, body and popover between frames but not the `over` and `top` pixmaps. The banner is an opaque `top` pixmap, so when only it changed, only the changed pane rows were repainted and the old banner stayed on screen (seen as "Sending file: 77%" after a 3 MiB upload on Windows) until a full repaint such as a mouse move. Both layers and their places are now part of the frame's layout, so a change repaints whole. Card #1640.
- **Known limitations:** floats are still not compared; a float change on its own frame is unchanged by this fix.
