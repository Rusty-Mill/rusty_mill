---
category: Fixed
changelog: rusty_tokio timers no longer deadlock when a task's waker calls back into the timer driver from its clone or drop.
---
## 2026-11-02 - rusty_tokio: timer wakers cloned and dropped outside the lock

- **Fixed:** the timer driver cloned wakers in `register`/`update_waker` and dropped the displaced or cancelled waker in `update_waker`/`cancel` while holding its `inner` lock. Waker clone and drop run user code, so a waker that touched a timer (for example, one whose drop cancels another sleep) deadlocked. Wakers are now cloned before locking and dropped after unlocking (#588).
