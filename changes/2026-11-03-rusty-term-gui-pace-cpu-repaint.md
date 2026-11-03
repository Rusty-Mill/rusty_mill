---
category: Changed
changelog: rusty_term --gui CPU renderer repaints output at most once per display refresh, so floods parse faster.
---
## 2026-11-03 - rusty_term GUI paces CPU repaint to the display refresh

- **Changed:** With the CPU renderer, a burst of output now repaints at most once per display refresh. Previously it repainted on every wakeup, 1100–1700 fps under a flood, and each of those frames held the grid lock the parser needs. On one Windows machine (240 Hz), 4 MB `type` floods ran 1.9–6.1% faster with pacing. Repaints the window requests itself (resize, timers, and input that moves the view) are not delayed. The GPU renderer already waits for vsync, so it is unchanged.
- **Known limitations:** After input to the child, the next visible output within 100 ms repaints at once. This is usually the echo, but output from another pane or an earlier queued wakeup can use the grace up first. The echo in that case, and any response after 100 ms, is paced and can wait up to one frame interval (4.2 ms at 240 Hz, 16.7 ms at the 60 Hz fallback) plus scheduling. The refresh rate is read when the renderer is built, which happens at window creation and on a DPI or font change. Moving the window to a monitor with a different refresh rate but the same scale keeps the old rate. If the monitor reports no rate, 60 Hz is used.
