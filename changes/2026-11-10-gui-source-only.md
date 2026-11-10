---
category: Removed
changelog: The GUI window code drops the last of local mode: the local tab bar, tab dragging, the local status row and local pane resize and close.
---
## 2026-11-10 - GUI windows take their layout only from the source (R3)

- **Removed:** the window's source, strip, body, popover and block place are no longer optional, so every local-mode branch in `gui/window.rs` is gone: the local chrome tab bar (tabs, `+`, `▾` shell menu, overflow, settings tab, drag-to-reorder, middle-click close), the bottom status row, and local pane resize, close and zoom. The resize keys do nothing now, as they already did in a mux window. Split, close and zoom still go to the server (handoff D159, D165). About 1,500 lines out; nothing a mux window showed changes.
- **Removed:** the pane-exit event. A pane's feed ending no longer closes anything in the window; the server's next layout drops the block.
- **Changed:** `rusty_term ctl` prints its retirement notice and exits 1 without an `Error: Custom {..}` dump after it. `backend::read_batch` is test-only now (public API removal; only the batching tests used it).
- **Docs:** the README's window section describes the mux-client window; handoff D165 records Nano's go and D166 corrects D159's count (19 branches, not 21).
