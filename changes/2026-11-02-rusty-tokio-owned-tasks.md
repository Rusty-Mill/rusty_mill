---
category: Fixed
changelog: rusty_tokio keeps every spawned task alive until it finishes or the runtime shuts down; shutdown cancels the rest, so their JoinHandles resolve as cancelled instead of hanging.
---
## 2026-11-02 - rusty_tokio: the runtime owns its tasks

- **Fixed:** a spawned task was owned only by its wakers and run-queue entries. A task pending with no waker stored anywhere was dropped silently (with everything its future held), its `JoinHandle` never resolved, and `num_alive_tasks` stayed raised for good. Tasks still queued when the runtime shut down were dropped the same way. The runtime now keeps every task in an owned-task registry, as tokio does, until the task finishes. Shutdown (`Drop`, `shutdown_timeout`, `shutdown_background`) cancels whatever is left: each future is dropped inside the runtime's context, and each `JoinHandle` resolves to a cancelled `JoinError`. A task mid-poll during `shutdown_background` is cancelled when its poll returns. Cancellation is panic-safe: a future whose destructor panics resolves its `JoinHandle` with that panic, and a panicking join waker doesn't stop the other tasks being cancelled (its panic is re-raised from shutdown afterwards). A spawn on a `Handle` after its runtime shut down returns a handle that is already cancelled, instead of one that hangs. `LocalSet`/`spawn_local` tasks are not covered yet (#405).
