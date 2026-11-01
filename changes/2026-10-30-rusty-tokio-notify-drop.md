---
category: Fixed
changelog: rusty_tokio's Notify no longer loses a notify_one to a Notified that was dropped before it resolved, such as the losing branch of a select! in a loop.
---
## 2026-10-30 - rusty_tokio: a dropped Notified leaves the queue

- **Fixed:** a `Notified` (or `OwnedNotified`) dropped while registered stayed in `Notify`'s waiter queue, so a later `notify_one` or `notify_last` could fire the dead waiter and wake nobody. A loop that races `notified()` against other futures, such as rusty_mux's PTY reader in #1247, lost wakeups that way. Dropping an unresolved waiter now removes it, and if a single wakeup (`notify_one`, `notify_last` or a permit taken by `enable`) had already reached it, passes that on to the next waiter or banks it again, as tokio does. The wakeup is claimed for the waiter under the queue lock, so a waiter dropped after it leaves the queue but before it is woken still passes the wakeup on. A `notify_waiters` wakeup is not passed on.
- **Known limitations:** removing a waiter is a linear scan of the queue, which is fine for the few waiters a `Notify` has in this workspace.
