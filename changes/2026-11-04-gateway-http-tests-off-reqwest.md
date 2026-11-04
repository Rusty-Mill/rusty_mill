---
category: Changed
changelog: agentgateway HTTP test suites run on rusty_tokio with rusty_request; rusty_request sends one Host when the caller sets it.
---
## 2026-11-04 - Gateway HTTP test suites off reqwest and tokio

- **Changed:** the nine agentgateway integration suites that talk plain HTTP (`a2a`, `agui`,
  `ext_authz`, `host_proxy`, `jwt_auth`, `limits`, `llm`, `mcp_host_origin`,
  `retry_and_ratelimit`) run on `#[rusty_tokio::test]` and drive the gateway with
  `rusty_request`, through a small test-only adapter in `tests/common/http.rs`. Every suite's
  shutdown token is now `rusty_tokio`'s, and `free_port` probes with a std listener, so it needs
  no runtime (card #410, part of #686).
- **Fixed:** `rusty_request` sent two `Host` fields when a caller set its own, which a server
  must refuse (RFC 9112 §3.2). A caller's `Host` now replaces the one from the URL, and a
  cross-origin redirect drops it.
- **Known limitations:** the MCP suites still use `rmcp`'s client and `tls.rs` still uses
  `reqwest`, so `tokio`, `tokio-util`, `reqwest` and `rmcp` stay dev-dependencies until the
  follow-up PRs.
