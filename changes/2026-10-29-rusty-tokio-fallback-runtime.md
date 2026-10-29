---
category: Added
changelog: rusty_tokio's `runtime::install_fallback()` lets its I/O, timers and spawn run on threads driven by another executor, such as tokio.
---
## 2026-10-29 - rusty_tokio opt-in fallback runtime

- **Added:** `rusty_tokio::runtime::install_fallback()` starts one
  process-wide runtime, with one worker plus the reactor, timer and blocking
  threads, and returns its `Handle`. After it runs, I/O registration, timers,
  `spawn` and `spawn_blocking` on a thread with no rusty_tokio runtime use
  that runtime instead of panicking. A future polled by tokio is woken by
  rusty_tokio's reactor through its ordinary `Waker`. Calling it again
  returns the same handle.
- **Changed:** without a runtime or a fallback, the panic message now also
  names `install_fallback()`. `Handle::current` and `Handle::try_current`
  are unchanged, and `block_in_place`, `time::pause`, `resume` and
  `advance` still need the thread's own runtime.
