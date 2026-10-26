---
category: Fixed
changelog: rusty_tokio io_uring reactor no longer hangs deregistering a socket whose poll is racing a spurious wakeup.
---
## 2026-10-25 - rusty_tokio io_uring deregistration no longer hangs

- **Fixed:** with `io-uring-reactor`, dropping a socket could hang the runtime about 1% of the time
  (seen in `tests/async_io.rs`). Deregistration waited for a terminal poll CQE after `PollRemove`
  returned `EALREADY`. That result means a wakeup currently owns the poll; if the event was already
  consumed (a listener whose connection was accepted), the poll re-arms and posts nothing, so the
  wait never ended while it held the driver lock. Cancellation now retries the remove.
- **Tests:** a reactor unit test makes the race deterministic with a `DEFER_TASKRUN` ring. It hangs
  without the fix. `async_io` with all features: 3/300 runs hang on main, 0/1000 on the fix.
