---
category: Changed
changelog: rk-mcp, skillopt-core, agentgateway-agui and rusty_mill_board run on rusty_tokio instead of tokio (#911, tier A).
---
## 2026-10-25 - Small crates on rusty_tokio (#911, tier A)

- **Changed:** `rk-mcp` (its connection lock and tests), `skillopt-core` and `agentgateway-agui` (tests), and `rusty_mill_board` (its private HTTP and signal runtimes, and `rusty_request` without the `tokio` feature) use `rusty_tokio`. None of them depends on `tokio` any more.
- **Known limitations:** `rk-compose`, `skillopt-cli`, `agentgateway-tls` and `rusty-hister-model` stay on tokio: each runs inside a tokio runtime held by a tier C crate (`rk-app`, `skillopt-model`'s `reqwest`, `agentgateway`, `sqlx`), and `rusty_tokio`'s timers, I/O and processes need their own runtime. The inventory records why.
