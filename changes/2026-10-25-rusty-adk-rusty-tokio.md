---
category: Changed
changelog: "rusty_adk's agent, graph, runner, session and AG-UI crates now run on `rusty_tokio` and `rusty_stream`. They no longer depend on `tokio`, `futures` or `async-stream` (#908)."
---
## 2026-10-25 - rusty_adk on rusty_tokio and rusty_stream

- **Changed:** `adk-tools`, `adk-graph`, `adk-agents`, `adk-runner`, `adk-agui`, `adk-macros`, `adk-sessions`, `rusty-adk` and `agui-agent-server` use `rusty_stream` in place of `futures` and `async-stream`, and `rusty_tokio` in place of `tokio` and `tokio-test`. `Agent::run` and `Graph::run` now return `rusty_stream::BoxStream`. `AdkAgent::new` takes a `rusty_tokio` `Handle`. Doctests drive their futures with `rusty_tokio::Runtime` (G10).
- **Changed:** the `adk-a2a` library moves to `rusty_stream` and `rusty_tokio::select!`. Its tests keep `tokio`, because they serve `rusty_a2a` over axum.
- **Known limitations:** streams from `adk-models` and from the `rusty_a2a` client are still `futures` streams. They are wrapped in `rusty_stream::Compat`. `weather-agent` still runs a tokio `main` because the live models use reqwest. `mcp-tool-server` still runs one because `adk-mcp`'s stdio transport is on tokio. `adk-mcp`, `adk-models` and `a2a-agent-server` are unchanged (tier C).
