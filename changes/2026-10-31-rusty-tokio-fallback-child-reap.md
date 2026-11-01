---
category: Fixed
changelog: rusty_tokio reaps a dropped kill_on_drop Child through the install_fallback runtime, so one dropped on another executor's thread no longer stays a zombie.
---
## 2026-10-31 - rusty_tokio: a dropped kill_on_drop Child is reaped under the fallback

- **Fixed:** with `install_fallback()` installed, a `kill_on_drop(true)` `Child` dropped on a thread with no rusty_tokio runtime (a tokio worker, say) was killed but never reaped, so it stayed a zombie until the process exited. `Child`'s `Drop` looked only for the thread's own runtime; it now also uses the fallback, as spawning, I/O and timers already do, and runs the reap on its blocking pool (#1302). This also holds when the `Child` is dropped by a thread-local destructor after rusty_tokio's own thread-local is gone: the lookup no longer panics there (which aborted the process) and uses the fallback.
- **Known limitations:** with neither a runtime on the dropping thread nor a fallback, the kill still goes out but the reap is skipped, as before. The new regression test is Unix-only; on Windows the reap path is the same code but untested.
