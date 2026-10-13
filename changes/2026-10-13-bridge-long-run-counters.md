---
category: Fixed
changelog: `rb_rl_game_bridge` keeps counting time correctly in a very long run: the frame and message counters no longer overflow, seconds are divided in f64, `TickTracker::observe` takes the frame counter, and a map's goal list is capped.
---
## 2026-10-13 - rb_rl_game_bridge: long-run counters

- **Fixed:** `GameFolder` added each bundle's frames to a `u32` with `+=`, which panics on overflow in a debug build after about 414 days at 120 Hz; it now wraps, as the wire's `u32` frame counter does.
- **Fixed:** `seconds_elapsed` was `frame_num as f32 / 120`. Past 2^24 frames (about 39 hours) the cast alone loses single frames. It is now divided in `f64` and rounded once, to the wire's `f32`.
- **Changed (API):** `TickTracker::observe(seconds_elapsed, frame_num)` takes the frame counter too. After about 39 hours even a correctly rounded `f32` of seconds cannot show one 1/120 s frame (the step is below its resolution), so the old rule missed some advancing frames (the seconds only change every other frame or so) and distribution to the bots degraded under the RLBotCS rule, though it did not stop for good. A forward step of the (wrapping) frame counter now also counts as time advancing. The only caller in the repo is `BridgeLoop::on_clump`; the seconds rule is unchanged.
- **Fixed:** `TickTracker` counted messages (`ticks_skipped`, `ticks_since_map_load`) with unchecked `u32` arithmetic, which overflows after about 207 days at 240 messages a second without a map change; both now saturate.
- **Fixed:** a map's goal list is capped at 16 (a map has two), so a game that never stops announcing goals cannot grow it without bound; a new map clears it as before.
- **Verified:** tests, each failing when its fix is removed: the wrap, the f64 seconds, the frame rule at 40 hours and across the wrap, the goal cap, the saturating counters, and a composed one that feeds frames 17,280,001 and 17,280,002 (identical `f32` seconds) from `GameFolder` through `BridgeLoop` and sees both distributed.
- **Known limitations:** none of these was seen in a live run; they need a game left running for more than a day and a half. The first item of board b24, respawning a bot car that was destroyed mid-map, is not done: a demolition keeps its actor (#634), and no mid-map destroy of a player actor was observed.
