---
category: Changed
changelog: rusty_mcp_middleware and rusty-mcp-client no longer pull in tokio; tokio callers of rusty-mcp-client turn on its new `tokio` feature.
---
## 2026-10-29 - rusty_mcp library code off tokio (#906)

- **Changed:** `rusty_mcp_middleware`'s `jwt` and `otel` features no longer
  enable `rusty_request`'s `tokio` backend. The JWKS fetch runs on the
  caller's `rusty_tokio` runtime; every workspace consumer (`agentgateway`,
  `agentgateway-auth`, `agentgateway-mcp`) already runs one.
- **Added:** `rusty-mcp-client` feature `tokio`, off by default. It lets a
  caller on a tokio runtime fetch OAuth client-credentials tokens.
  `adk-mcp` (`client`), `rk-mcp` (`rmcp`) and `rp-mcp` turn it on.
- **Changed:** the `rusty_mcp_middleware` and `rusty-mcp-client` test suites
  run on `rusty_tokio` and `rusty_request`; `tokio` and `reqwest` are gone
  from their dev-dependencies.
- **Fixed:** the OAuth token-endpoint mock in `rusty-mcp-client`'s tests
  reads the whole request, so a form body sent after the head no longer
  fails the test.
