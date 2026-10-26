---
category: Fixed
changelog: `rusty_tokio`'s task and blocking spans (the `tracing` feature) now exit before the task's `JoinHandle` resolves.
---
## 2026-10-25 - rusty_tokio: exit task spans before completing the join

- **Fixed:** with the `tracing` feature on, a task's result was published while its span was still entered. `join_inner.complete` ran inside the instrumented future (`spawn`, `Builder::spawn`, `spawn_local`), and the blocking pool sent a `spawn_blocking` result inside the closure's span guard. So `handle.await` could return before the subscriber saw the span's last exit, and a console-style subscriber briefly saw a finished task as still running. Now only the user's future or closure runs inside the span, and the join completes after the span exits.
- **Fixed (review):** the user's future is wrapped in its span before the outer task body is built. So a task aborted before its first poll still drops the future inside the task span, as before this change. `Instrumented` enters its span on drop. New current-thread and `LocalSet` scenarios abort a never-polled future and check, through a recorder-kept stack of entered spans, which span it was dropped in. Reverting either wrapper fails its scenario.
- **Fixed:** this made `tests/tracing_instrumentation.rs::console_subscriber_shaped_instrumentation` flaky. The assertions `enters == exits` for the plain task span (line 152) and the blocking span (line 203) raced, and PR #1065's workspace shard went red. The test is unchanged.
- **Verified:** on `main`, the test failed 88 of 100 runs pinned to one core (`taskset -c 0`), at line 203. With both fixes it fails 0 of 300 pinned runs and 0 of 300 unpinned. Reverting only the blocking fix fails 92 of 100 pinned. Reverting only the task fix fails 15 of 300 unpinned, at line 152. `cargo test -p rusty_tokio -p rusty_tokio-macros --all-features`, clippy `-D warnings` and fmt pass.
