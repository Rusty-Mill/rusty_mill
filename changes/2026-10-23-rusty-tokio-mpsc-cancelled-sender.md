---
category: Fixed
changelog: rusty_tokio bounded mpsc no longer leaves a sender parked with free capacity after another sender's send/reserve was cancelled.
---
## 2026-10-23 - rusty_tokio: cancelled mpsc senders keep the wake chain

- **Fixed:** a cancelled bounded `send`/`reserve`/`reserve_many`/`reserve_owned` future left its waker in the channel's waiter list. A received value wakes exactly one waiter, so that wake could go to the cancelled operation while a live sender stayed parked with room in the channel. Each wait now holds one keyed entry. Re-polling refreshes the entry instead of queueing a duplicate, dropping the future removes it, and a future dropped after it was woken passes the wake to the next waiter. Found in review of #954 (pre-existing, not introduced there).
- **Fixed:** wakers removed or replaced in the channel (a cancelled, refreshed or completed send/reserve wait, and a re-registered receive waker) are dropped after the channel lock is released. A waker may own a `Sender` of the same channel, and that `Sender`'s `Drop` takes the lock, so destroying it under the lock deadlocked safe code.
- **Tests:** `tests/mpsc_cancel.rs` covers a cancelled waiting send, a cancelled waiting reserve, a woken-then-cancelled send, and a re-polled send. Four channel-reentrant destructor tests cover the cancel, refresh, complete and receive re-register paths. Every test fails on the code it fixes.
