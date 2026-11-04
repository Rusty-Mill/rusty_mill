---
category: Fixed
changelog: rusty_tokio Runtime teardown cancels tasks before waiting on blocking jobs, as tokio does.
---
## 2026-11-04 - rusty_tokio runtime teardown cancels tasks first

- **Fixed:** dropping a `Runtime` waited for the blocking pool to drain before cancelling its
  async tasks, so a `spawn_blocking` job waiting on something a task owned (such as a channel
  whose sender lives in a pending task) never ended, and the drop hung forever. `Drop` and
  `shutdown_timeout` now join the workers, cancel tasks, then drain the blocking pool, as tokio
  does (card #720). A panic from a task's destructor is re-raised only after the pool drains.
