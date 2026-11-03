---
category: Fixed
changelog: rusty_tokio's Mutex no longer stays locked, or leaves the next waiter unwoken, when a waker's destructor panics.
---
## 2026-11-01 - rusty_tokio: Mutex survives a panicking waker destructor

- **Fixed:** two `Mutex` paths could break when a waker's destructor panicked.
  - When a waiter acquired the lock, its leftover wakers were dropped after it set `locked` but before its guard existed. A panic there left the mutex locked with no guard to release it, so every later `lock()` hung.
  - When the front waiter was cancelled, its waker was dropped before it woke the next waiter. A panic there stranded that waiter.
- `Mutex` now does what `RwLock` already does (#1344). An acquiring waiter drops its leftovers only after building its guard, so the panic unwinds through the guard and unlocks. A cancelled waiter wakes the next one first.
- Two new tests in `tests/lock_cancel.rs` fail on the base and pass now. One of them runs over both `lock` and `lock_owned` (#581).
