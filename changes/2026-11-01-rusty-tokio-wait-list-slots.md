---
category: Fixed
changelog: rusty_tokio's Barrier, OnceCell and SetOnce keep one waker per waiting future, so a re-polled or cancelled wait no longer piles up wakers.
---
## 2026-11-01 - rusty_tokio: Barrier, OnceCell and SetOnce keep one waker per waiter

- **Fixed:** `Barrier::wait`, `OnceCell::get_or_init` (while another caller initialises) and `SetOnce::wait` pushed a new waker clone on every pending poll and never removed one when the future was dropped, so a waiter re-polled by `select!` or a timeout loop stored a waker per poll until the round or initialisation completed. The three now share a crate-private `WaitList`: each waiting future owns one keyed slot, a re-poll replaces its waker in place, and resolving or dropping the future removes it. Wakers are cloned before locking and replaced or removed ones dropped after unlocking. `Barrier`'s cancelled arrival still counts toward the round, as documented. Thirteen new tests in `tests/wait_cancel.rs`: ten fail on the base and pass now, and three guard the drop-after-unlock order (they fail if a stale waker is dropped under the lock) (#579).
- **Known limitations:** removing a slot is a linear scan of that primitive's waiters.
