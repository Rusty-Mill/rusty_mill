---
category: Changed
changelog: "`agentgateway`'s integration tests serve their mock upstreams on `rusty_axum`, their scripted gRPC processors on `rusty_hyper` and their HTTP MCP target on `rusty_mcp_server`'s `Mount`, dropping the crate's `axum` dev-dependency (#875)."
---
## 2026-10-27 - agentgateway's integration tests leave axum

- **Changed:** `tests/common` gains `upstream(addr, router)`, a `rusty_axum` mock upstream on the gateway's `rusty_tokio` runtime that keeps serving until `Upstream::stop` (as a spawned `axum::serve` did), `header_pairs`, and `grpc_upstream`, a unary h2c gRPC mock on `rusty_hyper` whose framing and `grpc-status` trailer stay hand-written so it shares no gRPC code with the gateway's `rusty_tonic` client. Every suite's HTTP mock (`a2a`, `agui`, `ext_authz`, `host_proxy`, `llm`, `retry_and_ratelimit`, `tls`) moves onto `upstream`; the `mcp_guardrails` and `mcp_span_attributes` processors onto `grpc_upstream`. The `mcp_guardrails` HTTP MCP target, an `rmcp` streamable server nested in `axum`, becomes a `rusty_mcp_server` `Mount` with the same `echo` tool, still recording every request's path and headers and answering only at its mount.
- **Dependencies:** `axum` leaves `agentgateway`'s `[dev-dependencies]`, and so does the redundant `http-body-util` entry (it remains a normal dependency). `rusty_hyper` and `rusty_mcp_server`'s `rusty-axum` feature join them; no third-party dependency is added.
- **Known limitations:** the tests still drive the gateway with `reqwest` and the `rmcp` client, so `tokio` stays a dev-dependency. `rusty_mcp_middleware`'s JWKS mock still uses `axum`.
