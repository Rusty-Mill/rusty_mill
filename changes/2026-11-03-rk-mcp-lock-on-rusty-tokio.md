---
category: Changed
changelog: rk-mcp's connection lock moves to rusty_tokio's Mutex; rk-mcp drops its tokio dependency.
---
## 2026-11-03 - rk-mcp connection lock on rusty_tokio

- **Changed:** `RemoteMcpClient` guards its connection with `rusty_tokio::sync::Mutex`
  instead of `tokio::sync::Mutex` (card #402, #686). It was held back by the lost-wakeup on a
  cancelled waiter (#1064), fixed in #1066. `rk-mcp` no longer depends on `tokio`; `rusty_tokio`
  moves from a dev-dependency to a dependency. The mutex is runtime-free, so `rk-app` on a
  tokio runtime is unaffected.
