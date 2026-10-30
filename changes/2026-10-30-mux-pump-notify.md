---
category: Changed
changelog: rusty_mux's block pump waits for an echo-grace wake with a fresh `Notified` each turn, now that `rusty_tokio` hands on a dropped waiter's notification.
---
## 2026-10-30 - Simpler pump wake

- **Changed:** the server's PTY pump no longer keeps one pinned `Notified` across loop turns. It was a workaround (#1247) for `rusty_tokio`'s `Notify` losing a notification to a waiter dropped unfired, which #1250 fixed. No behaviour change.
