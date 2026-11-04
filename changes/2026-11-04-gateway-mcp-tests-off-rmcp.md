---
category: Changed
changelog: agentgateway MCP test suites and their stdio mock server move off rmcp onto rusty_mcp_client_native and rusty_mcp_server.
---
## 2026-11-04 - Gateway MCP test suites off rmcp

- **Changed:** the agentgateway MCP suites (`federation`, `mcp_guardrails`,
  `mcp_prompts_resources`, `mcp_rules`, `mcp_span_attributes`) run on `#[rusty_tokio::test]` and
  connect through `rusty_mcp_client_native`'s Streamable HTTP client, behind
  `tests/common/mcp.rs`. The stdio fixture `examples/mock_mcp_server.rs` is a `rusty_mcp_server`
  server with the same tools, prompts, resources and environment knobs. `rmcp` and `tokio-util`
  are no longer dev-dependencies of `agentgateway` (card #410, part of #686).
- **Known limitations:** the gateway's MCP paths are now tested against first-party clients and
  servers only; `rusty_mcp_client_native` and `rusty_mcp_server` keep their own interop tests
  against `rmcp`. `tls.rs` still uses `reqwest`, so `tokio` and `reqwest` stay until the next PR.
