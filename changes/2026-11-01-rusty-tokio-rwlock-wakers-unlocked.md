---
category: Fixed
changelog: rusty_tokio's RwLock no longer clones or drops a waker while holding its internal state lock, and a panicking waker no longer poisons it.
---
## 2026-11-01 - rusty_tokio: RwLock wakes, clones and drops wakers outside its state lock

- **Fixed:** `RwLock` cloned and dropped task wakers while holding its internal `std` mutex: when a waiter was queued, re-polled with a new waker, cancelled, or acquired, and when a release collected the waiters to wake. A waker's clone or drop may run executor code that re-enters the lock, which deadlocked, and a waker that panicked there poisoned the lock so every later `read`, `write` or guard drop panicked. `RwLock` now follows `Mutex` (#1066): queued wakers sit behind an `Arc` that a release copies by refcount alone, a waiter clones its waker before locking, displaced and removed wakers are dropped after unlocking, and a poisoned state lock is used as is. A waker destructor that panics as its waiter acquires or is cancelled now unwinds through the new guard (releasing the lock) or after the waiters behind it are woken, instead of leaving the lock held or a waiter unwoken. Five new tests in `tests/lock_cancel.rs` fail on the base and pass now (#403).
- **Known limitations:** `Mutex` still has the two panicking-destructor paths fixed here for `RwLock`; that is a separate follow-up.
