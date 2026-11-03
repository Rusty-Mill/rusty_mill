---
category: Fixed
changelog: rusty_term no longer paints half-drawn frames during a synchronized update (DEC 2026), and repaints when one times out even if no more output arrives.
---
## 2026-10-26 - rusty_term: synchronized output holds every paint and wakes at its timeout

- **Fixed:** an update left open past its 800 ms timeout was only noticed on the next event, so a client that set `?2026h` and went quiet left the screen stale until a key press or more output. Both front-ends now wake at the timeout and repaint. In terminal mode, output written after `?2026h` with nothing following now shows at 0.8 s; before, it waited for the next event (2.8 s in the check, when the child wrote again).
- **Fixed:** the terminal-mode renderer painted mid-update when input or a scroll woke it. `render_once` now holds every paint while an update is open, and its reader always signals so the hold can set its timer. A resize is the one exception, the same as in the window: the screen clear moved from the resize handler into the next frame (`resize_grid`), and that frame always paints the reflowed grid. A clear can no longer land while a paint is held, which would leave the screen blank until the update ended.
- **Fixed:** the window painted mid-update on a cursor blink, an animation tick, input or a resize. While an update is open, each pane repaints the frame it last painted, unless its size or scroll position changed. Every pane's timeout is tracked. When one pane's update ends while another pane still holds its own, the window repaints right away rather than at the later timeout.
- **Added:** `Grid::sync_output_until`, the open update's timeout. An update now closes at exactly 800 ms instead of just after.
- **Tested:** core tests for the timeout and for expiry. Terminal-mode `render_once` holds once with one wakeup and paints after the close. A resize during an update paints the reflowed grid and later frames hold again. Staggered pane timeouts repaint when the first one ends. Window `frame_to_paint` repaints the last frame while open, shows the grid after the close, and shows it after a resize.
