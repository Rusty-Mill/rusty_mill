---
category: Fixed
changelog: rusty_tokio's Notify and oneshot no longer clone or drop a waker, or drop an unreceived value, while holding their internal lock.
---
## 2026-11-01 - rusty_tokio: Notify and oneshot drop wakers outside their locks

- **Fixed:** a re-polled `Notified` dropped the waker it replaced while holding its waiter lock, and `Notify` woke a waiter with that lock still held (an edition 2021 `if let` keeps the guard through its block). `oneshot::Receiver` cloned its waker and dropped the one it replaced under the channel lock on every pending poll, and its `Drop` dropped the stored waker or an unreceived value there too. A waker's clone or drop may run code that re-enters the primitive (a waker that owns the `Sender`, say), which deadlocked. Both now clone before locking and drop what they displace after unlocking; `oneshot::Receiver` keeps a stored waker that already wakes the same task instead of replacing it. Four new tests in `tests/reentrant_waker.rs` fail on the base and pass now (#578).
- **Known limitations:** `Notify` waking under its waiter lock could not be forced into a deadlock by a test (a woken waiter returns before taking that lock); the change is source-traced.
