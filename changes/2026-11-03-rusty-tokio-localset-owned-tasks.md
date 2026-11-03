---
category: Fixed
changelog: rusty_tokio LocalSet owns its spawn_local tasks and cancels what is left when it drops.
---
## 2026-11-03 - rusty_tokio LocalSet owns its tasks

- **Fixed:** a `spawn_local` task pending with no stored waker was dropped early, and tasks
  left when a `LocalSet` dropped never resolved their `JoinHandle`. `LocalSet` now keeps its
  tasks in the same owned-task registry `Runtime` got in #405 (card #591). Dropping the set
  (or a `LocalRuntime`) cancels every task still alive: its future is dropped and its
  `JoinHandle` resolves to a cancelled `JoinError`, or to a panic if the destructor panicked.
  A `spawn_local` while the set drops is cancelled straight away.
- **Known limitations:** no graceful draining; nothing runs again once the set drops, as before.
