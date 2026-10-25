---
category: Changed
changelog: skillopt-core, agentgateway-agui, rusty_mill_board and rk-mcp's tests run on rusty_tokio instead of tokio (#911, tier A).
---
## 2026-10-25 - Small crates on rusty_tokio (#911, tier A)

- **Changed:** `skillopt-core`, `agentgateway-agui` and `rk-mcp` (tests), and `rusty_mill_board` (its private HTTP and signal runtimes, and `rusty_request` without the `tokio` feature) use `rusty_tokio`.
- **Changed:** `rusty_tonic`'s tests build the tonic oracle's streams with `rusty_stream` (`Compat`) instead of `futures-util`. `tokio` stays a dev-dependency only to host the oracle server.
- **Known limitations:** `rk-compose`, `skillopt-cli`, `agentgateway-tls` and `rusty-hister-model` stay on tokio: each runs inside a tokio runtime held by a tier C crate (`rk-app`, `skillopt-model`'s `reqwest`, `agentgateway`, `sqlx`), and `rusty_tokio`'s timers, I/O and processes need their own runtime. `rk-mcp` keeps `tokio::sync::Mutex` for its connection lock until #1064 makes `rusty_tokio`'s mutex cancellation-safe. The inventory records why.
