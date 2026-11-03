---
category: Fixed
changelog: rusty_term's window no longer shows a white or stale area after it is dragged partly off screen and back, or resized, on the CPU renderer.
---
## 2026-11-02 - rusty_term: repaint what the window lost

- **Fixed:** the CPU renderer presented only the rows it repainted. When the OS asked for a repaint because the window had lost pixels (dragged partly off screen and back, uncovered, resized), no row had changed, so nothing reached the window and the area stayed white or stale (#1387). It now presents the whole buffer every frame; it still composites only the changed rows, which is where the 2026-10-24 damage tracking saved its time.
- **Known limitations:** each frame now copies the whole window to the screen instead of the changed rows. Reproduced and checked under Xvfb on Linux; the Windows check is manual. Why the drag looked fine on ConPTY on Windows is not known; the fault is in the renderer, not the console host.
