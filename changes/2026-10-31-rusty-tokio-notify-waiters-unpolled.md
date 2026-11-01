---
category: Fixed
changelog: rusty_tokio `Notify::notify_waiters` now wakes a `Notified` that was created but not yet polled or enabled, as tokio guarantees.
---
## 2026-10-31 - rusty_tokio: notify_waiters reaches a created, unpolled Notified

- **Fixed:** `Notify::notify_waiters` only woke a `Notified` that had been polled or `enable()`d, so `let n = notify.notified(); notify.notify_waiters(); n.await` hung where tokio returns. `Notify` now counts `notify_waiters` calls, each `Notified` keeps the count it was created with, and one that finds the count changed when it first joins the queue resolves, leaving any banked permit for another waiter, as tokio 1.53's `poll_notified` does. Four new tests in `sync::notify` fail on the base and pass now (#576).
- **Known limitations:** none known. The call count wraps at `usize::MAX` calls, as tokio's does at a lower bound.
