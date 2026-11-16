---
category: Added
changelog: rusty-mux-gui gets focus follows mouse, dimming of inactive panes and pinch to zoom (C62).
---
## 2026-11-16 - GUI pane focus and gestures (C62)

- **Added:** `[window] focus_follows_mouse` focuses the pane under the pointer, as a click does, and `[window] dim_inactive_panes` shows unfocused panes at 70%. Both default off and have Settings rows. A pinch on macOS zooms the focused block out and in.
- **Known limitations:** pinch needs macOS (winit sends no pinch on Windows or Linux). No swipe and no animated focus highlight.
