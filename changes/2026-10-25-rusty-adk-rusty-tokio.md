---
category: Changed
changelog: "rusty_adk's agent, graph, runner, session and tool crates now use `rusty_stream` and `rusty_tokio`. They no longer depend on `tokio`, `futures` or `async-stream` (#908)."
---
## 2026-10-25 - rusty_adk on rusty_tokio and rusty_stream

- **Changed:** `adk-tools`, `adk-graph`, `adk-agents`, `adk-runner`, `adk-macros`, `adk-sessions` and `rusty-adk` use `rusty_stream` in place of `futures` and `async-stream`, and `rusty_tokio` in place of `tokio` and `tokio-test`. `Agent::run`, `Graph::run` and `Runner::run` now return `rusty_stream::BoxStream`. Doctests drive their futures with `rusty_tokio::Runtime` (G10). `adk-agui` moves from `futures` to `rusty_stream`.
- **Changed:** the `adk-a2a` library moves to `rusty_stream` and `rusty_tokio::select!`. Its tests keep `tokio` because they serve `rusty_a2a` over axum.
- **Changed:** the SQLite session services run each query on a thread of their own, not on a runtime's blocking pool. They now work under any executor.
- **Known limitations:**
  - Streams from `adk-models` and from the `rusty_a2a` client are still `futures` streams, wrapped in `rusty_stream::Compat`.
  - `adk-agui`, `agui-agent-server` and `weather-agent` stay on tokio because the live models use reqwest.
  - `mcp-tool-server` stays on tokio because of `adk-mcp`'s stdio transport.
  - `adk-mcp`, `adk-models` and `a2a-agent-server` are unchanged (tier C).
