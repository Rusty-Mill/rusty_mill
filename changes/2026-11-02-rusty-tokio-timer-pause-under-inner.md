---
category: Fixed
changelog: rusty_tokio time::resume() no longer leaves due timers up to 200 ms late, and a double pause()/resume() no longer breaks the runtime's timers.
---
## 2026-11-02 - rusty_tokio: pause and resume publish under the timer lock

- **Fixed:** `time::pause()` and `time::resume()` changed the clock without the timer driver's `inner` lock. The timer thread reads the pause state under that lock and then waits, so a resume landing in between went unseen until the 200 ms poll and due timers fired late. Both now change the clock under `inner` and notify after releasing it. The misuse panics (pause twice, resume while running) now fire after the locks are released, so they no longer poison the clock and break every later timer call (#587).
