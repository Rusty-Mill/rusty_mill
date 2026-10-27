---
category: Fixed
changelog: `rusty_tokio`'s `io::DuplexStream` no longer deadlocks when a waker it wakes or replaces owns the other half of the pair.
---
## 2026-10-27 - rusty_tokio: DuplexStream wakes outside its pipe lock

- **Fixed:** `io::DuplexStream` woke and replaced wakers while it held a pipe lock. A waker may own the other half of the pair, as a task holding both halves does. Waking or dropping that waker ran the half's `Drop`, which takes the same lock, and the thread deadlocked. A read, write, shutdown or drop now takes the waker out under the lock and wakes or drops it after releasing it. Same defect class as the `mpsc` fix of 2026-10-23.
- **Tests:** `tests/duplex_reentrant.rs` covers the drop, write, read, shutdown and re-register paths with a waker that owns a half. All five deadlocked before the fix.
