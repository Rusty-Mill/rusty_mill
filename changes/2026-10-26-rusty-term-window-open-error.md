---
category: Fixed
changelog: rusty_term's window mode now exits with an error instead of panicking when the windowing library cannot start, such as when libxkbcommon-x11 is missing.
---
## 2026-10-26 - Window mode reports a missing windowing library

On X11 without `libxkbcommon-x11`, winit panics while building its event loop, so `rusty_term --gui` crashed with a backtrace. `build_event_loop` now catches that panic, with the panic hook silenced, and returns `cannot open a window: <reason>`. The process exits 1. Checked by hiding the library under `xvfb-run`; with it present the window opens as before.
