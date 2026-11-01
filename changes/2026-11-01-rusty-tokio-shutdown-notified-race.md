---
category: Fixed
changelog: rusty_tokio's Handle::shutdown_notified no longer misses a shutdown that begins while it is first being polled.
---
## 2026-11-01 - rusty_tokio: shutdown_notified creates its Notified before checking the flag

- **Fixed:** `Handle::shutdown_notified` read the `shutting_down` flag before it created its `Notified`. A `begin_graceful_shutdown` that ran between those two steps set the flag after it was read, and broadcast before the `Notified` existed. The task then slept through the shutdown until something else woke it. It now creates the `Notified` first, as tokio prescribes, so that `notify_waiters` reaches it (possible since #576). The logic moved into a private `flag_or_notified` helper so that a unit test can force the interleaving. That test fails with the old order and passes now. Two more tests guard the flag-already-set path and the later-broadcast path (#583).
