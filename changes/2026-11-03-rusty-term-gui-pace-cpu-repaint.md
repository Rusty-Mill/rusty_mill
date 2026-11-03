---
category: Changed
changelog: rusty_term --gui CPU renderer repaints output at most once per display refresh, so floods parse faster.
---
## 2026-11-03 - rusty_term GUI paces CPU repaint to the display refresh

- **Changed:** With the CPU renderer, a burst of output now repaints at most once per display refresh. Previously it repainted on every wakeup, about 1150 fps under a flood. Each of those frames held the grid lock the parser needs, and on Windows that cost 6–8% of throughput. Input, resize and timer repaints are not delayed. The GPU renderer already waits for vsync, so it is unchanged.
- **Known limitations:** The refresh rate is read when the renderer is built, which happens at window creation and on a DPI or font change. Moving the window to a monitor with a different refresh rate but the same scale keeps the old rate. If the monitor reports no rate, 60 Hz is used.
