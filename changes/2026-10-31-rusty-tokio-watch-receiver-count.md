---
category: Fixed
changelog: rusty_tokio's watch::Receiver::wait_for no longer deadlocks when its predicate calls receiver_count() or clones or drops a Receiver.
---
## 2026-10-31 - rusty_tokio: watch receiver count outside the channel lock

- **Fixed:** `watch::Receiver::wait_for` runs its predicate while holding the channel's lock, and `Sender::receiver_count`, `Receiver::clone` and `Receiver`'s drop all took that same lock, so a predicate that called any of them hung forever (#1304). The receiver count is now an atomic beside the lock, as in tokio 1.53.3, so those calls take no lock and report the same counts tokio does. `send` reads the count before locking, as tokio's does. The new test hangs on the base and is bounded by a 5 s wait, so it fails there instead of hanging the suite.
- **Known limitations:** the predicate still runs under the lock, so anything else on the same channel that takes the lock deadlocks from it: `borrow`, `borrow_and_update`, `send`, `send_modify`, another receiver's `changed()`, and dropping the `Sender`. tokio's predicate holds a read lock, its `borrow` there usually succeeds and its `Sender` drop takes no lock. `wait_for`'s docs list these.
