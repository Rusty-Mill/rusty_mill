---
category: Changed
changelog: Test-only: the `rb_rlbot_core` hold test drives a ready-but-idle backend and bounds its first wait.
---
## 2026-10-12 - rb_rlbot_core: hold test hardening

- **Changed:** the test for `BroadcastHeld` / `BroadcastResumed` (#631) now drives a backend that is ready but has nothing to show (the hold stays latched, `Resumed` waits for the first real broadcast) and fails instead of waiting for ever for its first packet. Mutation-checked: emitting `Resumed` on readiness alone fails it. No production change. Optional hardening from the review of #631.
