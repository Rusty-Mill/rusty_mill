---
category: Changed
changelog: "The agent gateway crates reach `tracing` through the `rusty_log` facade instead of depending on `tracing` and `tracing-subscriber` themselves. No behaviour change."
---
## 2026-10-19 - the agent gateway moves onto rusty_log (tracing facade, step 2, second area)

- **Changed:** `agentgateway`, `agentgateway-a2a`, `agentgateway-agui`, `agentgateway-auth`, `agentgateway-llm`, `agentgateway-mcp`, `agentgateway-proxy` and `agentgateway-tls` depend on `rusty_log` instead of `tracing` (and, for `agentgateway`, `tracing-subscriber`, which it now gets through `rusty_log`'s `subscriber` feature, normal and dev alike). All `tracing::` and `tracing_subscriber::` paths in their sources, tests and examples are `rusty_log::` and `rusty_log::tracing_subscriber::`, 24 files. The facade re-exports the same crates, so no log line, span or filter changes. `agentgateway-config` and `agentgateway-core` never depended on `tracing`.
- **Policy:** the eight crates leave `TRACING_BASELINE` in the same PR as their manifests (22 crates left).
- **Verified:** clippy `-D warnings` and the full tests of all ten gateway crates (all features) pass, including `mcp_span_attributes`, which exercises the span layer end to end.
- **Not changed:** the rest of the workspace; nothing in the workspace outside Nexus uses the `#[instrument]` attribute (a comment in `agentgateway-mcp/src/span.rs` only mentions it), so that limitation of `rusty_log` has not bitten yet.
- **Depends on:** the `rusty-mcp` migration PR and, through it, the `rusty_log` crate PR.
