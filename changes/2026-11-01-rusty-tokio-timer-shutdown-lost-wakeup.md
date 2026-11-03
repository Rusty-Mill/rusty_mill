---
category: Fixed
changelog: rusty_tokio's timer driver no longer misses its shutdown signal and blocks runtime shutdown until the next timer deadline.
---
## 2026-11-01 - rusty_tokio: timer shutdown can no longer be lost

- **Fixed:** the timer thread checked its `shutdown` flag before taking its state lock, and `TimerDriver::shutdown` set the flag and notified without that lock. A shutdown that landed between the check and the thread's condvar wait was lost. The thread then slept until its next deadline, or 200 ms when nothing was scheduled, and runtime shutdown blocked in `join` for that long (an hour-long `sleep` held it for an hour). The flag is now set while holding the lock and checked while holding it. A unit test in `src/time/mod.rs` holds the lock to stop the thread inside that window. It fails on the base and passes now (#582).
