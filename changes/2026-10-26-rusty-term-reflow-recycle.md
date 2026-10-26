---
category: Changed
changelog: Resizing a rusty_term window with a full scrollback reflows about twice as fast.
---
## 2026-10-26 - Faster scrollback reflow on resize

`reflow_history` now takes the scrollback and screen by value. It packs the logical lines into one flat cell buffer, records the output rows as ranges into it, and builds only the rows that survive the scrollback cap, reusing the old lines' cell vectors. The padding check is memoized per cell. With 10,000 lines of colored scrollback at 200x50 (release build, the ignored `measure_reflow` test), the first resize to 150x50 drops from about 30 ms to about 12.5 ms, and the later resizes drop from 10/8.6/3.8 ms to 7.4/7/2.3 ms. Reflow output is unchanged; all core reflow tests pass.
