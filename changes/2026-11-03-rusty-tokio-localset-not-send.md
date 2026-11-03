---
category: Changed
changelog: "Breaking: rusty_tokio LocalSet is now !Send and !Sync, like tokio's."
---
## 2026-11-03 - rusty_tokio LocalSet is !Send

- **Changed (breaking):** `rusty_tokio::task::LocalSet` is now `!Send + !Sync`, matching
  tokio (card #718). It holds `!Send` tasks, so moving it to another thread was never sound
  to rely on; before, only a runtime assert caught a second thread using it. The compiler
  now rejects that, and the runtime thread check (and its two tests) is gone. A compile_fail
  doctest pins the new bound. No workspace crate moved a `LocalSet` across threads.
