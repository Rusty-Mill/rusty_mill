---
category: Fixed
changelog: Stop rusty_term's PTY readers from starving the render thread of the grid lock; bench_frame_time reports reader hold and gains --pipe.
---
## 2026-10-24 - Render-thread grid-lock starvation

- **Fixed:** the PTY reader re-locked the grid for each chunk faster than a woken waiter could run, so on the unfair `std` mutex the render thread (on the GUI's event-loop thread, freezing input too) waited up to hundreds of milliseconds for a 0.1 ms snapshot. `sync::Mutex` now counts blocked `lock()` callers, and the GUI reader loop and TUI parser task take the new `lock_bulk()`, which yields while anyone is waiting (at most 1 ms). On alt_screen_flicker, `bench_frame_time` render wait p99 went from 420 ms (12 frames in 2 s) to under 1 ms (182 of 180), with no parser throughput loss. Std-only; no dependency added.
- **Changed:** `bench_frame_time` reports the reader's hold per chunk and the render thread's wait max, renames the old hold column to "snapshot hold", and `--pipe` feeds chunks from reads of an OS pipe, like a PTY.
- **Known limitations:** measured on Linux only, with a pipe standing in for a real PTY. Windows SRW locks are also unfair and should behave the same, but that is unverified. The TUI's single-task reader path shares a loop with rendering and is unchanged.
